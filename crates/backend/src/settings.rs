//! User settings are snapshotted when the next Win+H capture starts.
use crate::Result;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_FILE_BYTES: u64 = 8 * 1024;
/// 右 Alt 的虚拟键码
pub const HOTKEY_RIGHT_ALT: u32 = 0xA5;
pub const SILENCE_SECONDS_OPTIONS: &[u32] = &[1, 2, 3, 5, 10];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub microphone: Option<String>,
    pub silence_seconds: u32,
    /// 定稿文字上屏前的整理（textflow）
    pub text: TextSettings,
    pub history_enabled: bool,
    pub history_keep_days: u32,
    /// 快捷键听写：雪耳自己的面板，不经过 Win+H
    pub hotkey_enabled: bool,
    /// 是否接管 Win+H（把识别换成雪耳）。默认不接管：Win+H 还是微软的（主人 2026-11-15 定）
    pub winh_enabled: bool,
    /// Windows 虚拟键码，默认右 Alt（0xA5）
    pub hotkey_vk: u32,
    /// 快捷键听写一次最长听多少分钟（停顿不会结束，到点才自动停）；主人 2026-11-15 要「特别特别长」
    pub max_listen_minutes: u32,
    /// 晓晓提示音（没听清、系统更新后退回），本地播放
    pub sounds_enabled: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            microphone: None,
            silence_seconds: 3,
            text: TextSettings::default(),
            history_enabled: true,
            history_keep_days: 7,
            hotkey_enabled: true,
            winh_enabled: false,
            hotkey_vk: HOTKEY_RIGHT_ALT,
            sounds_enabled: true,
            max_listen_minutes: 30,
        }
    }
}

/// 文字整理开关。热词不在这里：热词表是单独的 hotwords.txt，一行一个词。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextSettings {
    pub strip_fillers: bool,
    pub normalize_numbers: bool,
    pub drop_final_period: bool,
    pub cjk_latin_space: bool,
    pub fullwidth_punct_in_cjk: bool,
    /// 整句说「发送」时按回车；默认关，免得在终端里把半截话发出去
    pub allow_send: bool,
    /// 终端里说「换行」时发 Ctrl+J。Claude Code 里是换行；普通 bash/zsh 里 Ctrl+J 就是回车、会执行命令，
    /// 常在普通命令行里听写的可以关掉（关掉后终端里「换行」什么都不做）。
    pub terminal_newline: bool,
}

impl Default for TextSettings {
    fn default() -> Self {
        Self {
            strip_fillers: true,
            normalize_numbers: true,
            drop_final_period: false,
            cjk_latin_space: false,
            fullwidth_punct_in_cjk: false,
            allow_send: false,
            terminal_newline: true,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        if !SILENCE_SECONDS_OPTIONS.contains(&self.silence_seconds) {
            return Err("静音时间仅支持 1、2、3、5、10 秒".into());
        }
        if self
            .microphone
            .as_ref()
            .is_some_and(|id| id.len() > 1024 || id.chars().any(char::is_control))
        {
            return Err("麦克风 ID 无效".into());
        }
        if !(1..=365).contains(&self.history_keep_days) {
            return Err("历史记录保留天数需在 1～365 之间".into());
        }
        // 只允许 F1～F24、左右 Ctrl/Alt/Shift、CapsLock、ScrollLock 这类不会打出字的键
        let allowed = matches!(self.hotkey_vk, 0x14 | 0x91 | 0x70..=0x87 | 0xA0..=0xA5);
        if !(1..=240).contains(&self.max_listen_minutes) {
            return Err("最长听写时间需在 1～240 分钟之间".into());
        }
        if !allowed {
            return Err("快捷键只能用 F1～F24、左右 Ctrl/Alt/Shift、CapsLock 或 ScrollLock".into());
        }
        Ok(())
    }

    pub fn load_default() -> Result<Self> {
        Self::load(&Self::path()?)
    }

    pub fn save_default(&self) -> Result<()> {
        self.save(&Self::path()?)
    }

    /// 设置文件的修改时间（没有这个文件就是 None），用来发现设置被改了
    pub fn modified_default() -> Option<std::time::SystemTime> {
        std::fs::metadata(Self::path().ok()?).ok()?.modified().ok()
    }

    fn path() -> Result<PathBuf> {
        let local = std::env::var_os("LOCALAPPDATA")
            .filter(|value| !value.is_empty())
            .ok_or("LOCALAPPDATA 不可用")?;
        Ok(PathBuf::from(local).join("XueerEar/settings.json"))
    }

    fn load(path: &Path) -> Result<Self> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error.into()),
        };
        if file.metadata()?.len() > MAX_FILE_BYTES {
            return Err("设置文件超过 8 KiB".into());
        }
        let mut bytes = Vec::with_capacity(512);
        file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err("设置文件超过 8 KiB".into());
        }
        let settings: Self = serde_json::from_slice(&bytes)?;
        settings.validate()?;
        Ok(settings)
    }

    fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err("设置文件超过 8 KiB".into());
        }
        let filename = path.file_name().ok_or("设置文件路径无效")?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut nonce = [0; 8];
        getrandom::fill(&mut nonce).map_err(|_| "无法创建设置临时文件")?;
        let temporary = path.with_file_name(format!(
            ".{}.{}.{:016x}.tmp",
            filename.to_string_lossy(),
            std::process::id(),
            u64::from_le_bytes(nonce)
        ));
        // create_new ensures cleanup only touches the temporary file we own.
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let result = (|| -> Result<()> {
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            // The sibling file shares a filesystem; rename replaces an existing
            // regular file on both Windows and Unix without exposing partial JSON.
            fs::rename(&temporary, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        path: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let mut nonce = [0; 8];
            getrandom::fill(&mut nonce).unwrap();
            let root = std::env::temp_dir().join(format!(
                "wechat-settings-{}-{:016x}",
                std::process::id(),
                u64::from_le_bytes(nonce)
            ));
            fs::create_dir(&root).unwrap();
            Self {
                path: root.join("settings.json"),
                root,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
            let _ = fs::remove_dir(&self.root);
        }
    }

    #[test]
    fn missing_and_partial_settings_use_declared_defaults() {
        let fixture = Fixture::new();
        assert_eq!(Settings::load(&fixture.path).unwrap(), Settings::default());
        fs::write(&fixture.path, br#"{"microphone":"device-a"}"#).unwrap();
        assert_eq!(
            Settings::load(&fixture.path).unwrap(),
            Settings {
                microphone: Some("device-a".into()),
                silence_seconds: 3,
                ..Settings::default()
            }
        );
    }

    #[test]
    fn old_settings_files_get_text_and_history_defaults() {
        let fixture = Fixture::new();
        fs::write(&fixture.path, br#"{"microphone":null,"silence_seconds":2}"#).unwrap();
        let loaded = Settings::load(&fixture.path).unwrap();
        assert_eq!(loaded.silence_seconds, 2);
        assert_eq!(loaded.text, TextSettings::default());
        assert!(loaded.text.strip_fillers && loaded.text.normalize_numbers);
        assert!(!loaded.text.allow_send);
        assert!(loaded.history_enabled);
        assert_eq!(loaded.history_keep_days, 7);
    }

    #[test]
    fn text_and_history_settings_round_trip_and_validate() {
        let fixture = Fixture::new();
        let mut settings = Settings::default();
        settings.text.drop_final_period = true;
        settings.history_keep_days = 30;
        settings.save(&fixture.path).unwrap();
        assert_eq!(Settings::load(&fixture.path).unwrap(), settings);

        for days in [0, 366] {
            settings.history_keep_days = days;
            assert!(settings.validate().is_err(), "{days}");
        }
        settings.history_keep_days = 7;
        assert!(settings.hotkey_enabled);
        assert!(!settings.winh_enabled);
        assert_eq!(settings.max_listen_minutes, 30);
        settings.max_listen_minutes = 0;
        assert!(settings.validate().is_err());
        settings.max_listen_minutes = 241;
        assert!(settings.validate().is_err());
        settings.max_listen_minutes = 30;
        assert_eq!(settings.hotkey_vk, HOTKEY_RIGHT_ALT);
        for vk in [0x41, 0x0D, 0x20] {
            settings.hotkey_vk = vk;
            assert!(settings.validate().is_err(), "{vk:#x}");
        }
        for vk in [0x14, 0x70, 0x87, 0xA3] {
            settings.hotkey_vk = vk;
            assert!(settings.validate().is_ok(), "{vk:#x}");
        }
    }

    #[test]
    fn saves_replace_existing_json_and_invalid_changes_preserve_it() {
        let fixture = Fixture::new();
        let first = Settings {
            microphone: Some("device-a".into()),
            silence_seconds: 1,
            ..Settings::default()
        };
        first.save(&fixture.path).unwrap();
        assert_eq!(Settings::load(&fixture.path).unwrap(), first);
        let second = Settings {
            microphone: None,
            silence_seconds: 10,
            ..Settings::default()
        };
        second.save(&fixture.path).unwrap();
        assert_eq!(Settings::load(&fixture.path).unwrap(), second);
        let previous = fs::read(&fixture.path).unwrap();
        let invalid = Settings {
            silence_seconds: 4,
            ..second
        };
        assert!(invalid.save(&fixture.path).is_err());
        assert_eq!(fs::read(&fixture.path).unwrap(), previous);
        assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 1);
    }

    #[test]
    fn malformed_oversized_and_invalid_settings_are_not_silently_defaulted() {
        let fixture = Fixture::new();
        for bytes in [
            b"{".as_slice(),
            br#"{"silence_seconds":4}"#,
            br#"{"microphone":"bad\nID"}"#,
            br#"{"silence_seconds":"3"}"#,
        ] {
            fs::write(&fixture.path, bytes).unwrap();
            assert!(Settings::load(&fixture.path).is_err());
        }
        fs::write(&fixture.path, vec![b' '; MAX_FILE_BYTES as usize + 1]).unwrap();
        assert!(Settings::load(&fixture.path).is_err());
        for seconds in SILENCE_SECONDS_OPTIONS {
            assert!(
                Settings {
                    microphone: None,
                    silence_seconds: *seconds,
                    ..Settings::default()
                }
                .validate()
                .is_ok()
            );
        }
        assert!(
            Settings {
                microphone: Some("x".repeat(1024)),
                silence_seconds: 3,
                ..Settings::default()
            }
            .validate()
            .is_ok()
        );
        assert!(
            Settings {
                microphone: Some("x".repeat(1025)),
                silence_seconds: 3,
                ..Settings::default()
            }
            .validate()
            .is_err()
        );
    }
}

#[cfg(test)]
mod terminal_newline_tests {
    use super::*;

    #[test]
    fn terminal_newline_defaults_on_and_old_files_keep_it_on() {
        assert!(Settings::default().text.terminal_newline);
        let old: Settings = serde_json::from_str(r#"{"text": {"allow_send": true}}"#).unwrap();
        assert!(old.text.terminal_newline);
    }
}
