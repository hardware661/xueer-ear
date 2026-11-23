//! 对外入口：一句识别结果进来，按固定顺序处理，得到要上屏的文字或一条指令。

use crate::{
    CommandOptions, Hotwords, PunctOptions, Snippets, VoiceCommand, apply_punctuation,
    drop_false_full_stop, is_unfinished, normalize_numbers, parse_command, strip_fillers,
};

#[derive(Debug, Clone)]
pub struct Options {
    pub hotwords: Hotwords,
    pub snippets: Snippets,
    pub punct: PunctOptions,
    pub commands: CommandOptions,
    pub strip_fillers: bool,
    pub normalize_numbers: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            hotwords: Hotwords::default(),
            snippets: Snippets::default(),
            punct: PunctOptions::default(),
            commands: CommandOptions::default(),
            strip_fillers: true,
            normalize_numbers: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    Text(String),
    Command(VoiceCommand),
    /// 整句是快捷短语的口令：换成它的文字（里面的换行上屏时换成空格）
    Snippet(String),
}

/// 顺序：口癖 → 指令 → 快捷短语 → 热词 → 数字 → 标点。
/// 指令放在口癖之后（「嗯，换行」也算指令）、热词之前（热词不会把指令词改掉）；
/// 快捷短语排在指令后面（口令和指令撞了指令赢），整句是口令就不再整理；
/// 数字放在热词之后：热词按读音匹配，要先于把汉字改成阿拉伯数字。
/// 云端文字里的换行等控制字符先换成空格：雪耳从不往输入框里打换行（主人 2026-11-24 砍掉了「换行」）。
pub fn process(text: &str, options: &Options) -> Output {
    process_with(text, options, true)
}

/// 同 [`process`]，但整句是指令的说法也当普通文字（快捷键听写里，前面没停够的「删除」照样打出来）
pub fn process_text(text: &str, options: &Options) -> Output {
    process_with(text, options, false)
}

fn process_with(text: &str, options: &Options, commands: bool) -> Output {
    let text: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let text = if options.strip_fillers {
        strip_fillers(&text)
    } else {
        text
    };
    if commands && let Some(command) = parse_command(&text, &options.commands) {
        return Output::Command(command);
    }
    if let Some(snippet) = options.snippets.lookup(&text) {
        return Output::Snippet(snippet.to_string());
    }
    Output::Text(tidy(&text, options))
}

fn tidy(text: &str, options: &Options) -> String {
    let text = options.hotwords.apply(text);
    let text = if options.normalize_numbers {
        normalize_numbers(&text)
    } else {
        text
    };
    drop_false_full_stop(&apply_punctuation(&text, &options.punct))
}

/// 终端整段输入：把连续几句拼成一段。句子之间已有句末标点就直接接上，
/// 否则中文补「，」、英文补空格；句子里的换行换成空格。结果绝不含换行或回车。
pub fn join_for_terminal(parts: &[&str]) -> String {
    let mut out = String::new();
    for part in parts {
        let piece = terminal_append(&out, part);
        out.push_str(&piece);
    }
    out
}

/// 终端里接在 `previous`（已经上屏的文字）后面要打的那一段：分隔符 + 整理过的这一句。
/// 只看 `previous` 的最后一个字决定分隔符，不去改动已经上屏的部分。
pub fn terminal_append(previous: &str, part: &str) -> String {
    let part = part.replace(['\r', '\n'], " ");
    let part = part.trim();
    if part.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    if let Some(last) = previous.trim_end().chars().last() {
        let first = part.chars().next().unwrap_or(' ');
        if is_unfinished(previous) {
            // 上一句话没说完（「然后我的」），这句直接接上，不加逗号
        } else if "。！？；，、.!?;,".contains(last) {
            if last.is_ascii() {
                out.push(' ');
            }
        } else if last.is_ascii_alphanumeric() && first.is_ascii_alphanumeric() {
            out.push(' ');
        } else {
            out.push('，');
        }
    }
    out.push_str(part);
    out
}
