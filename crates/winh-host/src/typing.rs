//! 快捷键模式的上屏计划（纯逻辑，可在 WSL 测）：把定稿文字和语音指令变成按键序列。
//! 真正发送键盘事件在 `hotkey.rs`（只在 Windows 编）。
use crate::revision::Revision;
use xueer_ear_textflow::{Output, VoiceCommand};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    /// 一段文字，用 Unicode 键入
    Text(String),
    Enter,
    /// 终端里换行不发送（Claude Code 文档：Ctrl+J 在所有终端都是换行；模拟的 Shift+Enter 在 Windows Terminal 里被当成回车发出去了）
    CtrlJ,
    /// 退格若干次
    Backspace(usize),
}

/// 这次听写里雪耳已经上屏的东西，一条一条记着，用来往回删和在终端里补句间分隔。
#[derive(Default)]
pub struct Typist {
    typed: Vec<Typed>,
}

enum Typed {
    Sentence(String),
    /// 语音「换行」打出来的那个换行
    NewLine,
}

impl Typed {
    /// 删掉它要退格几次
    fn width(&self) -> usize {
        match self {
            Self::Sentence(text) => text.chars().count(),
            Self::NewLine => 1,
        }
    }
}

impl Typist {
    /// 当前这一行里的上一句（换行之后是空的）
    fn previous(&self) -> &str {
        match self.typed.last() {
            Some(Typed::Sentence(text)) => text,
            _ => "",
        }
    }

    /// 主人自己按 Enter 把内容发走了：之前打的都不在输入框里了
    pub fn forget(&mut self) {
        self.typed.clear();
    }

    /// 上屏一句定稿文字。终端里：换行换成空格，和上一句之间用 `join_for_terminal` 的规则补分隔。
    pub fn text(&mut self, text: &str, terminal: bool) -> Vec<Key> {
        let typed = if terminal {
            let previous = self.previous();
            let joined = xueer_ear_textflow::join_for_terminal(&[previous, text]);
            joined[previous.len()..].to_string()
        } else {
            text.to_string()
        };
        if typed.is_empty() {
            return vec![];
        }
        self.typed.push(Typed::Sentence(typed.clone()));
        vec![Key::Text(typed)]
    }

    pub fn command(&mut self, command: VoiceCommand, terminal: bool) -> Vec<Key> {
        match command {
            VoiceCommand::NewLine => {
                self.typed.push(Typed::NewLine);
                // 终端里 Enter 就是发送，用 Ctrl+J 换行（2026-11-15 主人真机：Shift+Enter 被发送出去了）
                vec![if terminal { Key::CtrlJ } else { Key::Enter }]
            }
            // 删掉上一句；它前面紧挨着的换行也一起删，这样能一路删回上一行
            // （主人 2026-11-15：换行之后只能删第二行）。不发 Ctrl+Z：终端里那是挂起前台程序。
            VoiceCommand::DeleteLastSentence => {
                let mut count = 0;
                while matches!(self.typed.last(), Some(Typed::NewLine)) {
                    count += self.typed.pop().map_or(0, |t| t.width());
                }
                count += self.typed.pop().map_or(0, |t| t.width());
                backspaces(count)
            }
            VoiceCommand::DeleteAll => {
                let count = self.typed.drain(..).map(|t| t.width()).sum();
                backspaces(count)
            }
            VoiceCommand::Send => {
                self.typed.clear();
                vec![Key::Enter]
            }
        }
    }
}

fn backspaces(count: usize) -> Vec<Key> {
    if count == 0 {
        vec![]
    } else {
        vec![Key::Backspace(count)]
    }
}

/// 面板听写：边说边出字。中间结果直接打到光标处，后来的结果只改还没说定的那一截；
/// 定稿时换成整理过的文字（或执行语音指令）。
pub struct Live {
    revision: Revision,
    typist: Typist,
    terminal: bool,
    last_rewrite: Option<std::time::Instant>,
    /// 主人按 Enter 时屏幕上那截还没说定的文字（已经被发走了），这句后续结果要去掉它
    sent: Option<String>,
    /// 主人在这句说到一半时动了键盘或鼠标：光标可能已经不在原处，这句剩下的时间只往后接字。
    /// 值是冻结时这句已经上屏的字数。
    frozen: Option<usize>,
    /// 这句开头的语音指令已经提前执行了：后续结果里先去掉这个指令词再上屏
    early: Option<String>,
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
            sent: None,
            frozen: None,
            early: None,
        }
    }

    /// 这句到目前为止只说了一个指令词（调用方用 Postprocess::command_only 判断过），马上执行，
    /// 不等静音收句：先擦掉屏幕上的指令词，再按指令按键。之后这句的结果会去掉开头的指令词。
    /// 主人动过键盘鼠标（冻结）时，删除类指令不执行，交给定稿时照常处理。
    pub fn early_command(&mut self, command: VoiceCommand) -> Vec<Key> {
        if self.early.is_some() {
            return vec![];
        }
        if self.frozen.is_some()
            && matches!(
                command,
                VoiceCommand::DeleteLastSentence | VoiceCommand::DeleteAll
            )
        {
            return vec![];
        }
        let spoken = self.revision.pending().to_string();
        let erase = if self.frozen.is_some() {
            0
        } else {
            self.revision.pending_graphemes()
        };
        self.revision.clear();
        self.frozen = self.frozen.map(|_| 0);
        self.early = Some(spoken.trim().to_string());
        let mut keys = edit(erase, "");
        keys.extend(self.typist.command(command, self.terminal));
        keys
    }

    /// 主人自己按了 Enter：屏幕上没说定的那截已经跟着发走了，输入框是空的。
    pub fn user_sent(&mut self) {
        let shown = self.revision.pending().to_string();
        self.revision.clear();
        self.typist.forget();
        if !shown.is_empty() {
            let previous = self.sent.take().unwrap_or_default();
            self.sent = Some(previous + &shown);
        }
    }

    /// 屏幕上还没说定的那一截。听写出错时这截字留在屏幕上，调用方把它存进历史。
    pub fn leftover(&self) -> &str {
        self.revision.pending()
    }

    /// 主人自己动了键盘或鼠标（点了别处、打了字、选了文字）。雪耳改字靠「退格再重打」，
    /// 不知道光标在哪时再退格就会删错地方，所以这句剩下的时间只往后接字，之前打的也不再往回删。
    pub fn user_touched(&mut self) {
        if self.frozen.is_none() {
            self.frozen = Some(self.revision.pending_graphemes());
        }
        self.typist.forget();
    }

    /// 冻结后：按字数跳过已经上屏的部分，只把多出来的字接上。
    fn append_only(&mut self, text: &str) -> Vec<Key> {
        use unicode_segmentation::UnicodeSegmentation;
        let shown = self.frozen.unwrap_or(0);
        let graphemes: Vec<&str> = text.graphemes(true).collect();
        if graphemes.len() <= shown {
            return vec![];
        }
        let extra: String = graphemes[shown..].concat();
        self.frozen = Some(graphemes.len());
        vec![Key::Text(extra)]
    }

    /// 去掉这句里已经被 Enter 发走的开头，只剩还没上屏的部分。
    /// 云端定稿常在发走的文字后面补标点或改一两个字，所以按「去掉标点后的前缀」对齐。
    pub fn unsent(&self, text: &str) -> String {
        let text = &self.without_early(text);
        let Some(sent) = self.sent.as_deref() else {
            return text.to_string();
        };
        let wanted: Vec<char> = sent.chars().filter(|c| !is_punct(*c)).collect();
        let mut matched = 0;
        let mut cut = 0;
        for (i, c) in text.char_indices() {
            if matched == wanted.len() {
                break;
            }
            if is_punct(c) {
                continue;
            }
            if c != wanted[matched] {
                break;
            }
            matched += 1;
            cut = i + c.len_utf8();
        }
        if matched == 0 {
            return text.to_string();
        }
        // 发走部分后面紧跟的标点也算发走了的那句的，别单独留在新输入框开头
        let rest = text[cut..].trim_start_matches(is_punct);
        rest.to_string()
    }

    /// 提前执行过指令的这句：去掉开头的指令词和跟在后面的标点（标点不参与对齐）
    fn without_early(&self, text: &str) -> String {
        let Some(word) = self.early.as_deref() else {
            return text.to_string();
        };
        let wanted: Vec<char> = word.chars().filter(|c| !is_punct(*c)).collect();
        let mut matched = 0;
        let mut cut = 0;
        for (i, c) in text.char_indices() {
            if matched == wanted.len() {
                break;
            }
            if is_punct(c) {
                continue;
            }
            if c != wanted[matched] {
                break;
            }
            matched += 1;
            cut = i + c.len_utf8();
        }
        text[cut..].trim_start_matches(is_punct).to_string()
    }

    /// 中间结果：退掉上次没说定的部分里变了的字，再补上新的。
    pub fn partial(&mut self, text: &str) -> Vec<Key> {
        self.partial_at(text, std::time::Instant::now())
    }

    /// 同上，带时间。只往后加字的立刻打；要退格改前面的字，离上次屏幕变化至少隔 `REWRITE_GAP`，
    /// 其间的修订先攒着（下一次或定稿时一起改），免得云端频繁修订时屏幕来回删写、一直闪。
    pub fn partial_at(&mut self, text: &str, now: std::time::Instant) -> Vec<Key> {
        let text = self.separated(&clean(text));
        if self.frozen.is_some() {
            return self.append_only(&text);
        }
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
        if self.frozen.is_some() {
            return self.finalize_frozen(output);
        }
        let erase = self.revision.pending_graphemes();
        self.revision.clear();
        // 这句结束了，发走的前缀、提前执行的指令都只对这句有效
        self.sent = None;
        let early = self.early.take().is_some();
        // 指令已经提前执行过、这句没剩别的字：什么都不打
        if early && matches!(&output, Output::Text(t) if t.trim().is_empty()) {
            return edit(erase, "");
        }
        let mut keys = edit(erase, "");
        keys.extend(match output {
            Output::Text(text) => self.typist.text(&clean(&text), self.terminal),
            Output::Command(command) => self.typist.command(command, self.terminal),
            Output::TextThen(text, command) => {
                let mut keys = self.typist.text(&clean(&text), self.terminal);
                keys.extend(self.typist.command(command, self.terminal));
                keys
            }
        });
        keys
    }

    /// 冻结状态下的定稿：不退格、不替换。文字只补还没上屏的尾巴；会删东西的指令不执行。
    fn finalize_frozen(&mut self, output: Output) -> Vec<Key> {
        let keys = match output {
            Output::Text(text) | Output::TextThen(text, _) => {
                let text = self.separated(&clean(&text));
                self.append_only(&text)
            }
            Output::Command(VoiceCommand::NewLine) => {
                self.typist.command(VoiceCommand::NewLine, self.terminal)
            }
            Output::Command(VoiceCommand::Send) => {
                self.typist.command(VoiceCommand::Send, self.terminal)
            }
            Output::Command(_) => vec![],
        };
        self.revision.clear();
        self.sent = None;
        self.frozen = None;
        self.early = None;
        keys
    }

    /// 终端里和上一句之间的分隔要在出中间结果时就带上，定稿时由 typist 统一处理。
    fn separated(&self, text: &str) -> String {
        if !self.terminal || text.is_empty() {
            return text.to_string();
        }
        let previous = self.typist.previous();
        let joined = xueer_ear_textflow::join_for_terminal(&[previous, text]);
        joined[previous.len()..].to_string()
    }
}

fn is_punct(c: char) -> bool {
    c.is_whitespace() || "，。！？；：、,.!?;:…—\"'“”‘’（）()".contains(c)
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
        assert_eq!(t.command(VoiceCommand::NewLine, true), [Key::CtrlJ]);
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
                Key::Enter | Key::CtrlJ => doc.push('\n'),
            }
        }
    }

    #[test]
    fn after_the_owner_touches_keys_nothing_is_erased_only_appended() {
        // 主人中途点了别处或自己打了字：雪耳不知道光标在哪，再退格就会删错地方
        let t0 = std::time::Instant::now();
        let ms = |n| t0 + std::time::Duration::from_millis(n);
        let mut live = Live::new(false);
        let mut doc = String::new();
        type_into(&mut doc, &live.partial_at("今天天器", ms(0)));
        live.user_touched();
        // 云端改了前面的字：不退格，只把多出来的字接上
        let keys = live.partial_at("今天天气很好", ms(400));
        assert!(keys.iter().all(|k| matches!(k, Key::Text(_))), "{keys:?}");
        type_into(&mut doc, &keys);
        assert_eq!(doc, "今天天器很好");
        // 定稿也不替换，只补还没上屏的尾巴
        let keys = live.finalize(Output::Text("今天天气很好。".into()));
        assert!(keys.iter().all(|k| matches!(k, Key::Text(_))), "{keys:?}");
        type_into(&mut doc, &keys);
        assert_eq!(doc, "今天天器很好。");
    }

    #[test]
    fn touching_keys_only_freezes_the_current_sentence() {
        let t0 = std::time::Instant::now();
        let ms = |n| t0 + std::time::Duration::from_millis(n);
        let mut live = Live::new(false);
        let mut doc = String::new();
        type_into(&mut doc, &live.partial_at("第一句", ms(0)));
        live.user_touched();
        type_into(&mut doc, &live.finalize(Output::Text("第一句。".into())));
        // 下一句恢复正常的边说边改
        type_into(&mut doc, &live.partial_at("第二剧", ms(1000)));
        type_into(&mut doc, &live.partial_at("第二句话", ms(1400)));
        assert_eq!(doc, "第一句。第二句话");
    }

    #[test]
    fn commands_after_touching_do_not_backspace() {
        // 动过键鼠后「删除」也不能退格：不知道光标在哪
        let mut live = Live::new(false);
        let mut doc = String::new();
        type_into(&mut doc, &live.finalize(Output::Text("上一句。".into())));
        type_into(&mut doc, &live.partial("删除"));
        live.user_touched();
        let keys = live.finalize(Output::Command(VoiceCommand::DeleteLastSentence));
        assert!(keys.is_empty(), "{keys:?}");
        // 「换行」这种不删东西的指令照常执行
        live.user_touched();
        assert_eq!(
            live.finalize(Output::Command(VoiceCommand::NewLine)),
            [Key::Enter]
        );
    }

    #[test]
    fn enter_mid_sentence_does_not_retype_what_was_sent() {
        let t0 = std::time::Instant::now();
        let ms = |n| t0 + std::time::Duration::from_millis(n);
        let mut live = Live::new(true);
        let mut doc = String::new();
        type_into(&mut doc, &live.partial_at("这句话非常好", ms(0)));
        assert_eq!(doc, "这句话非常好");
        // 主人按 Enter 发走了：输入框清空
        live.user_sent();
        doc.clear();
        // 云端随后送来同一句的定稿：不该再打一遍
        // 剩下的只有标点：不打，免得发完的输入框里留一个「。」
        let raw = live.unsent("这句话非常好。");
        assert_eq!(raw, "");
        type_into(&mut doc, &live.finalize(Output::Text(raw)));
        assert_eq!(doc, "");
    }

    #[test]
    fn speech_after_enter_continues_with_only_the_new_part() {
        let t0 = std::time::Instant::now();
        let ms = |n| t0 + std::time::Duration::from_millis(n);
        let mut live = Live::new(false);
        let mut doc = String::new();
        type_into(&mut doc, &live.partial_at("先发这句", ms(0)));
        live.user_sent();
        doc.clear();
        let raw = live.unsent("先发这句然后接着说");
        type_into(&mut doc, &live.partial_at(&raw, ms(300)));
        assert_eq!(doc, "然后接着说");
        let raw = live.unsent("先发这句，然后接着说。");
        type_into(&mut doc, &live.finalize(Output::Text(raw)));
        assert_eq!(doc, "然后接着说。");
        // 定稿之后前缀作废，下一句照常
        assert_eq!(live.unsent("下一句"), "下一句");
    }

    #[test]
    fn unsent_with_nothing_sent_is_identity() {
        let live = Live::new(false);
        assert_eq!(live.unsent("原样"), "原样");
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
    fn delete_last_sentence_said_at_the_end_removes_the_text_before_it() {
        let mut live = Live::new(false);
        let mut doc = String::from("上一句。");
        live.finalize(Output::Text(String::new()));
        type_into(&mut doc, &live.partial("这句是错的删掉上一句"));
        let keys = live.finalize(Output::TextThen(
            "这句是错的，".into(),
            VoiceCommand::DeleteLastSentence,
        ));
        type_into(&mut doc, &keys);
        assert_eq!(doc, "上一句。");
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
        // 撤销就是删掉上一句（主人 2026-11-15：两个应该是同一个功能），不发 Ctrl+Z
        let mut t = Typist::default();
        t.text("写错了", false);
        assert_eq!(
            t.command(VoiceCommand::DeleteLastSentence, false),
            [Key::Backspace(3)]
        );
    }

    #[test]
    fn delete_reaches_back_across_a_spoken_newline() {
        // 主人：「发了 12345，然后换行之后，就只能删除第二行的东西了」
        for terminal in [true, false] {
            let mut t = Typist::default();
            let mut doc = String::new();
            type_into(&mut doc, &t.text("12345。", terminal));
            type_into(&mut doc, &t.command(VoiceCommand::NewLine, terminal));
            type_into(&mut doc, &t.text("好。", terminal));
            assert_eq!(doc, "12345。\n好。");
            type_into(
                &mut doc,
                &t.command(VoiceCommand::DeleteLastSentence, terminal),
            );
            assert_eq!(doc, "12345。\n");
            // 再删一次：换行和上一行那句一起删掉
            type_into(
                &mut doc,
                &t.command(VoiceCommand::DeleteLastSentence, terminal),
            );
            assert_eq!(doc, "");
            assert_eq!(t.command(VoiceCommand::DeleteLastSentence, terminal), []);
        }
    }

    #[test]
    fn terminal_text_after_a_newline_starts_clean() {
        // 换行后的第一句前面不该补「，」
        let mut t = Typist::default();
        t.text("第一行", true);
        t.command(VoiceCommand::NewLine, true);
        assert_eq!(t.text("第二行", true), [Key::Text("第二行".into())]);
    }

    #[test]
    fn delete_all_removes_everything_typed_this_session() {
        let mut t = Typist::default();
        let mut doc = String::from("手打的");
        type_into(&mut doc, &t.text("第一句。", false));
        type_into(&mut doc, &t.command(VoiceCommand::NewLine, false));
        type_into(&mut doc, &t.text("第二句🙂", false));
        type_into(&mut doc, &t.command(VoiceCommand::DeleteAll, false));
        assert_eq!(doc, "手打的");
        assert_eq!(t.command(VoiceCommand::DeleteAll, false), []);
    }

    #[test]
    fn leftover_is_what_is_on_screen_but_not_yet_final() {
        // 出错时：屏幕上没说定的那截留着，交给调用方存进历史
        let mut live = Live::new(false);
        assert_eq!(live.leftover(), "");
        live.finalize(Output::Text("第一句。".into()));
        live.partial("第二句说到一");
        assert_eq!(live.leftover(), "第二句说到一");
        // 说定了就没有剩下的
        live.finalize(Output::Text("第二句说到一半。".into()));
        assert_eq!(live.leftover(), "");
    }

    #[test]
    fn a_spoken_command_runs_before_the_sentence_is_final() {
        // 主人 2026-11-16：「撤回和删除的速度…不是特别快，还有发送的速度」。
        // 说完「删除」停一下就执行，不等 3 秒静音收句；之后的定稿不再打出「删除」两个字
        let mut live = Live::new(false);
        let mut doc = String::new();
        type_into(&mut doc, &live.finalize(Output::Text("第一句。".into())));
        type_into(&mut doc, &live.partial("删除"));
        assert_eq!(doc, "第一句。删除");
        type_into(
            &mut doc,
            &live.early_command(VoiceCommand::DeleteLastSentence),
        );
        assert_eq!(doc, "");
        type_into(&mut doc, &live.partial(&live.unsent("删除。")));
        type_into(
            &mut doc,
            &live.finalize(Output::Text(live.unsent("删除。"))),
        );
        assert_eq!(doc, "");
        // 下一句照常
        type_into(&mut doc, &live.partial("新的一句"));
        type_into(&mut doc, &live.finalize(Output::Text("新的一句。".into())));
        assert_eq!(doc, "新的一句。");
    }

    #[test]
    fn words_after_an_early_command_are_still_typed() {
        let mut live = Live::new(false);
        let mut doc = String::new();
        type_into(&mut doc, &live.partial("换行"));
        type_into(&mut doc, &live.early_command(VoiceCommand::NewLine));
        assert_eq!(doc, "\n");
        let rest = live.unsent("换行，接着说");
        type_into(&mut doc, &live.partial(&rest));
        type_into(
            &mut doc,
            &live.finalize(Output::Text(live.unsent("换行，接着说。"))),
        );
        assert_eq!(doc, "\n接着说。");
    }

    #[test]
    fn early_delete_is_skipped_after_the_owner_touched_the_keyboard() {
        let mut live = Live::new(false);
        let mut doc = String::new();
        type_into(&mut doc, &live.finalize(Output::Text("第一句。".into())));
        type_into(&mut doc, &live.partial("删除"));
        live.user_touched();
        assert_eq!(live.early_command(VoiceCommand::DeleteLastSentence), []);
    }
}
