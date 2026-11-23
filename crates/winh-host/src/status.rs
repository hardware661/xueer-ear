//! Small state snapshot for the GUI; contains neither dictation text nor credentials.
use crate::install::Result;
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Serialize)]
struct Snapshot {
    backend: &'static str,
    phase: &'static str,
    microphone: String,
    /// 最近一句走的识别通道（"doubao" / "wetype"），诊断信息用
    channel: &'static str,
    peak: u32,
    partials: usize,
    finals: usize,
    last_error: String,
    pid: u32,
    updated_ms: u64,
    /// 正在一轮听写里（对外约定：雪儿 speaker 看到它就先不出声，这轮结束再念）。
    /// 后台活着时至少每 5 秒重写一次本文件，读的一方应把 15 秒没更新的当成没在听写
    dictating: bool,
    /// 进行中的轮数（新一轮可能在上一轮收尾时就开始）
    #[serde(skip)]
    rounds: u32,
}
struct Store {
    path: PathBuf,
    snapshot: Snapshot,
}
#[derive(Clone)]
pub struct Status(Arc<Mutex<Store>>);
/// 一轮听写的凭据：拿着时 `dictating` 为真，丢掉（正常结束、出错、panic）就算这一轮结束
pub struct Round(Status);

impl Drop for Round {
    fn drop(&mut self) {
        self.0.change(|s| {
            s.rounds = s.rounds.saturating_sub(1);
            s.dictating = s.rounds > 0;
        });
    }
}

impl Status {
    pub fn new(backend: &'static str) -> Result<Self> {
        let folder = PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA不可用")?)
            .join("XueerEar");
        std::fs::create_dir_all(&folder)?;
        Ok(Self::at(folder.join("status.json"), backend))
    }

    fn at(path: PathBuf, backend: &'static str) -> Self {
        let status = Self(Arc::new(Mutex::new(Store {
            path,
            snapshot: Snapshot {
                backend,
                phase: "connecting",
                microphone: String::new(),
                channel: "",
                peak: 0,
                partials: 0,
                finals: 0,
                last_error: String::new(),
                pid: std::process::id(),
                updated_ms: 0,
                dictating: false,
                rounds: 0,
            },
        })));
        status.touch();
        status
    }

    /// 开始一轮听写；返回的凭据丢掉时这一轮结束
    pub fn round(&self) -> Round {
        self.change(|s| {
            s.rounds += 1;
            s.dictating = true;
        });
        Round(self.clone())
    }
    fn change(&self, change: impl FnOnce(&mut Snapshot)) {
        let Ok(mut store) = self.0.lock() else {
            return;
        };
        change(&mut store.snapshot);
        store.snapshot.updated_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64;
        if let Ok(bytes) = serde_json::to_vec(&store.snapshot) {
            let temp = store
                .path
                .with_extension(format!("{}.tmp", std::process::id()));
            if std::fs::write(&temp, bytes).is_ok() {
                let _ = std::fs::rename(&temp, &store.path);
            }
        }
    }
    pub fn touch(&self) {
        self.change(|_| {});
    }
    pub fn phase(&self, phase: &'static str) {
        self.change(|s| {
            s.phase = phase;
            if phase == "armed" {
                s.last_error.clear();
                s.partials = 0;
                s.finals = 0;
            }
            if matches!(phase, "paused" | "ready") {
                s.peak = 0;
            }
        });
    }
    pub fn microphone(&self, name: String) {
        self.change(|s| s.microphone = name);
    }
    pub fn channel(&self, name: &'static str) {
        self.change(|s| s.channel = name);
    }
    pub fn progress(&self, peak: u32, partials: usize, finals: usize) {
        self.change(|s| {
            s.peak = peak;
            s.partials = partials;
            s.finals = finals;
        });
    }
    pub fn error(&self, error: String) {
        self.change(|s| {
            s.phase = "error";
            s.last_error = error;
            s.peak = 0;
        });
    }
    pub fn restored(&self) {
        self.change(|s| {
            s.backend = "windows";
            s.phase = "stopped";
            s.pid = 0;
            s.peak = 0;
        });
    }
}

pub fn active_backend_label() -> &'static str {
    let Some(root) = std::env::var_os("LOCALAPPDATA") else {
        return "识别";
    };
    let path = PathBuf::from(root).join("XueerEar/status.json");
    let Ok(file) = std::fs::File::open(path) else {
        return "识别";
    };
    use std::io::Read;
    let mut bytes = Vec::new();
    if file.take(16_385).read_to_end(&mut bytes).is_err() || bytes.len() > 16_384 {
        return "识别";
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return "识别";
    };
    match value["backend"].as_str() {
        Some("wetype") => "输入法识别",
        Some("wechat") => "微信识别",
        _ => "识别",
    }
}

pub fn startup_error(message: &str) {
    // Only update the snapshot owned by this failed child, never another running broker.
    let Some(root) = std::env::var_os("LOCALAPPDATA") else {
        return;
    };
    let path = PathBuf::from(root).join("XueerEar/status.json");
    let Ok(bytes) = std::fs::read(&path) else {
        return;
    };
    if bytes.len() > 16_384 {
        return;
    }
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return;
    };
    if value["pid"].as_u64() != Some(u64::from(std::process::id())) {
        return;
    }
    value["phase"] = "error".into();
    value["last_error"] = message.into();
    value["updated_ms"] = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64)
        .into();
    if let Ok(bytes) = serde_json::to_vec(&value) {
        let _ = std::fs::write(path, bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dictating(path: &std::path::Path) -> bool {
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        value["dictating"].as_bool().unwrap()
    }

    #[test]
    fn a_round_is_dictating_until_it_ends_even_when_rounds_overlap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("status.json");
        let status = Status::at(path.clone(), "wetype");
        assert!(!dictating(&path));
        let first = status.round();
        assert!(dictating(&path));
        // 新一轮在上一轮收尾时就开始了：上一轮结束不能把它清掉
        let second = status.round();
        drop(first);
        assert!(dictating(&path));
        drop(second);
        assert!(!dictating(&path));
    }

    #[test]
    fn a_round_that_panics_still_ends() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("status.json");
        let status = Status::at(path.clone(), "wetype");
        let worker = status.clone();
        let _ = std::thread::spawn(move || {
            let _round = worker.round();
            panic!("听写线程出错");
        })
        .join();
        assert!(!dictating(&path));
    }
}
