//! 语音指令：整句只说指令词（可带句末标点）才算指令，句中出现一律当普通文字。
//! 这里只负责认出指令；真正按键由调用方（Win+H / 快捷键两条输出路径）执行。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceCommand {
    NewLine,
    DeleteLastSentence,
    Undo,
    /// 按回车。默认关闭，免得在终端里把半截话发给 AI。
    Send,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandOptions {
    pub allow_send: bool,
}

const WORDS: &[(&str, VoiceCommand)] = &[
    ("换行", VoiceCommand::NewLine),
    ("删掉上一句", VoiceCommand::DeleteLastSentence),
    ("删除上一句", VoiceCommand::DeleteLastSentence),
    ("撤销", VoiceCommand::Undo),
    ("发送", VoiceCommand::Send),
];

/// 整句是指令就返回它，否则返回 None（当普通文字上屏）。
pub fn parse_command(text: &str, options: &CommandOptions) -> Option<VoiceCommand> {
    let core = text
        .trim()
        .trim_end_matches(['。', '！', '？', '!', '?', '.', '，', ','])
        .trim();
    let command = WORDS.iter().find(|(w, _)| *w == core).map(|(_, c)| *c)?;
    (command != VoiceCommand::Send || options.allow_send).then_some(command)
}
