//! User settings are snapshotted when the next Win+H capture starts.
use crate::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use xueer_ear_textflow::{CommandOptions, VoiceCommand};

/// 设置文件上限（正常不到 1 KB；1.3 第二轮的设置文件里可能还留着改过的说法）
const MAX_FILE_BYTES: u64 = 16 * 1024;
/// 右 Alt 的虚拟键码
pub const HOTKEY_RIGHT_ALT: u32 = 0xA5;
pub const SILENCE_SECONDS_OPTIONS: &[u32] = &[1, 2, 3, 5, 10];
/// 说完指令后停多久执行（毫秒）的几档；设置文件里手写别的值，200～3000 之间也认
pub const SETTLE_MS_OPTIONS: &[u32] = &[300, 600, 1000, 1500];
/// 指令前要停多久才算单独说（毫秒）的几档；手写的 300～3000 之间也认
pub const PAUSE_MS_OPTIONS: &[u32] = &[500, 1000, 1500, 2000];
/// 多久没有新字自动结束（秒）的几档；手写的 3～600 之间也认
pub const QUIET_STOP_OPTIONS: &[u32] = &[5, 10, 20, 30, 60];
/// 一次最长听多久（分钟）的几档；手写的 1～240 之间也认
pub const MAX_LISTEN_OPTIONS: &[u32] = &[10, 30, 60, 120];

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
    /// 这么久没识别出新字，就自己结束这一轮（秒；主人 2026-11-15：「10秒钟就比较合适」）
    pub quiet_stop_seconds: u32,
    /// 晓晓提示音（没听清、系统更新后退回），本地播放
    pub sounds_enabled: bool,
    /// 以管理员身份运行（往管理员权限的窗口里打字要用）。默认关：普通权限就够，打开时不弹确认框
    pub run_as_admin: bool,
    /// 主通道：默认豆包（主人 2026-11-23 定）。另一家一直是备用：主通道连不上、出错的那句自动改走备用
    pub engine: Engine,
    /// 语音指令：哪些开着、前后各停多久
    pub commands: CommandSettings,
    /// 降噪：断句前先滤掉风扇、空调这种一直响的底噪（发给云端的仍是原声）
    pub noise_reduction: bool,
    /// 听写时把别的程序的声音调小，说完恢复
    pub duck_others: bool,
}

/// 主通道走哪家的云端；另一家是备用（主人 2026-11-24：两家互为备用，选哪家哪家为主）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    /// 豆包输入法为主，微信输入法备用
    #[default]
    Doubao,
    /// 微信输入法为主，豆包输入法备用
    Wetype,
}

/// 语音指令（控制面板「语音指令」那一行）。说法固定，规则见 textflow 的 commands 模块
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CommandSettings {
    /// 改过开关的指令，键是指令的 id（如 "send"）；没写的用默认（「发送」关，别的开）
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub switches: BTreeMap<String, bool>,
    /// 指令前要停这么久（毫秒）才算单独说；没停够的指令当普通字打出来（主人 2026-11-24：要强调等待）
    pub pause_ms: u32,
    /// 单独说完一个指令、停这么久没再出新字就执行，不等静音收句（毫秒）
    pub settle_ms: u32,
}

impl Default for CommandSettings {
    fn default() -> Self {
        Self {
            switches: BTreeMap::new(),
            pause_ms: 1000,
            settle_ms: 600,
        }
    }
}

impl CommandSettings {
    pub fn pause(&self) -> Duration {
        Duration::from_millis(u64::from(self.pause_ms))
    }

    pub fn settle(&self) -> Duration {
        Duration::from_millis(u64::from(self.settle_ms))
    }
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
            quiet_stop_seconds: 10,
            run_as_admin: false,
            engine: Engine::Doubao,
            commands: CommandSettings::default(),
            noise_reduction: true,
            duck_others: true,
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
    /// 用软件自带的通用热词表（AI/编程常听错的词，见 textflow 的 builtin_hotwords.txt）
    pub builtin_hotwords: bool,
}

impl Default for TextSettings {
    fn default() -> Self {
        Self {
            strip_fillers: true,
            normalize_numbers: true,
            drop_final_period: false,
            cjk_latin_space: false,
            fullwidth_punct_in_cjk: false,
            builtin_hotwords: true,
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
        self.validate_commands()
    }

    fn validate_commands(&self) -> Result<()> {
        let commands = &self.commands;
        if !(300..=3000).contains(&commands.pause_ms) {
            return Err("指令前停顿的时间需在 0.3～3 秒之间".into());
        }
        if !(200..=3000).contains(&commands.settle_ms) {
            return Err("说完指令后等待的时间需在 0.2～3 秒之间".into());
        }
        if !(3..=600).contains(&self.quiet_stop_seconds) {
            return Err("没有新字自动结束的时间需在 3～600 秒之间".into());
        }
        if let Some(id) = commands
            .switches
            .keys()
            .find(|id| VoiceCommand::from_id(id).is_none())
        {
            return Err(format!("不认识的语音指令「{id}」").into());
        }
        Ok(())
    }

    /// 语音指令现在哪些开着（没改过的用默认）
    pub fn command_options(&self) -> CommandOptions {
        CommandOptions::with_switches(|command| self.commands.switches.get(command.id()).copied())
    }

    /// 多久没识别出新字就结束这一轮
    pub fn quiet_stop(&self) -> Duration {
        Duration::from_secs(u64::from(self.quiet_stop_seconds))
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
        Ok(Self::data_dir()?.join("settings.json"))
    }

    /// 数据目录 %LOCALAPPDATA%\XueerEar（设置、热词、快捷短语、历史）
    pub fn data_dir() -> Result<PathBuf> {
        let local = std::env::var_os("LOCALAPPDATA")
            .filter(|value| !value.is_empty())
            .ok_or("LOCALAPPDATA 不可用")?;
        Ok(PathBuf::from(local).join("XueerEar"))
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
        let raw: serde_json::Value = serde_json::from_slice(&bytes)?;
        let mut settings: Self = serde_json::from_value(raw.clone())?;
        settings.upgrade(&raw);
        settings.validate()?;
        Ok(settings)
    }

    /// 旧版设置文件里挪了地方的项（下次保存时旧字段自然去掉）：
    /// - 1.2 及以前「说「发送」时按回车」是 text.allow_send；
    /// - 1.3 第二轮的指令有「说法」列表：清空 = 关掉，「发送」有说法 = 开着；
    /// - 「多久没有新字自动结束」以前在 commands 里。
    ///
    /// 新的开关已经写了的以新的为准
    fn upgrade(&mut self, raw: &serde_json::Value) {
        let mut chosen: Vec<(String, bool)> = Vec::new();
        if raw
            .pointer("/text/allow_send")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        {
            chosen.push((VoiceCommand::Send.id().to_owned(), true));
        }
        if let Some(words) = raw
            .pointer("/commands/words")
            .and_then(serde_json::Value::as_object)
        {
            for (id, list) in words {
                let Some(command) = VoiceCommand::from_id(id) else {
                    continue;
                };
                let said = list.as_array().is_some_and(|list| !list.is_empty());
                if !said || !command.on_by_default() {
                    chosen.push((id.clone(), said));
                }
            }
        }
        for (id, on) in chosen {
            self.commands.switches.entry(id).or_insert(on);
        }
        if raw.get("quiet_stop_seconds").is_none()
            && let Some(seconds) = raw
                .pointer("/commands/quiet_stop_seconds")
                .and_then(serde_json::Value::as_u64)
        {
            self.quiet_stop_seconds = u32::try_from(seconds).unwrap_or(u32::MAX);
        }
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

    #[test]
    fn engine_defaults_to_doubao_and_round_trips() {
        assert_eq!(Settings::default().engine, Engine::Doubao);
        let old: Settings = serde_json::from_str(r#"{"silence_seconds":3}"#).unwrap();
        assert_eq!(old.engine, Engine::Doubao);
        let chosen: Settings = serde_json::from_str(r#"{"engine":"wetype"}"#).unwrap();
        assert_eq!(chosen.engine, Engine::Wetype);
        let json = serde_json::to_string(&chosen).unwrap();
        assert!(json.contains(r#""engine":"wetype""#));
        assert!(serde_json::from_str::<Settings>(r#"{"engine":"other"}"#).is_err());
    }

    #[test]
    fn noise_reduction_and_ducking_default_on_for_old_settings_files() {
        let old: Settings = serde_json::from_str(r#"{"silence_seconds":3}"#).unwrap();
        assert!(old.noise_reduction && old.duck_others);
        let off: Settings =
            serde_json::from_str(r#"{"noise_reduction":false,"duck_others":false}"#).unwrap();
        assert!(!off.noise_reduction && !off.duck_others);
    }

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
        assert!(!loaded.command_options().is_on(VoiceCommand::Send));
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
mod command_tests {
    use super::*;

    fn load(json: &str) -> Result<Settings> {
        // 每次一个新目录：测试并行跑时，长度一样的两段设置不能写到同一个文件里
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "xueer-commands-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("settings.json");
        fs::write(&path, json).unwrap();
        let loaded = Settings::load(&path);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&root);
        loaded
    }

    #[test]
    fn command_defaults() {
        let s = Settings::default();
        assert_eq!(s.commands.pause(), Duration::from_secs(1));
        assert_eq!(s.commands.settle(), Duration::from_millis(600));
        assert_eq!(s.quiet_stop(), Duration::from_secs(10));
        let options = s.command_options();
        assert!(!options.is_on(VoiceCommand::Send), "发送默认关");
        assert!(options.is_on(VoiceCommand::DeleteLastSentence));
        // 没改过开关就不往文件里写
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("switches"), "{json}");
    }

    #[test]
    fn switches_and_times_round_trip() {
        let loaded = load(
            r#"{"quiet_stop_seconds": 30, "commands": {"switches": {"send": true, "delete_all": false},
                "pause_ms": 1500, "settle_ms": 1000}}"#,
        )
        .unwrap();
        let options = loaded.command_options();
        assert!(options.is_on(VoiceCommand::Send));
        assert!(!options.is_on(VoiceCommand::DeleteAll));
        assert_eq!(loaded.commands.pause(), Duration::from_millis(1500));
        assert_eq!(loaded.commands.settle(), Duration::from_secs(1));
        assert_eq!(loaded.quiet_stop(), Duration::from_secs(30));
        let again: Settings =
            serde_json::from_str(&serde_json::to_string(&loaded).unwrap()).unwrap();
        assert_eq!(again, loaded);
    }

    // 主人 2026-11-24 前的设置文件：「发送」在 1.2 是 text.allow_send，1.3 第二轮是「说法」列表；
    // 「多久没有新字自动结束」以前放在 commands 里
    #[test]
    fn older_settings_files_keep_what_the_owner_chose() {
        let owner = load(
            r#"{"silence_seconds": 1, "text": {"terminal_newline": true},
                "commands": {"words": {"send": ["发送"]}, "plain_translate": "auto",
                "settle_ms": 600, "quiet_stop_seconds": 20}}"#,
        )
        .unwrap();
        assert!(owner.command_options().is_on(VoiceCommand::Send));
        assert_eq!(owner.quiet_stop(), Duration::from_secs(20));
        assert_eq!(owner.silence_seconds, 1);
        let json = serde_json::to_string(&owner).unwrap();
        for gone in [r#""words""#, "plain_translate", "terminal_newline"] {
            assert!(!json.contains(gone), "{gone}: {json}");
        }
        // 1.2 的「说发送时按回车」
        let on = load(r#"{"text": {"allow_send": true}}"#).unwrap();
        assert!(on.command_options().is_on(VoiceCommand::Send));
        let off = load(r#"{"text": {"allow_send": false}}"#).unwrap();
        assert!(!off.command_options().is_on(VoiceCommand::Send));
        // 第二轮里把指令的说法清空 = 关掉，换成说法的照样开着；已经没有的指令不管
        let cleared = load(
            r#"{"commands": {"words": {"delete_all": [], "delete_last": ["撤回"], "new_line": []}}}"#,
        )
        .unwrap();
        let options = cleared.command_options();
        assert!(!options.is_on(VoiceCommand::DeleteAll));
        assert!(options.is_on(VoiceCommand::DeleteLastSentence));
        // 新的开关已经写了的，以它为准
        let chosen =
            load(r#"{"text": {"allow_send": true}, "commands": {"switches": {"send": false}}}"#)
                .unwrap();
        assert!(!chosen.command_options().is_on(VoiceCommand::Send));
    }

    #[test]
    fn broken_command_settings_are_refused() {
        for json in [
            r#"{"commands": {"switches": {"other": true}}}"#,
            r#"{"commands": {"settle_ms": 50}}"#,
            r#"{"commands": {"pause_ms": 100}}"#,
            r#"{"commands": {"pause_ms": 5000}}"#,
            r#"{"quiet_stop_seconds": 1}"#,
        ] {
            assert!(load(json).is_err(), "{json}");
        }
    }

    #[test]
    fn builtin_hotwords_default_on_for_old_files() {
        let old: Settings = serde_json::from_str(r#"{"text": {"strip_fillers": true}}"#).unwrap();
        assert!(Settings::default().text.builtin_hotwords);
        assert!(old.text.builtin_hotwords);
    }
}
