//! 快捷键模式的上屏计划（纯逻辑，可在 WSL 测）：把识别出的文字变成按键序列。
//! 语音指令不在这里：指令管的是整个输入框（`inputbox`），由 `hotkey.rs` 读框、算好按键再发。
//! 真正发送键盘事件在 `hotkey.rs`（只在 Windows 编）。
use crate::revision::Revision;
use xueer_ear_textflow::Output;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    /// 一段文字，用 Unicode 键入
    Text(String),
    /// 回车（只有「发送」指令按）
    Enter,
    /// 退格若干次
    Backspace(usize),
    /// Ctrl+A 全选
    SelectAll,
    /// Ctrl+C 复制（读不到输入框的程序，全选复制来读；终端里从不按）
    Copy,
    /// Ctrl+End：光标挪到输入框最后（普通程序；终端里不按）
    End,
    /// 往右挪光标若干个字（终端的输入框里挪到最后）
    Right(usize),
}

/// 记着刚打出去的上一句：终端里下一句要按它补分隔（「，」或空格）
#[derive(Default)]
pub struct Typist {
    previous: String,
}

impl Typist {
    /// 主人自己按 Enter 把内容发走了，或者输入框被指令改过：上一句不再算数
    pub fn forget(&mut self) {
        self.previous.clear();
    }

    /// 上屏一句定稿文字。终端里：换行换成空格，和上一句之间用 `terminal_append` 的规则补分隔。
    pub fn text(&mut self, text: &str, terminal: bool) -> Vec<Key> {
        let typed = if terminal {
            xueer_ear_textflow::terminal_append(&self.previous, text)
        } else {
            text.to_string()
        };
        if typed.is_empty() {
            return vec![];
        }
        self.previous = typed.clone();
        vec![Key::Text(typed)]
    }

    /// 上屏一段快捷短语：文字原样打，里面的换行换成空格。雪耳从不按回车换行，
    /// 微信这类聊天框里回车就是发送（主人 2026-11-24 砍掉了「换行」）
    pub fn snippet(&mut self, text: &str, terminal: bool) -> Vec<Key> {
        let flat = text
            .split('\n')
            .map(|line| clean(line).trim().to_string())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        self.text(&flat, terminal)
    }
}

/// 面板听写：边说边出字。中间结果直接打到光标处，后来的结果只改还没说定的那一截；
/// 定稿时换成整理过的文字。
pub struct Live {
    revision: Revision,
    typist: Typist,
    terminal: bool,
    /// 主人按 Enter 时屏幕上那截还没说定的文字（已经被发走了），这句后续结果要去掉它
    sent: Option<String>,
    /// 主人在这句说到一半时动了键盘或鼠标：光标可能已经不在原处，这句剩下的时间只往后接字。
    /// 值是冻结时这句已经上屏的字数。
    frozen: Option<usize>,
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
            last_fix: None,
        }
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

    /// 语音指令改过输入框（删了、清空了、换成译文、发出去了）：之后的字接在 `now` 后面
    pub fn box_changed(&mut self, now: &str) {
        self.revision.clear();
        self.typist.previous = now.to_string();
    }

    /// 屏幕上还没说定的那一截。听写出错时这截字留在屏幕上，调用方把它存进历史。
    pub fn leftover(&self) -> &str {
        self.revision.pending()
    }

    /// 主人自己动了键盘或鼠标（点了别处、打了字、选了文字）。雪耳改字靠「退格再重打」，
    /// 不知道光标在哪时再退格就会删错地方，所以这句剩下的时间只往后接字。
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
    /// 云端定稿常在发走的文字后面补标点、改一两个字（「天器」→「天气」），所以按「去掉标点后的字数」跳过，
    /// 不要求逐字相同（审查 R6）：按前缀对齐时，前面改了一个字就会把后半句甚至整句重打一遍。
    pub fn unsent(&self, text: &str) -> String {
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

    /// 定稿：把屏幕上没说定的那一截整个换成整理后的文字。
    /// 指令不经过这里（`hotkey.rs` 先认出来，屏幕上也没打过它），万一来了就只擦掉没说定的那截
    pub fn finalize(&mut self, output: Output) -> Vec<Key> {
        let raw = match &output {
            Output::Text(text) | Output::Snippet(text) => text.clone(),
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
        let shown = self.revision.pending().to_string();
        self.revision.clear();
        // 这句结束了，发走的前缀只对这句有效
        self.sent = None;
        let mut keys = edit(erase, "");
        keys.extend(match output {
            Output::Text(text) => self.typist.text(&clean(&text), self.terminal),
            Output::Snippet(text) => self.typist.snippet(&text, self.terminal),
            Output::Command(_) => vec![],
        });
        keep_common_prefix(&shown, keys)
    }

    /// 冻结状态下的定稿：不退格、不替换，按原话补上还没上屏的尾巴。
    /// 快捷短语也不换：口令已经在屏幕上了，换不掉。
    fn finalize_frozen(&mut self, raw: &str, output: Output) -> Vec<Key> {
        let keys = match output {
            Output::Text(_) | Output::Snippet(_) => {
                let text = self.separated(&clean(raw));
                self.append_only(&text)
            }
            Output::Command(_) => vec![],
        };
        self.revision.clear();
        self.sent = None;
        self.frozen = None;
        keys
    }

    /// 终端里和上一句之间的分隔要在出中间结果时就带上，定稿时由 typist 统一处理。
    fn separated(&self, text: &str) -> String {
        if !self.terminal || text.is_empty() {
            return text.to_string();
        }
        xueer_ear_textflow::terminal_append(&self.typist.previous, text)
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

fn is_punct(c: char) -> bool {
    c.is_whitespace() || "，。！？；：、,.!?;:…—\"'“”‘’（）()".contains(c)
}

/// 没有输入框时按键不发出去，打在这份文字上（说完复制到剪贴板）：和真打字一样按字形退格
pub fn apply_keys(doc: &mut String, keys: &[Key]) {
    use unicode_segmentation::UnicodeSegmentation;
    for key in keys {
        match key {
            Key::Text(text) => doc.push_str(text),
            Key::Enter => doc.push('\n'),
            Key::Backspace(count) => {
                let keep = doc.graphemes(true).count().saturating_sub(*count);
                let cut: usize = doc.graphemes(true).take(keep).map(str::len).sum();
                doc.truncate(cut);
            }
            // 攒着的字只有这一份，全选后接着打的字就是替换它
            Key::SelectAll => doc.clear(),
            // 光标一直在最后；复制不改文字
            Key::End | Key::Right(_) | Key::Copy => {}
        }
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

    fn type_into(doc: &mut String, keys: &[Key]) {
        apply_keys(doc, keys);
    }

    #[test]
    fn text_is_typed() {
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
    }

    #[test]
    fn newlines_in_text_never_reach_a_terminal() {
        let mut t = Typist::default();
        assert_eq!(t.text("一行\n两行", true), [Key::Text("一行 两行".into())]);
    }

    // 主人 2026-11-24 砍掉了「换行」：快捷短语里的换行也换成空格，任何程序里都不按回车
    #[test]
    fn snippet_line_breaks_become_spaces_everywhere() {
        for terminal in [false, true] {
            let mut live = Live::new(terminal);
            let mut doc = String::new();
            type_into(&mut doc, &live.partial("我的签名"));
            let keys = live.finalize(Output::Snippet("雪儿\n敬上\n".into()));
            assert!(!keys.contains(&Key::Enter), "{keys:?}");
            type_into(&mut doc, &keys);
            assert_eq!(doc, "雪儿 敬上");
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
        // 云端随后送来同一句的定稿：不该再打一遍；剩下的只有标点也不打
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
    fn enter_mid_sentence_skips_the_sent_part_even_if_the_cloud_respelled_it() {
        // 审查 R6：屏幕上「今天天器很好」被 Enter 发走；定稿「今天天气很好。」→ 新输入框里出现「气很好。」
        for shown in ["今天天器很好", "金天天气很好"] {
            let mut live = Live::new(false);
            let mut doc = String::new();
            type_into(&mut doc, &live.partial(shown));
            live.user_sent();
            doc.clear();
            let rest = live.unsent("今天天气很好。");
            type_into(&mut doc, &live.finalize(Output::Text(rest)));
            assert_eq!(doc, "", "{shown}：发走的那句不该再打出来");
        }
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
        let mut live = Live::new(false);
        let mut doc = String::new();
        type_into(&mut doc, &live.partial_at(&"一".repeat(20), ms(0)));
        // 前面 20 个字全变：说话中不改，定稿时一次改
        let keys = live.partial_at(&"二".repeat(21), ms(800));
        assert!(keys.iter().all(|k| matches!(k, Key::Text(_))), "{keys:?}");
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

    // 指令改过输入框后（比如删掉了最后一句），终端里下一句按新的结尾补分隔
    #[test]
    fn after_a_command_changed_the_box_the_next_sentence_joins_its_new_end() {
        let mut live = Live::new(true);
        let mut doc = String::from("先读文档。");
        live.box_changed("先读文档。");
        type_into(&mut doc, &live.finalize(Output::Text("再跑测试".into())));
        assert_eq!(doc, "先读文档。再跑测试");
        let mut live = Live::new(true);
        live.box_changed("");
        assert_eq!(
            live.finalize(Output::Text("开头".into())),
            [Key::Text("开头".into())]
        );
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
    fn a_frozen_sentence_keeps_the_spoken_words_instead_of_a_snippet() {
        // 主人中途动了键盘：屏幕上的口令不能退格换掉，只按原话补完
        let mut live = Live::new(false);
        let mut doc = String::new();
        type_into(&mut doc, &live.partial("我的"));
        live.user_touched();
        let keys = live.finalize_from("我的邮箱", Output::Snippet("a@b.c".into()));
        type_into(&mut doc, &keys);
        assert_eq!(doc, "我的邮箱");
    }

    #[test]
    fn keys_can_be_applied_to_text_kept_off_screen() {
        let mut doc = String::new();
        apply_keys(
            &mut doc,
            &[
                Key::Text("第一句👨‍👩‍👧".into()),
                Key::Backspace(1),
                Key::End,
                Key::Text("第二句".into()),
                Key::Right(3),
            ],
        );
        assert_eq!(doc, "第一句第二句");
        apply_keys(&mut doc, &[Key::Backspace(99)]);
        assert_eq!(doc, "");
    }
}
