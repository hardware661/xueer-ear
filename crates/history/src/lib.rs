//! 本地听写历史：每句一行 JSON 追加到 `history.jsonl`，按天数清理，只存硬盘、不上传。
//! 默认开启、保留 7 天（主人 2026-11-15 定），设置里可改或关闭。

use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    path::PathBuf,
};

const DAY_MS: u64 = 86_400_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Unix 毫秒
    pub at_ms: u64,
    pub text: String,
    /// 上屏时的前台程序（exe 名），可为空
    pub app: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub keep_days: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            keep_days: 7,
        }
    }
}

pub struct History {
    path: PathBuf,
    settings: Settings,
}

impl History {
    pub fn new(path: PathBuf, settings: Settings) -> Self {
        Self { path, settings }
    }

    /// 追加一句；关闭时什么都不写。
    pub fn append(&self, entry: &Entry) -> io::Result<()> {
        if !self.settings.enabled {
            return Ok(());
        }
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut line = serde_json::to_string(entry)?;
        line.push('\n');
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?
            .write_all(line.as_bytes())
    }

    /// 最近 n 句，新的在前。文件不存在或关闭时返回空；坏行（如写到一半断电）跳过。
    pub fn recent(&self, n: usize) -> io::Result<Vec<Entry>> {
        if !self.settings.enabled {
            return Ok(Vec::new());
        }
        let mut all = self.read_all()?;
        all.reverse();
        all.truncate(n);
        Ok(all)
    }

    /// 删掉比 keep_days 更早的记录，返回删了几条。先写临时文件再替换，中途失败原文件不受影响。
    pub fn prune(&self, now_ms: u64) -> io::Result<usize> {
        let all = self.read_all()?;
        let cutoff = now_ms.saturating_sub(u64::from(self.settings.keep_days) * DAY_MS);
        let keep: Vec<&Entry> = all.iter().filter(|e| e.at_ms >= cutoff).collect();
        let removed = all.len() - keep.len();
        if removed == 0 {
            return Ok(0);
        }
        let tmp = self.path.with_extension("jsonl.tmp");
        {
            let mut f = File::create(&tmp)?;
            for e in keep {
                let mut line = serde_json::to_string(e)?;
                line.push('\n');
                f.write_all(line.as_bytes())?;
            }
            f.sync_all()?;
        }
        fs::rename(&tmp, &self.path)?;
        Ok(removed)
    }

    fn read_all(&self) -> io::Result<Vec<Entry>> {
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut out = Vec::new();
        for line in BufReader::new(file).split(b'\n') {
            if let Ok(entry) = serde_json::from_slice::<Entry>(&line?) {
                out.push(entry);
            }
        }
        Ok(out)
    }
}
