//! service.log 只追加不截断，用久了会一直变大（审查）。后台服务每次启动时检查一下：
//! 超过 `MAX_BYTES` 就只留最后一半（从完整的一行开始），旧的部分扔掉。
use std::{fs, io, path::Path};

pub const MAX_BYTES: u64 = 1024 * 1024;

pub fn trim(path: &Path, max: u64) -> io::Result<()> {
    let Ok(meta) = fs::metadata(path) else {
        return Ok(());
    };
    if meta.len() <= max {
        return Ok(());
    }
    let bytes = fs::read(path)?;
    let start = bytes.len() - (max / 2) as usize;
    // 从下一行开头留起，不留半行（也不会切在一个汉字中间）
    let start = bytes[start..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(bytes.len(), |i| start + i + 1);
    fs::write(path, &bytes[start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_big_log_keeps_its_last_whole_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("service.log");
        let text: String = (0..200).map(|i| format!("第{i}行：雪耳\n")).collect();
        fs::write(&path, &text).unwrap();
        trim(&path, 400).unwrap();
        let kept = fs::read_to_string(&path).unwrap();
        assert!(kept.len() <= 200, "留下 {} 字节", kept.len());
        assert!(kept.ends_with("第199行：雪耳\n"));
        assert!(kept.starts_with('第'), "要从完整的一行开始：{kept:?}");
    }

    #[test]
    fn small_or_missing_logs_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("service.log");
        trim(&path, 400).unwrap();
        assert!(!path.exists());
        fs::write(&path, "一行\n").unwrap();
        trim(&path, 400).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "一行\n");
    }
}
