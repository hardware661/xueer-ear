//! 快捷键模式的上屏计划（纯逻辑，可在 WSL 测）：把定稿文字和语音指令变成按键序列。
//! 真正发送键盘事件在 `hotkey.rs`（只在 Windows 编）。
use crate::revision::Revision;
use xueer_ear_textflow::{Output, VoiceCommand};

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

/// 面板听写：边说边出字。中间结果直接打到光标处，后来的结果只改还没说定的那一截；
/// 定稿时换成整理过的文字（或执行语音指令）。
pub struct Live {
    revision: Revision,
    typist: Typist,
    terminal: bool,
    last_rewrite: Option<std::time::Instant>,
}

/// 退格改字离上次屏幕变化至少隔这么久
const REWRITE_GAP: std::time::Duration = std::time::Duration::from_millis(250);

impl Live {
    pub fn new(terminal: bool) -> Self {
        Self {
            revision: Revision::default(),
            typist: Typist::default(),
            terminal,
            last_rewrite: None,
        }
    }

    /// 中间结果：退掉上次没说定的部分里变了的字，再补上新的。
    pub fn partial(&mut self, text: &str) -> Vec<Key> {
        self.partial_at(text, std::time::Instant::now())
    }

    /// 同上，带时间。只往后加字的立刻打；要退格改前面的字，离上次屏幕变化至少隔 `REWRITE_GAP`，
    /// 其间的修订先攒着（下一次或定稿时一起改），免得云端频繁修订时屏幕来回删写、一直闪。
    pub fn partial_at(&mut self, text: &str, now: std::time::Instant) -> Vec<Key> {
        let text = self.separated(&clean(text));
        let Ok(update) = self.revision.prepare(&text, false) else {
            return vec![];
        };
        // 要退格改字时，离上次屏幕变化（打字或改字）不到 REWRITE_GAP 就先不动
        if update.erase > 0
            && self
                .last_rewrite
                .is_some_and(|at| now.duration_since(at) < REWRITE_GAP)
        {
            return vec![];
        }
        if update.erase > 0 || !update.insert.is_empty() {
            self.last_rewrite = Some(now);
        }
        let keys = edit(update.erase, &update.insert);
        self.revision.accept(update);
        keys
    }

    /// 定稿：把屏幕上没说定的那一截整个换成整理后的文字，或者擦掉后执行指令。
    pub fn finalize(&mut self, output: Output) -> Vec<Key> {
        let erase = self.revision.pending_graphemes();
        self.revision.clear();
        let mut keys = edit(erase, "");
        keys.extend(match output {
            Output::Text(text) => self.typist.text(&clean(&text), self.terminal),
            Output::Command(command) => self.typist.command(command, self.terminal),
        });
        keys
    }

    /// 终端里和上一句之间的分隔要在出中间结果时就带上，定稿时由 typist 统一处理。
    fn separated(&self, text: &str) -> String {
        if !self.terminal || text.is_empty() {
            return text.to_string();
        }
        let previous = self.typist.sentences.last().map_or("", String::as_str);
        let joined = xueer_ear_textflow::join_for_terminal(&[previous, text]);
        joined[previous.len()..].to_string()
    }
}

fn clean(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

fn edit(erase: usize, insert: &str) -> Vec<Key> {
    let mut keys = Vec::new();
    if erase > 0 {
        keys.push(Key::Backspace(erase));
    }
    if !insert.is_empty() {
        keys.push(Key::Text(insert.to_string()));
    }
    keys
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

    /// 把按键序列应用到一段文字上，模拟编辑器
    fn type_into(doc: &mut String, keys: &[Key]) {
        for k in keys {
            match k {
                Key::Text(t) => doc.push_str(t),
                Key::Backspace(n) => {
                    for _ in 0..*n {
                        doc.pop();
                    }
                }
                Key::Enter | Key::ShiftEnter => doc.push('\n'),
                Key::CtrlZ => {}
            }
        }
    }

    #[test]
    fn live_rewrites_are_throttled_but_appends_flow() {
        let t0 = std::time::Instant::now();
        let ms = |n| t0 + std::time::Duration::from_millis(n);
        let mut live = Live::new(false);
        let mut doc = String::new();
        type_into(&mut doc, &live.partial_at("今天", ms(0)));
        // 只往后加字：马上打
        type_into(&mut doc, &live.partial_at("今天天", ms(40)));
        assert_eq!(doc, "今天天");
        // 要改前面的字：离上次改不到 250 ms 就先不动，免得屏幕来回删写
        assert!(live.partial_at("今天田", ms(80)).is_empty());
        let keys = live.partial_at("今天天气", ms(120));
        assert!(keys.iter().all(|k| matches!(k, Key::Text(_))), "{keys:?}");
        type_into(&mut doc, &keys);
        assert_eq!(doc, "今天天气");
        type_into(&mut doc, &live.partial_at("今天天器很", ms(500)));
        assert_eq!(doc, "今天天器很");
        // 定稿永远照写
        type_into(
            &mut doc,
            &live.finalize(Output::Text("今天天气很好".into())),
        );
        assert_eq!(doc, "今天天气很好");
    }

    #[test]
    fn live_partials_revise_only_the_tail() {
        let t0 = std::time::Instant::now();
        let ms = |n| t0 + std::time::Duration::from_millis(n);
        let mut live = Live::new(false);
        let mut doc = String::from("原有内容");
        type_into(&mut doc, &live.partial_at("今天", ms(0)));
        type_into(&mut doc, &live.partial_at("今天天气", ms(300)));
        type_into(&mut doc, &live.partial_at("今天天器很好", ms(600)));
        assert_eq!(doc, "原有内容今天天器很好");
        type_into(
            &mut doc,
            &live.finalize(Output::Text("今天天气很好。".into())),
        );
        assert_eq!(doc, "原有内容今天天气很好。");
        // 下一句从定稿后面接着写，不动前一句
        type_into(&mut doc, &live.partial("下一句"));
        assert_eq!(doc, "原有内容今天天气很好。下一句");
    }

    #[test]
    fn live_final_cleanup_replaces_spoken_text() {
        let mut live = Live::new(false);
        let mut doc = String::new();
        type_into(&mut doc, &live.partial("嗯百分之五十"));
        type_into(&mut doc, &live.finalize(Output::Text("50%".into())));
        assert_eq!(doc, "50%");
    }

    #[test]
    fn live_command_erases_spoken_words_then_acts() {
        let mut live = Live::new(false);
        let mut doc = String::from("第一句。");
        live.finalize(Output::Text(String::new()));
        type_into(&mut doc, &live.partial("换行"));
        let keys = live.finalize(Output::Command(VoiceCommand::NewLine));
        type_into(&mut doc, &keys);
        assert_eq!(doc, "第一句。\n");
    }

    #[test]
    fn live_terminal_joins_sentences_and_never_enters() {
        let mut live = Live::new(true);
        let mut doc = String::new();
        type_into(&mut doc, &live.partial("先读文档"));
        type_into(&mut doc, &live.finalize(Output::Text("先读文档".into())));
        type_into(&mut doc, &live.partial("再跑"));
        type_into(&mut doc, &live.partial("再跑测试"));
        type_into(&mut doc, &live.finalize(Output::Text("再跑测试".into())));
        assert_eq!(doc, "先读文档，再跑测试");
    }

    #[test]
    fn live_empty_final_erases_partial() {
        let mut live = Live::new(false);
        let mut doc = String::from("x");
        type_into(&mut doc, &live.partial("嗯"));
        type_into(&mut doc, &live.finalize(Output::Text(String::new())));
        assert_eq!(doc, "x");
    }

    #[test]
    fn live_control_characters_become_spaces() {
        let mut live = Live::new(false);
        let mut doc = String::new();
        type_into(&mut doc, &live.partial("a\nb"));
        assert_eq!(doc, "a b");
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
