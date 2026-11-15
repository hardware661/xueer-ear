//! 对外入口：一句识别结果进来，按固定顺序处理，得到要上屏的文字或一条指令。

use crate::{
    CommandOptions, Hotwords, PunctOptions, VoiceCommand, apply_punctuation, normalize_numbers,
    parse_command, strip_fillers,
};

#[derive(Debug, Clone)]
pub struct Options {
    pub hotwords: Hotwords,
    pub punct: PunctOptions,
    pub commands: CommandOptions,
    pub strip_fillers: bool,
    pub normalize_numbers: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            hotwords: Hotwords::default(),
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
    /// 一句话末尾单独一小句是指令（「这句是错的，删掉上一句」）：先打文字，再执行指令
    TextThen(String, VoiceCommand),
}

/// 顺序：口癖 → 指令 → 热词 → 数字 → 标点。
/// 指令放在口癖之后（「嗯，换行」也算指令）、热词之前（热词不会把指令词改掉）；
/// 数字放在热词之后：热词按读音匹配，要先于把汉字改成阿拉伯数字。
pub fn process(text: &str, options: &Options) -> Output {
    let text = if options.strip_fillers {
        strip_fillers(text)
    } else {
        text.to_string()
    };
    if let Some(command) = parse_command(&text, &options.commands) {
        return Output::Command(command);
    }
    if let Some((head, command)) = split_trailing_command(&text, &options.commands) {
        return Output::TextThen(tidy(head, options), command);
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
    apply_punctuation(&text, &options.punct)
}

/// 最后一个逗号/句号之后那一小句如果整句是指令，就拆成（前面的文字, 指令）。
fn split_trailing_command<'a>(
    text: &'a str,
    options: &CommandOptions,
) -> Option<(&'a str, VoiceCommand)> {
    let body = text.trim_end_matches(['。', '！', '？', '!', '?', '.', '，', ',']);
    let cut = body.rfind(['，', ',', '。', '；', ';', '！', '？'])?;
    let separator = body[cut..].chars().next()?.len_utf8();
    let command = parse_command(&body[cut + separator..], options)?;
    Some((&text[..cut + separator], command))
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
        if "。！？；，、.!?;,".contains(last) {
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
