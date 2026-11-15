//! 语音指令：整句只说指令词（可带句末标点）才算指令，句中出现一律当普通文字。
//! 这里只负责认出指令；真正按键由调用方（Win+H / 快捷键两条输出路径）执行。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceCommand {
    NewLine,
    /// 删掉雪耳刚打的上一句。「删除」「撤销」「撤回」都是它（主人 2026-11-15：应该是同一个功能）
    DeleteLastSentence,
    /// 删掉这次听写里雪耳打的全部内容
    DeleteAll,
    /// 按回车。默认关闭，免得在终端里把半截话发给 AI。
    Send,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandOptions {
    pub allow_send: bool,
}

const WORDS: &[(&str, VoiceCommand)] = &[
    ("换行", VoiceCommand::NewLine),
    ("删除", VoiceCommand::DeleteLastSentence),
    ("删掉", VoiceCommand::DeleteLastSentence),
    ("删了", VoiceCommand::DeleteLastSentence),
    ("撤销", VoiceCommand::DeleteLastSentence),
    ("撤回", VoiceCommand::DeleteLastSentence),
    ("删掉上一句", VoiceCommand::DeleteLastSentence),
    ("删除上一句", VoiceCommand::DeleteLastSentence),
    ("删掉上一句话", VoiceCommand::DeleteLastSentence),
    ("删除上一句话", VoiceCommand::DeleteLastSentence),
    ("删掉上句话", VoiceCommand::DeleteLastSentence),
    ("删除上句话", VoiceCommand::DeleteLastSentence),
    ("删掉上一行", VoiceCommand::DeleteLastSentence),
    ("删除上一行", VoiceCommand::DeleteLastSentence),
    ("全部删除", VoiceCommand::DeleteAll),
    ("全部删掉", VoiceCommand::DeleteAll),
    ("全都删除", VoiceCommand::DeleteAll),
    ("全都删掉", VoiceCommand::DeleteAll),
    ("全删了", VoiceCommand::DeleteAll),
    ("清空", VoiceCommand::DeleteAll),
    ("发送", VoiceCommand::Send),
];

/// 能接在一句话末尾执行的指令（「第一行，换行」「今天吃什么，发送」「这句不对，删掉上一句」）。
/// 「删除」「清空」「撤销」这类短词在日常说话里太常见（「这些临时文件，全部删除」），
/// 只在整句单独说时才算（审查 R2，2026-11-16）。
pub fn parse_trailing_command(text: &str, options: &CommandOptions) -> Option<VoiceCommand> {
    let command = parse_command(text, options)?;
    let core = text
        .trim()
        .trim_end_matches(['。', '！', '？', '!', '?', '.', '，', ','])
        .trim();
    let explicit = core.contains("上一") || core.contains("上句");
    match command {
        VoiceCommand::NewLine | VoiceCommand::Send => Some(command),
        VoiceCommand::DeleteLastSentence if explicit => Some(command),
        _ => None,
    }
}

/// 整句是指令就返回它，否则返回 None（当普通文字上屏）。
pub fn parse_command(text: &str, options: &CommandOptions) -> Option<VoiceCommand> {
    let core = text
        .trim()
        .trim_end_matches(['。', '！', '？', '!', '?', '.', '，', ','])
        .trim();
    let command = WORDS.iter().find(|(w, _)| *w == core).map(|(_, c)| *c)?;
    (command != VoiceCommand::Send || options.allow_send).then_some(command)
}
