//! 快捷键改了不用重启服务（纯逻辑，可在 WSL 测）：盯着设置文件的修改时间，变了才重读。
use std::time::SystemTime;

pub struct KeyWatch {
    vk: u32,
    stamp: Option<SystemTime>,
}

impl KeyWatch {
    pub fn new(vk: u32, stamp: Option<SystemTime>) -> Self {
        Self { vk, stamp }
    }

    /// `stamp` 是设置文件此刻的修改时间。和上次不一样才调 `load` 读快捷键；
    /// 读出来和现在用的不一样就返回新的键。读失败（文件正写到一半）下次再试。
    pub fn check(
        &mut self,
        stamp: Option<SystemTime>,
        load: impl FnOnce() -> Option<u32>,
    ) -> Option<u32> {
        if stamp == self.stamp {
            return None;
        }
        let vk = load()?;
        self.stamp = stamp;
        if vk == self.vk {
            return None;
        }
        self.vk = vk;
        Some(vk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(seconds: u64) -> Option<SystemTime> {
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
    }

    #[test]
    fn untouched_file_is_not_read_again() {
        let mut watch = KeyWatch::new(0xA5, at(1));
        assert_eq!(watch.check(at(1), || panic!("文件没变不该读")), None);
    }

    #[test]
    fn new_key_is_reported_once() {
        let mut watch = KeyWatch::new(0xA5, at(1));
        assert_eq!(watch.check(at(2), || Some(0x77)), Some(0x77));
        assert_eq!(watch.check(at(2), || panic!("已经读过了")), None);
        // 改了别的设置、快捷键没变
        assert_eq!(watch.check(at(3), || Some(0x77)), None);
        assert_eq!(watch.check(at(4), || Some(0xA5)), Some(0xA5));
    }

    #[test]
    fn unreadable_file_is_retried_and_a_created_file_is_noticed() {
        let mut watch = KeyWatch::new(0xA5, None);
        assert_eq!(watch.check(at(5), || None), None);
        assert_eq!(watch.check(at(5), || Some(0x78)), Some(0x78));
    }
}
