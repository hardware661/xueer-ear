//! 快捷键模式的上屏计划（纯逻辑，可在 WSL 测）：把定稿文字和语音指令变成按键序列。
//! 真正发送键盘事件在 `hotkey.rs`（只在 Windows 编）。
use xueer_ear_textflow::VoiceCommand;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    /// 一段文字，用 Unicode 键入
    Text(String),
    Enter,
    ShiftEnter,
    CtrlZ,
    /// 退格若干次
    Backspace(usize),
}

/// 记住本次快捷键会话里已经上屏的句子，用来执行「删掉上一句」和终端里的句间分隔。
#[derive(Default)]
pub struct Typist {
    sentences: Vec<String>,
}

impl Typist {
    /// 上屏一句定稿文字。终端里：换行换成空格，和上一句之间用 `join_for_terminal` 的规则补分隔。
    pub fn text(&mut self, text: &str, terminal: bool) -> Vec<Key> {
        let typed = if terminal {
            let previous = self.sentences.last().map_or("", String::as_str);
            let joined = xueer_ear_textflow::join_for_terminal(&[previous, text]);
            joined[previous.len()..].to_string()
        } else {
            text.to_string()
        };
        if typed.is_empty() {
            return vec![];
        }
        self.sentences.push(typed.clone());
        vec![Key::Text(typed)]
    }

    pub fn command(&mut self, command: VoiceCommand, terminal: bool) -> Vec<Key> {
        match command {
            VoiceCommand::NewLine => {
                self.sentences.clear();
                // 终端里 Enter 就是发送；Shift+Enter 在 Claude Code / Codex 里是换行
                vec![if terminal {
                    Key::ShiftEnter
                } else {
                    Key::Enter
                }]
            }
            VoiceCommand::DeleteLastSentence => self
                .sentences
                .pop()
                .map(|s| vec![Key::Backspace(s.chars().count())])
                .unwrap_or_default(),
            VoiceCommand::Undo => {
                self.sentences.pop();
                vec![Key::CtrlZ]
            }
            VoiceCommand::Send => {
                self.sentences.clear();
                vec![Key::Enter]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_typed_and_remembered() {
        let mut t = Typist::default();
        assert_eq!(t.text("你好", false), [Key::Text("你好".into())]);
        assert_eq!(t.text("", false), []);
    }

    #[test]
    fn consecutive_terminal_sentences_get_a_separator() {
        let mut t = Typist::default();
        assert_eq!(t.text("先读文档", true), [Key::Text("先读文档".into())]);
        assert_eq!(t.text("再跑测试", true), [Key::Text("，再跑测试".into())]);
        assert_eq!(t.text("ok", true), [Key::Text("，ok".into())]);
    }

    #[test]
    fn newlines_in_text_never_reach_a_terminal() {
        let mut t = Typist::default();
        assert_eq!(t.text("一行\n两行", true), [Key::Text("一行 两行".into())]);
    }

    #[test]
    fn delete_last_sentence_backspaces_by_chars() {
        let mut t = Typist::default();
        t.text("第一句。", false);
        t.text("第二句🙂", false);
        assert_eq!(
            t.command(VoiceCommand::DeleteLastSentence, false),
            [Key::Backspace(4)]
        );
        assert_eq!(
            t.command(VoiceCommand::DeleteLastSentence, false),
            [Key::Backspace(4)]
        );
        assert_eq!(t.command(VoiceCommand::DeleteLastSentence, false), []);
    }

    #[test]
    fn newline_in_terminal_is_shift_enter_not_enter() {
        let mut t = Typist::default();
        assert_eq!(t.command(VoiceCommand::NewLine, false), [Key::Enter]);
        assert_eq!(t.command(VoiceCommand::NewLine, true), [Key::ShiftEnter]);
    }

    #[test]
    fn send_and_undo() {
        let mut t = Typist::default();
        t.text("提交", true);
        assert_eq!(t.command(VoiceCommand::Send, true), [Key::Enter]);
        // 发出去以后不能再删
        assert_eq!(t.command(VoiceCommand::DeleteLastSentence, true), []);
        assert_eq!(t.command(VoiceCommand::Undo, false), [Key::CtrlZ]);
    }
}
