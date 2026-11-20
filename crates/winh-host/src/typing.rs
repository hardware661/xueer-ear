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
pub struct Typist {
    typed: Vec<Typed>,
    /// 终端里「换行」发不发 Ctrl+J（设置 text.terminal_newline）
    terminal_newline: bool,
}

impl Default for Typist {
    fn default() -> Self {
        Self {
            typed: Vec::new(),
            terminal_newline: true,
        }
    }
}

enum Typed {
    Sentence(String),
    /// 语音「换行」打出来的那个换行
    NewLine,
}

impl Typed {
    /// 删掉它要退格几次：按字形数（和中间结果改字的算法一致，审查 R11）。
    /// 编辑框里一次退格删一个字形，组合表情（👨‍👩‍👧）按 char 数会多删前面的字。
    fn width(&self) -> usize {
        use unicode_segmentation::UnicodeSegmentation;
        match self {
            Self::Sentence(text) => text.graphemes(true).count(),
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
            xueer_ear_textflow::terminal_append(self.previous(), text)
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
            VoiceCommand::NewLine if terminal && !self.terminal_newline => vec![],
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
            // 翻译要联网，按键在译文回来后由 `replace_all` 给出；这里什么都不按
            VoiceCommand::Translate
            | VoiceCommand::TranslateToEnglish
            | VoiceCommand::TranslateToChinese => vec![],
        }
    }

    /// 这次听写里雪耳打出去、还留在输入框里的全部文字（换行按 \n）
    pub fn round_text(&self) -> String {
        self.typed
            .iter()
            .map(|t| match t {
                Typed::Sentence(text) => text.as_str(),
                Typed::NewLine => "\n",
            })
            .collect()
    }

    /// 把这次听写打出去的全部文字换成 `text`（翻译结果）：退格删掉，再整段打上。
    /// 之后再说「撤回」删的就是这整段。
    pub fn replace_all(&mut self, text: &str, terminal: bool) -> Vec<Key> {
        let count: usize = self.typed.drain(..).map(|t| t.width()).sum();
        let text = if terminal {
            text.replace(['\r', '\n'], " ")
        } else {
            text.to_string()
        };
        let mut keys = backspaces(count);
        if !text.is_empty() {
            self.typed.push(Typed::Sentence(text.clone()));
            keys.push(Key::Text(text));
        }
        keys
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
    /// 主人按 Enter 时屏幕上那截还没说定的文字（已经被发走了），这句后续结果要去掉它
    sent: Option<String>,
    /// 主人在这句说到一半时动了键盘或鼠标：光标可能已经不在原处，这句剩下的时间只往后接字。
    /// 值是冻结时这句已经上屏的字数。
    frozen: Option<usize>,
    /// 这句里的语音指令已经提前执行了（值是指令词）：后续结果里去掉指令词和它前面的部分再上屏
    early: Option<String>,
    /// 这一轮里主人动过键盘或鼠标
    touched_round: bool,
    /// 上次说话中途退格改字的时间
    last_fix: Option<std::time::Instant>,
}

/// 说话中途一次最多退格改这么多字；更多的留到定稿
const LIVE_FIX_MAX: usize = 8;
/// 说话中途两次改字至少隔这么久
const LIVE_FIX_GAP: std::time::Duration = std::time::Duration::from_millis(350);

impl Live {
    pub fn new(terminal: bool) -> Self {
        Self {
            revision: Revision::default(),
            typist: Typist::default(),
            terminal,
            sent: None,
            frozen: None,
            early: None,
            touched_round: false,
            last_fix: None,
        }
    }

    /// 终端里「换行」发不发 Ctrl+J（审查 R4）
    pub fn set_terminal_newline(&mut self, on: bool) {
        self.typist.terminal_newline = on;
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
        let spoken = last_clause(self.revision.pending());
        let erase = if self.frozen.is_some() {
            0
        } else {
            self.revision.pending_graphemes()
        };
        self.revision.clear();
        self.frozen = self.frozen.map(|_| 0);
        self.early = Some(spoken);
        let mut keys = edit(erase, "");
        keys.extend(self.typist.command(command, self.terminal));
        keys
    }

    /// 句尾带指令（「今天吃什么，发送」）、停了一下没再说：不等静音收句，现在就把前面的话定稿上屏，
    /// 再执行指令。`spoken` 是此刻的中间结果，用来记住指令词，之后这句的结果去掉指令词及以前的部分。
    pub fn early_finish(&mut self, output: Output, spoken: &str) -> Vec<Key> {
        let Output::TextThen(head, command) = output else {
            return vec![];
        };
        if self.early.is_some() {
            return vec![];
        }
        let deletes = matches!(
            command,
            VoiceCommand::DeleteLastSentence | VoiceCommand::DeleteAll
        );
        if self.frozen.is_some() && deletes {
            return vec![];
        }
        let mut keys = self.finalize(Output::Text(head));
        keys.extend(self.typist.command(command, self.terminal));
        self.early = Some(last_clause(spoken));
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

    /// 这一轮雪耳打出去、还留在输入框里的全部文字（说「翻译」时整段拿去翻）。
    /// 主人这轮动过键鼠时是 None：光标可能已经不在原处，不能按退格整段替换。
    pub fn round_text(&self) -> Option<String> {
        (!self.touched_round).then(|| self.typist.round_text())
    }

    /// 把这一轮打出去的字整段换成译文
    pub fn replace_round(&mut self, text: &str) -> Vec<Key> {
        self.typist.replace_all(text, self.terminal)
    }

    /// 主人自己动了键盘或鼠标（点了别处、打了字、选了文字）。雪耳改字靠「退格再重打」，
    /// 不知道光标在哪时再退格就会删错地方，所以这句剩下的时间只往后接字，之前打的也不再往回删。
    pub fn user_touched(&mut self) {
        self.touched_round = true;
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
    /// 云端定稿常在发走的文字后面补标点、改一两个字（「天器」→「天气」），所以按「去掉标点后的字数」跳过，
    /// 不要求逐字相同（审查 R6）：按前缀对齐时，前面改了一个字就会把后半句甚至整句重打一遍。
    pub fn unsent(&self, text: &str) -> String {
        let text = &self.without_early(text);
        let Some(sent) = self.sent.as_deref() else {
            return text.to_string();
        };
        let wanted = sent.chars().filter(|c| !is_punct(*c)).count();
        let mut counted = 0;
        let mut cut = 0;
        for (i, c) in text.char_indices() {
            if counted == wanted {
                break;
            }
            if !is_punct(c) {
                counted += 1;
            }
            cut = i + c.len_utf8();
        }
        // 发走部分后面紧跟的标点也算发走了的那句的，别单独留在新输入框开头
        text[cut..].trim_start_matches(is_punct).to_string()
    }

    /// 提前执行过指令的这句：去掉指令词和它前面的部分（前面的已经上屏或发走了），只留后面接着说的。
    /// 按「去掉标点后」找指令词最后一次出现的位置：云端常改前面几个字、换标点，
    /// 也会把后面的话直接粘在指令词后面（「换行接着说」）。找不到就整句照打，宁可多打也不丢字（审查 R3）。
    fn without_early(&self, text: &str) -> String {
        let Some(word) = self.early.as_deref() else {
            return text.to_string();
        };
        let word: Vec<char> = word.chars().filter(|c| !is_punct(*c)).collect();
        if word.is_empty() {
            return text.to_string();
        }
        // 去掉标点后的字，以及每个字在原文里结束的字节位置
        let letters: Vec<(char, usize)> = text
            .char_indices()
            .filter(|(_, c)| !is_punct(*c))
            .map(|(i, c)| (c, i + c.len_utf8()))
            .collect();
        let found = (0..=letters.len().saturating_sub(word.len()))
            .rev()
            .find(|&start| {
                letters.len() >= word.len()
                    && letters[start..start + word.len()]
                        .iter()
                        .map(|(c, _)| *c)
                        .eq(word.iter().copied())
            });
        match found {
            Some(start) => {
                let end = letters[start + word.len() - 1].1;
                text[end..].trim_start_matches(is_punct).to_string()
            }
            None => text.to_string(),
        }
    }

    /// 中间结果：退掉上次没说定的部分里变了的字，再补上新的。
    pub fn partial(&mut self, text: &str) -> Vec<Key> {
        self.partial_at(text, std::time::Instant::now())
    }

    /// 同上，带时间。往后接字随时打；要退格改前面的字时，只有改动不大（`LIVE_FIX_MAX` 个字以内）、
    /// 且离上次改字至少 `LIVE_FIX_GAP` 才当场改，否则先留着，下一次或定稿时一起改。
    /// 2026-11-19：中途反复大段「退格再重打」会漏字；全部留到定稿又太慢、错字在屏幕上留太久。
    pub fn partial_at(&mut self, text: &str, now: std::time::Instant) -> Vec<Key> {
        use unicode_segmentation::UnicodeSegmentation;
        let text = self.separated(&clean(text));
        if self.frozen.is_some() {
            return self.append_only(&text);
        }
        let Ok(update) = self.revision.prepare(&text, false) else {
            return vec![];
        };
        let may_fix = update.erase <= LIVE_FIX_MAX
            && self
                .last_fix
                .is_none_or(|at| now.duration_since(at) >= LIVE_FIX_GAP);
        if update.erase == 0 || may_fix {
            if update.erase > 0 {
                self.last_fix = Some(now);
            }
            let keys = edit(update.erase, &update.insert);
            self.revision.accept(update);
            return keys;
        }
        // 改动太大或刚改过：只把多出来的字接上
        let shown = self.revision.pending_graphemes();
        let graphemes: Vec<&str> = text.graphemes(true).collect();
        if graphemes.len() <= shown {
            return vec![];
        }
        let extra: String = graphemes[shown..].concat();
        self.revision.append(&extra);
        vec![Key::Text(extra)]
    }

    /// 定稿：把屏幕上没说定的那一截整个换成整理后的文字，或者擦掉后执行指令。
    pub fn finalize(&mut self, output: Output) -> Vec<Key> {
        let raw = match &output {
            Output::Text(text) | Output::TextThen(text, _) => text.clone(),
            Output::Command(_) => String::new(),
        };
        self.finalize_from(&raw, output)
    }

    /// 同上，带上云端给的原话 `raw`（整理前）。主人中途动过键盘鼠标时，屏幕上是原话，
    /// 只能按原话的字数往后接，不能拿整理后的文字对齐（「百分之五十」整理成「50%」后字数变了，审查 R5）。
    pub fn finalize_from(&mut self, raw: &str, output: Output) -> Vec<Key> {
        if self.frozen.is_some() {
            return self.finalize_frozen(raw, output);
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
        let shown = self.revision.pending().to_string();
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
        keep_common_prefix(&shown, keys)
    }

    /// 冻结状态下的定稿：不退格、不替换，按原话补上还没上屏的尾巴；会删东西的指令不执行。
    fn finalize_frozen(&mut self, raw: &str, output: Output) -> Vec<Key> {
        let keys = match output {
            Output::Text(_) => {
                let text = self.separated(&clean(raw));
                self.append_only(&text)
            }
            Output::TextThen(_, command) => {
                // 原话去掉句尾那一小句指令
                let body = raw.trim_end_matches(is_punct);
                let cut = body.rfind(is_punct).map_or(0, |i| {
                    i + body[i..].chars().next().map_or(0, char::len_utf8)
                });
                let text = self.separated(&clean(&raw[..cut]));
                let mut keys = self.append_only(&text);
                if matches!(command, VoiceCommand::NewLine | VoiceCommand::Send) {
                    keys.extend(self.typist.command(command, self.terminal));
                }
                keys
            }
            Output::Command(command @ (VoiceCommand::NewLine | VoiceCommand::Send)) => {
                self.typist.command(command, self.terminal)
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
        xueer_ear_textflow::terminal_append(self.typist.previous(), text)
    }
}

/// 定稿时屏幕上已经是 `shown`：开头一样的字不用退格重打，只改从第一个不同的字开始的部分
fn keep_common_prefix(shown: &str, mut keys: Vec<Key>) -> Vec<Key> {
    use unicode_segmentation::UnicodeSegmentation;
    let [Key::Backspace(erase), Key::Text(text), ..] = keys.as_mut_slice() else {
        return keys;
    };
    let common = shown
        .graphemes(true)
        .zip(text.graphemes(true))
        .take_while(|(a, b)| a == b)
        .count();
    if common == 0 {
        return keys;
    }
    let cut: usize = text.graphemes(true).take(common).map(str::len).sum();
    *erase -= common;
    *text = text[cut..].to_string();
    keys.retain(|k| !matches!(k, Key::Backspace(0)) && !matches!(k, Key::Text(t) if t.is_empty()));
    keys
}

/// 最后一小句（去掉标点和空格），用来记住说出来的指令词
fn last_clause(text: &str) -> String {
    let body = text.trim_end_matches(is_punct);
    let start = body.rfind(is_punct).map_or(0, |i| {
        i + body[i..].chars().next().map_or(0, char::len_utf8)
    });
    body[start..].chars().filter(|c| !is_punct(*c)).collect()
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
    fn terminal_joins_after_ascii_punctuation_without_crashing() {
        // 审查 R1：上一句以英文标点结尾，再说两句，按上一句长度切拼接结果切到汉字中间，进程崩溃
        let mut t = Typist::default();
        let mut doc = String::new();
        for sentence in ["OK.", "好的。", "继续"] {
            type_into(&mut doc, &t.text(sentence, true));
        }
        assert_eq!(doc, "OK. 好的。继续");
        // 英文接英文：中间的空格不能被吃掉
        let mut t = Typist::default();
        let mut doc = String::new();
        for sentence in ["Run tests.", "ok", "done"] {
            type_into(&mut doc, &t.text(sentence, true));
        }
        assert_eq!(doc, "Run tests. ok done");
        // 删上一句只删这一句自己打出去的字（含它前面补的分隔）
        type_into(&mut doc, &t.command(VoiceCommand::DeleteLastSentence, true));
        assert_eq!(doc, "Run tests. ok");
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
    fn translate_replaces_everything_typed_this_round() {
        let mut t = Typist::default();
        let mut doc = String::from("原来就有的");
        type_into(&mut doc, &t.text("今天吃什么。", false));
        type_into(&mut doc, &t.command(VoiceCommand::NewLine, false));
        type_into(&mut doc, &t.text("明天呢？", false));
        assert_eq!(t.command(VoiceCommand::Translate, false), []);
        assert_eq!(t.round_text(), "今天吃什么。\n明天呢？");
        type_into(
            &mut doc,
            &t.replace_all("What to eat today?\nAnd tomorrow?", false),
        );
        assert_eq!(doc, "原来就有的What to eat today?\nAnd tomorrow?");
        // 再撤回，删的是整段译文，不碰原来就有的字
        type_into(
            &mut doc,
            &t.command(VoiceCommand::DeleteLastSentence, false),
        );
        assert_eq!(doc, "原来就有的");
    }

    #[test]
    fn saying_translate_at_the_end_gives_the_whole_round_for_translation() {
        let mut live = Live::new(false);
        let mut doc = String::from("原有");
        type_into(&mut doc, &live.partial("今天吃什么"));
        type_into(
            &mut doc,
            &live.finalize(Output::Text("今天吃什么？".into())),
        );
        type_into(&mut doc, &live.partial("明天呢翻译"));
        type_into(
            &mut doc,
            &live.finalize(Output::TextThen("明天呢，".into(), VoiceCommand::Translate)),
        );
        assert_eq!(live.round_text().as_deref(), Some("今天吃什么？明天呢，"));
        type_into(
            &mut doc,
            &live.replace_round("What to eat today? And tomorrow?"),
        );
        assert_eq!(doc, "原有What to eat today? And tomorrow?");
    }

    #[test]
    fn no_replacement_after_the_owner_touched_the_keyboard() {
        let mut live = Live::new(false);
        live.finalize(Output::Text("第一句。".into()));
        live.user_touched();
        assert_eq!(live.round_text(), None);
    }

    #[test]
    fn translation_never_puts_a_newline_into_a_terminal() {
        let mut t = Typist::default();
        t.text("第一句", true);
        assert_eq!(
            t.replace_all("one\ntwo", true),
            [Key::Backspace(3), Key::Text("one two".into())]
        );
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
        type_into(&mut doc, &live.finalize(Output::Text("第二句话。".into())));
        assert_eq!(doc, "第一句。第二句话。");
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
    fn small_corrections_show_while_speaking_big_ones_wait_for_the_end() {
        // 主人 2026-11-19：只在定稿时改字太慢，屏幕上错字留太久。小改动（几个字）说话中就改，
        // 但两次改字至少隔一会儿；大段改写还是等定稿，免得一次塞太多键又漏字。
        let t0 = std::time::Instant::now();
        let ms = |n| t0 + std::time::Duration::from_millis(n);
        let mut live = Live::new(false);
        let mut doc = String::new();
        type_into(&mut doc, &live.partial_at("今天天器", ms(0)));
        type_into(&mut doc, &live.partial_at("今天天气很好", ms(400)));
        assert_eq!(doc, "今天天气很好");
        // 刚改过，马上又要改：先不动
        assert!(
            live.partial_at("今天天气狠好呀", ms(450))
                .iter()
                .all(|k| matches!(k, Key::Text(_)))
        );
        let long_before: String = "一".repeat(20);
        let mut live = Live::new(false);
        let mut doc = String::new();
        type_into(&mut doc, &live.partial_at(&long_before, ms(0)));
        // 前面 20 个字全变：说话中不改，定稿时一次改
        let keys = live.partial_at(&"二".repeat(21), ms(800));
        assert!(keys.iter().all(|k| matches!(k, Key::Text(_))), "{keys:?}");
    }

    #[test]
    fn rapid_revisions_only_append_and_fix_once_at_the_end() {
        // 2026-11-19 主人：漏字发生在说话中途反复「退格再重打」时（「功能」只剩「功」），终端、微信、网页都有。
        // 云端连着改（间隔很短）时只往后接字，前面的错字等这句定稿时一次改好。
        let t0 = std::time::Instant::now();
        let ms = |n| t0 + std::time::Duration::from_millis(n);
        let mut live = Live::new(false);
        let mut doc = String::from("原有");
        let mut all = Vec::new();
        for (text, at) in [
            ("今天", 0),
            ("今天天器", 300),
            ("今天天气很", 600),
            ("今天田气很好", 900),
        ] {
            let keys = live.partial_at(text, ms(at));
            all.extend(keys.clone());
            type_into(&mut doc, &keys);
        }
        // 300 ms 里云端改了两次：只当场改第一次，第二次留到定稿
        assert_eq!(
            all.iter()
                .filter(|k| matches!(k, Key::Backspace(_)))
                .count(),
            1,
            "{all:?}"
        );
        assert_eq!(doc, "原有今天天气很好");
        let keys = live.finalize(Output::Text("今天天气很好。".into()));
        assert_eq!(
            keys.iter()
                .filter(|k| matches!(k, Key::Backspace(_)))
                .count(),
            1
        );
        type_into(&mut doc, &keys);
        assert_eq!(doc, "原有今天天气很好。");
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

    #[test]
    fn a_command_at_the_end_of_a_sentence_runs_early_too() {
        // 主人 2026-11-16：「发送的速度还是特别慢」——说完一句接着说「发送」，云端把它并进同一句，
        // 原来要等 3 秒静音收句才执行
        let mut live = Live::new(true);
        let mut doc = String::new();
        type_into(&mut doc, &live.partial("今天吃什么，发送"));
        type_into(
            &mut doc,
            &live.early_finish(
                Output::TextThen("今天吃什么，".into(), VoiceCommand::Send),
                "今天吃什么，发送",
            ),
        );
        assert_eq!(doc, "今天吃什么，\n");
        // 定稿到了，云端顺手改了前面几个字、加了标点：已经发走了，什么都不再打
        for text in ["今天吃点什么，发送", "今天吃点什么？发送。"] {
            let rest = live.unsent(text);
            type_into(&mut doc, &live.partial(&rest));
        }
        let rest = live.unsent("今天吃点什么？发送。");
        type_into(&mut doc, &live.finalize(Output::Text(rest)));
        assert_eq!(doc, "今天吃什么，\n", "不能再打一遍或再按一次回车");
        // 下一句照常
        type_into(&mut doc, &live.partial("下一句"));
        type_into(&mut doc, &live.finalize(Output::Text("下一句。".into())));
        assert_eq!(doc, "今天吃什么，\n下一句。");
    }

    #[test]
    fn words_after_an_early_trailing_command_are_kept() {
        let mut live = Live::new(false);
        let mut doc = String::new();
        type_into(&mut doc, &live.partial("第一行，换行"));
        type_into(
            &mut doc,
            &live.early_finish(
                Output::TextThen("第一行，".into(), VoiceCommand::NewLine),
                "第一行，换行",
            ),
        );
        let rest = live.unsent("第一行，换行，第二行");
        type_into(&mut doc, &live.partial(&rest));
        let rest = live.unsent("第一行。换行。第二行。");
        type_into(&mut doc, &live.finalize(Output::Text(rest)));
        assert_eq!(doc, "第一行，\n第二行。");
    }

    #[test]
    fn speech_glued_to_an_early_command_without_punctuation_is_kept() {
        // 审查 R3：说「换行」停一下已经执行，再说「接着说」，云端合成「换行接着说」（没逗号），后半句全丢
        let mut live = Live::new(false);
        let mut doc = String::new();
        type_into(&mut doc, &live.partial("换行"));
        type_into(&mut doc, &live.early_command(VoiceCommand::NewLine));
        for text in ["换行接", "换行接着说"] {
            let rest = live.unsent(text);
            type_into(&mut doc, &live.partial(&rest));
        }
        let rest = live.unsent("换行接着说。");
        type_into(&mut doc, &live.finalize(Output::Text(rest)));
        assert_eq!(doc, "\n接着说。");
    }

    #[test]
    fn speech_glued_to_an_early_trailing_command_is_kept() {
        let mut live = Live::new(true);
        let mut doc = String::new();
        type_into(&mut doc, &live.partial("先跑测试，发送"));
        type_into(
            &mut doc,
            &live.early_finish(
                Output::TextThen("先跑测试，".into(), VoiceCommand::Send),
                "先跑测试，发送",
            ),
        );
        assert_eq!(doc, "先跑测试，\n");
        let rest = live.unsent("先跑测试发送以后再提交");
        type_into(&mut doc, &live.partial(&rest));
        let rest = live.unsent("先跑测试，发送以后再提交。");
        type_into(&mut doc, &live.finalize(Output::Text(rest)));
        assert_eq!(doc, "先跑测试，\n以后再提交。");
    }

    #[test]
    fn frozen_sentence_keeps_words_when_the_final_text_was_tidied() {
        // 审查 R5：屏幕上是原话「我有百分之五十的」，中途点了鼠标；定稿整理成「我有50%的把握。」，
        // 按字数对齐只补了「。」，「把握」丢了
        let mut live = Live::new(false);
        let mut doc = String::new();
        type_into(&mut doc, &live.partial("我有百分之五十的"));
        live.user_touched();
        type_into(
            &mut doc,
            &live.finalize_from(
                "我有百分之五十的把握。",
                Output::Text("我有50%的把握。".into()),
            ),
        );
        assert_eq!(doc, "我有百分之五十的把握。");
    }

    #[test]
    fn frozen_sentence_still_runs_a_trailing_newline() {
        let mut live = Live::new(false);
        let mut doc = String::new();
        type_into(&mut doc, &live.partial("第一行"));
        live.user_touched();
        type_into(
            &mut doc,
            &live.finalize_from(
                "第一行，换行",
                Output::TextThen("第一行，".into(), VoiceCommand::NewLine),
            ),
        );
        assert_eq!(doc, "第一行，\n");
    }

    #[test]
    fn enter_mid_sentence_skips_the_sent_part_even_if_the_cloud_respelled_it() {
        // 审查 R6：屏幕上「今天天器很好」被 Enter 发走；定稿「今天天气很好。」→ 新输入框里出现「气很好。」
        for shown in ["今天天器很好", "金天天气很好"] {
            let mut live = Live::new(false);
            let mut doc = String::new();
            type_into(&mut doc, &live.partial(shown));
            live.user_sent();
            doc.clear(); // 输入框被 Enter 清空
            let rest = live.unsent("今天天气很好。");
            type_into(&mut doc, &live.finalize(Output::Text(rest)));
            assert_eq!(doc, "", "{shown}：发走的那句不该再打出来");
        }
        // 发走后接着说的照常打
        let mut live = Live::new(false);
        let mut doc = String::new();
        type_into(&mut doc, &live.partial("今天天器很好"));
        live.user_sent();
        doc.clear();
        let rest = live.unsent("今天天气很好，明天下雨");
        type_into(&mut doc, &live.partial(&rest));
        let rest = live.unsent("今天天气很好，明天下雨。");
        type_into(&mut doc, &live.finalize(Output::Text(rest)));
        assert_eq!(doc, "明天下雨。");
    }

    #[test]
    fn spoken_newline_can_be_switched_off_in_terminals() {
        // 审查 R4：终端里「换行」发 Ctrl+J；Claude Code 里是换行，普通 bash 里就是回车，会执行命令
        let mut live = Live::new(true);
        live.set_terminal_newline(false);
        let mut doc = String::new();
        type_into(&mut doc, &live.partial("ls"));
        type_into(&mut doc, &live.finalize(Output::Text("ls".into())));
        type_into(
            &mut doc,
            &live.finalize(Output::Command(VoiceCommand::NewLine)),
        );
        assert_eq!(doc, "ls", "关掉后终端里不发 Ctrl+J");
        // 不是终端照常换行
        let mut live = Live::new(false);
        live.set_terminal_newline(false);
        assert_eq!(
            live.finalize(Output::Command(VoiceCommand::NewLine)),
            [Key::Enter]
        );
        // 默认开着
        let mut live = Live::new(true);
        assert_eq!(
            live.finalize(Output::Command(VoiceCommand::NewLine)),
            [Key::CtrlJ]
        );
    }

    #[test]
    fn deleting_a_sentence_counts_emoji_as_one_character_like_revisions_do() {
        // 审查 R11：Typist 按 char 计退格（👨‍👩‍👧 算 5 个），中间结果的改字按字形计（算 1 个），两边不一致
        let mut t = Typist::default();
        t.text("👨‍👩‍👧好", false);
        assert_eq!(
            t.command(VoiceCommand::DeleteLastSentence, false),
            [Key::Backspace(2)]
        );
    }
}
