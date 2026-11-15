//! 定稿文字上屏前的整理（textflow）和上屏后的历史记录（history）。
//!
//! 只处理定稿（final_result）；边说边改的中间结果原样上屏，免得数字、口癖在中途跳变。
//! 整句是语音指令（换行、撤销……）时：Win+H 路径原样上屏；快捷键路径用 `finalize` 拿到指令去按键。
use crate::settings::Settings;
use std::path::Path;
use xueer_ear_history::{Entry, History, Settings as HistorySettings};
use xueer_ear_protocol::transcript::SpeechEvent;
use xueer_ear_textflow::{
    CommandOptions, Hotwords, Options, Output, PunctOptions, VoiceCommand, parse_command, process,
    strip_fillers,
};

/// 热词文件上限，防止误放一个大文件拖慢每句话。
const MAX_HOTWORDS_BYTES: u64 = 256 * 1024;

pub struct Postprocess {
    options: Options,
    history: History,
}

impl Postprocess {
    /// `dir` 是数据目录（%LOCALAPPDATA%\XueerEar），里面放 hotwords.txt 和 history.jsonl。
    pub fn new(settings: &Settings, dir: &Path) -> Self {
        let text = &settings.text;
        let options = Options {
            hotwords: read_hotwords(&dir.join("hotwords.txt")),
            punct: PunctOptions {
                drop_final_period: text.drop_final_period,
                cjk_latin_space: text.cjk_latin_space,
                fullwidth_punct_in_cjk: text.fullwidth_punct_in_cjk,
            },
            commands: CommandOptions {
                allow_send: text.allow_send,
            },
            strip_fillers: text.strip_fillers,
            normalize_numbers: text.normalize_numbers,
        };
        let history = History::new(
            dir.join("history.jsonl"),
            HistorySettings {
                enabled: settings.history_enabled,
                keep_days: settings.history_keep_days,
            },
        );
        Self { options, history }
    }

    /// 整理一条识别事件（Win+H 路径）；定稿且非空时写一条历史。指令先原样上屏。
    pub fn apply(&self, event: SpeechEvent, now_ms: u64) -> SpeechEvent {
        if !event.final_result {
            return event;
        }
        let text = match self.finalize(&event.text, now_ms) {
            Output::Text(text) | Output::TextThen(text, _) => text,
            Output::Command(_) => event.text,
        };
        SpeechEvent {
            final_result: true,
            text,
        }
    }

    /// 整理一句定稿文字（快捷键路径能执行指令，所以把指令原样交回）。
    /// 是文字且非空时写一条历史；历史写失败只打日志，不影响上屏。
    pub fn finalize(&self, text: &str, now_ms: u64) -> Output {
        let output = process(text, &self.options);
        if let Output::Text(text) | Output::TextThen(text, _) = &output {
            if !text.is_empty() {
                let entry = Entry {
                    at_ms: now_ms,
                    text: text.clone(),
                    app: String::new(),
                };
                if let Err(error) = self.history.append(&entry) {
                    eprintln!("历史记录写入失败：{error}");
                }
            }
        }
        output
    }

    /// 一句还没定稿时，这句话是不是整句只有一个语音指令（「删除」「撤回」「发送」……）。
    /// 是的话快捷键路径可以马上执行，不用等静音收句。句中带指令词的（「删除这个文件」）不算。
    pub fn command_only(&self, text: &str) -> Option<VoiceCommand> {
        let text = if self.options.strip_fillers {
            strip_fillers(text)
        } else {
            text.to_string()
        };
        parse_command(&text, &self.options.commands)
    }

    /// 听写出错中断时，屏幕上那截还没说定的话：原样记进历史（不整理、不当指令），免得白说。
    pub fn keep_unfinished(&self, text: &str, now_ms: u64) {
        if text.trim().is_empty() {
            return;
        }
        let entry = Entry {
            at_ms: now_ms,
            text: text.to_string(),
            app: String::new(),
        };
        if let Err(error) = self.history.append(&entry) {
            eprintln!("历史记录写入失败：{error}");
        }
    }

    /// 启动时调用一次：清掉超过保留天数的历史。
    pub fn prune_history(&self, now_ms: u64) {
        if let Err(error) = self.history.prune(now_ms) {
            eprintln!("历史记录清理失败：{error}");
        }
    }
}

fn read_hotwords(path: &Path) -> Hotwords {
    let Ok(meta) = std::fs::metadata(path) else {
        return Hotwords::default();
    };
    if meta.len() > MAX_HOTWORDS_BYTES {
        eprintln!("热词文件超过 256 KiB，已忽略");
        return Hotwords::default();
    }
    match std::fs::read_to_string(path) {
        Ok(text) => Hotwords::parse(&text),
        Err(error) => {
            eprintln!("热词文件读取失败：{error}");
            Hotwords::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use xueer_ear_textflow::VoiceCommand;

    fn final_event(text: &str) -> SpeechEvent {
        SpeechEvent {
            final_result: true,
            text: text.into(),
        }
    }

    fn setup(settings: &Settings) -> (tempfile::TempDir, Postprocess) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("hotwords.txt"), "雪耳\nNew API\n").unwrap();
        let p = Postprocess::new(settings, dir.path());
        (dir, p)
    }

    #[test]
    fn final_text_is_cleaned_and_recorded() {
        let (dir, p) = setup(&Settings::default());
        let out = p.apply(final_event("嗯，重启new api，雪而百分之五十"), 1_000);
        assert_eq!(out, final_event("重启New API，雪耳50%"));
        let history = fs::read_to_string(dir.path().join("history.jsonl")).unwrap();
        assert!(history.contains("重启New API，雪耳50%"), "{history}");
    }

    #[test]
    fn partial_text_passes_through_untouched_and_unrecorded() {
        let (dir, p) = setup(&Settings::default());
        let partial = SpeechEvent {
            final_result: false,
            text: "嗯，百分之".into(),
        };
        assert_eq!(p.apply(partial.clone(), 1_000), partial);
        assert!(!dir.path().join("history.jsonl").exists());
    }

    #[test]
    fn voice_commands_stay_as_spoken_text_for_now() {
        // 执行指令需要按键，属于 Win+H 接管部分，这里先原样上屏
        let (_dir, p) = setup(&Settings::default());
        assert_eq!(p.apply(final_event("换行。"), 1).text, "换行。");
    }

    #[test]
    fn finalize_hands_back_commands_without_recording() {
        let (dir, p) = setup(&Settings::default());
        assert_eq!(
            p.finalize("换行。", 1),
            Output::Command(xueer_ear_textflow::VoiceCommand::NewLine)
        );
        assert!(!dir.path().join("history.jsonl").exists());
    }

    #[test]
    fn switches_off_cleanup_and_history() {
        let mut settings = Settings::default();
        settings.text.strip_fillers = false;
        settings.text.normalize_numbers = false;
        settings.history_enabled = false;
        let (dir, p) = setup(&settings);
        assert_eq!(
            p.apply(final_event("嗯，百分之五十"), 1).text,
            "嗯，百分之五十"
        );
        assert!(!dir.path().join("history.jsonl").exists());
    }

    #[test]
    fn missing_hotword_file_and_empty_finals_are_fine() {
        let dir = tempfile::tempdir().unwrap();
        let p = Postprocess::new(&Settings::default(), dir.path());
        assert_eq!(p.apply(final_event("雪而"), 1).text, "雪而");
        assert_eq!(p.apply(final_event("嗯。"), 2).text, "");
        let history = fs::read_to_string(dir.path().join("history.jsonl")).unwrap();
        assert_eq!(history.lines().count(), 1, "空的定稿不该记历史：{history}");
    }

    #[test]
    fn unfinished_text_is_kept_in_history_as_is() {
        // 出错时屏幕上那截没说定的话：原样记一条，不整理、不当指令
        let (dir, p) = setup(&Settings::default());
        p.keep_unfinished("嗯，删除", 2_000);
        p.keep_unfinished("  ", 2_001);
        let history = fs::read_to_string(dir.path().join("history.jsonl")).unwrap();
        assert_eq!(history.lines().count(), 1, "{history}");
        assert!(history.contains("嗯，删除"), "{history}");
        // 历史关着就不记
        let mut off = Settings::default();
        off.history_enabled = false;
        let (dir, p) = setup(&off);
        p.keep_unfinished("没说完的话", 2_000);
        assert!(!dir.path().join("history.jsonl").exists());
    }

    #[test]
    fn only_a_whole_sentence_command_counts_as_early_command() {
        let (_dir, p) = setup(&Settings::default());
        assert_eq!(
            p.command_only("删除"),
            Some(VoiceCommand::DeleteLastSentence)
        );
        assert_eq!(
            p.command_only("嗯，撤回。"),
            Some(VoiceCommand::DeleteLastSentence)
        );
        assert_eq!(p.command_only("全部删除"), Some(VoiceCommand::DeleteAll));
        assert_eq!(p.command_only("删除这个文件"), None);
        assert_eq!(p.command_only("这句不对，删掉上一句"), None);
        // 「发送」只有开关打开时才算
        assert_eq!(p.command_only("发送"), None);
        let mut on = Settings::default();
        on.text.allow_send = true;
        let (_dir, p) = setup(&on);
        assert_eq!(p.command_only("发送"), Some(VoiceCommand::Send));
    }
}
