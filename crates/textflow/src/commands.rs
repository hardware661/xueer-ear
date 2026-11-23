//! 语音指令：整句只说一个指令的说法（可带句末标点）才算指令，句中出现一律当普通文字。
//! 这里只负责认出指令；真正按键由调用方（Win+H / 快捷键两条输出路径）执行。
//!
//! 规则固定，说法可以改（控制面板「语音指令」，存在 settings.json，主人 2026-11-24）：
//! - 单独说一句才算指令。比对时不分大小写、不管标点和空格，但不按读音猜：常被听错的写法加成另一个说法。
//! - 能接在一句话后面说的（「好的，发送」）只有换行、发送和三种翻译；「删除上一句」的说法里带「上一」「上句」的也行。
//! - 指令比快捷短语优先；一个说法只能属于一个指令，也不能和快捷短语的口令读音一样。
//! - 说法 2～10 个字，每个指令最多 16 个；清空就是不用这个指令。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceCommand {
    NewLine,
    /// 删掉雪耳刚打的上一句。「删除」「撤销」「撤回」都是它（主人 2026-11-15：应该是同一个功能）
    DeleteLastSentence,
    /// 删掉这次听写里雪耳打的全部内容
    DeleteAll,
    /// 按回车。默认关闭，免得在终端里把半截话发给 AI。
    Send,
    /// 把这一轮听写打出来的字整段翻译（主人 2026-11-19：只在最后单独说，和「发送」一样）。
    /// 方向看设置：默认按内容定，有中文就翻成英文，全是英文就翻成中文
    Translate,
    /// 「翻译成英文」：整段变成英文，已经是英文的部分不动（主人 2026-11-21）
    TranslateToEnglish,
    /// 「翻译成中文」：整段变成中文，已经是中文的部分不动
    TranslateToChinese,
}

/// 每个指令最多几个说法
pub const MAX_WORDS: usize = 16;
/// 一个说法最少、最多几个字（只数字母、数字和汉字）
const WORD_CHARS: std::ops::RangeInclusive<usize> = 2..=10;

impl VoiceCommand {
    /// 控制面板里的顺序
    pub const ALL: [Self; 7] = [
        Self::NewLine,
        Self::DeleteLastSentence,
        Self::DeleteAll,
        Self::Send,
        Self::Translate,
        Self::TranslateToEnglish,
        Self::TranslateToChinese,
    ];

    /// 是不是三种翻译指令之一
    pub fn is_translate(self) -> bool {
        matches!(
            self,
            Self::Translate | Self::TranslateToEnglish | Self::TranslateToChinese
        )
    }

    /// 设置文件里的名字
    pub fn id(self) -> &'static str {
        match self {
            Self::NewLine => "new_line",
            Self::DeleteLastSentence => "delete_last",
            Self::DeleteAll => "delete_all",
            Self::Send => "send",
            Self::Translate => "translate",
            Self::TranslateToEnglish => "to_english",
            Self::TranslateToChinese => "to_chinese",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|command| command.id() == id)
    }

    /// 控制面板上的名字
    pub fn name(self) -> &'static str {
        match self {
            Self::NewLine => "换行",
            Self::DeleteLastSentence => "删除上一句",
            Self::DeleteAll => "全部删除",
            Self::Send => "发送",
            Self::Translate => "翻译",
            Self::TranslateToEnglish => "翻译成英文",
            Self::TranslateToChinese => "翻译成中文",
        }
    }

    /// 做什么（改说法的小框里那一行，要短）
    pub fn effect(self) -> &'static str {
        match self {
            Self::NewLine => "另起一行（终端里发 Ctrl+J）",
            Self::DeleteLastSentence => "删掉雪耳打的上一句",
            Self::DeleteAll => "删掉雪耳在这个框里打的全部",
            Self::Send => "按回车",
            Self::Translate => "整框翻译，方向看设置",
            Self::TranslateToEnglish => "整框变成英文",
            Self::TranslateToChinese => "整框变成中文",
        }
    }

    /// 自带的说法
    pub fn builtin_words(self) -> &'static [&'static str] {
        match self {
            Self::NewLine => &["换行"],
            Self::DeleteLastSentence => &[
                "删除",
                "删掉",
                "删了",
                "撤销",
                "撤回",
                "删掉上一句",
                "删除上一句",
                "删掉上一句话",
                "删除上一句话",
                "删掉上句话",
                "删除上句话",
                "删掉上一行",
                "删除上一行",
            ],
            Self::DeleteAll => &[
                "全部删除",
                "全部删掉",
                "全都删除",
                "全都删掉",
                "全删了",
                "清空",
            ],
            Self::Send => &["发送"],
            Self::Translate => &["翻译"],
            Self::TranslateToEnglish => &["翻译成英文"],
            Self::TranslateToChinese => &["翻译成中文"],
        }
    }

    /// 默认开着吗：只有「发送」默认关，免得在终端里把半截话发出去
    pub fn on_by_default(self) -> bool {
        self != Self::Send
    }

    /// 能接在一句话末尾执行吗（「第一行，换行」「今天吃什么，发送」「这句不对，删掉上一句」）。
    /// 「删除」「清空」「撤销」这类短词在日常说话里太常见（「这些临时文件，全部删除」），
    /// 只在整句单独说时才算（审查 R2，2026-11-16）；说法里明说了「上一句」的才能接在句尾。
    fn may_trail(self, said: &str) -> bool {
        match self {
            // 翻译和「发送」一样，单独说或接在句尾都算（主人 2026-11-19；11-21 加了「翻译成英文 / 中文」）
            Self::NewLine | Self::Send => true,
            command if command.is_translate() => true,
            Self::DeleteLastSentence => said.contains("上一") || said.contains("上句"),
            _ => false,
        }
    }
}

/// 每个指令现在用的说法（空 = 不用这个指令）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOptions {
    words: Vec<(VoiceCommand, Vec<String>)>,
}

impl Default for CommandOptions {
    /// 自带说法，「发送」关着
    fn default() -> Self {
        Self::with_words(|_| None)
    }
}

impl CommandOptions {
    /// `custom` 给出改过的说法（空列表 = 关掉这个指令）；None 用默认：自带说法，「发送」关着
    pub fn with_words(custom: impl Fn(VoiceCommand) -> Option<Vec<String>>) -> Self {
        let words = VoiceCommand::ALL
            .into_iter()
            .map(|command| {
                let words = custom(command).unwrap_or_else(|| {
                    if command.on_by_default() {
                        command
                            .builtin_words()
                            .iter()
                            .map(|w| w.to_string())
                            .collect()
                    } else {
                        Vec::new()
                    }
                });
                (command, words)
            })
            .collect();
        Self { words }
    }

    /// 某个指令现在的说法；空的就是关着
    pub fn words(&self, command: VoiceCommand) -> &[String] {
        self.words
            .iter()
            .find(|(c, _)| *c == command)
            .map_or(&[], |(_, words)| words.as_slice())
    }

    /// 整句（已去掉口癖）是哪个指令的说法
    fn lookup(&self, said: &str) -> Option<VoiceCommand> {
        let key = plain(said);
        if key.is_empty() {
            return None;
        }
        self.words
            .iter()
            .find(|(_, words)| words.iter().any(|w| plain(w) == key))
            .map(|(command, _)| *command)
    }

    /// 读音和 `trigger` 一样的说法（现在用的和自带的都算：自带的随时可能恢复）
    pub(crate) fn clashing_word(&self, trigger: &str) -> Option<String> {
        let current = self
            .words
            .iter()
            .flat_map(|(_, words)| words.iter().cloned());
        let builtin = VoiceCommand::ALL
            .into_iter()
            .flat_map(|c| c.builtin_words().iter().map(|w| w.to_string()));
        current.chain(builtin).find(|word| {
            plain(word) == plain(trigger) || crate::hotwords::sounds_like(word, trigger)
        })
    }
}

/// 句尾单独一小句是能接在句尾的指令就返回它（规则见 [`VoiceCommand::may_trail`]）。
pub fn parse_trailing_command(text: &str, options: &CommandOptions) -> Option<VoiceCommand> {
    let command = parse_command(text, options)?;
    command.may_trail(text).then_some(command)
}

/// 整句是指令就返回它，否则返回 None（当普通文字上屏）。
pub fn parse_command(text: &str, options: &CommandOptions) -> Option<VoiceCommand> {
    options.lookup(text)
}

/// 把小框里填的一行拆成几个说法：用顿号、逗号、分号、斜杠隔开；
/// 中文说法之间只隔了空格的也拆开（「撤回 撤销」），英文词组里的空格留着（「new line」）。
pub fn split_words(input: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for piece in input.split(['、', '，', ',', '；', ';', '/', '|', '\n', '\r']) {
        let piece = piece.trim();
        if piece.is_empty() {
            continue;
        }
        let tokens: Vec<&str> = piece.split_whitespace().collect();
        if tokens.len() > 1 && tokens.iter().all(|t| t.chars().any(is_han)) {
            out.extend(tokens.iter().map(|t| t.to_string()));
        } else {
            out.push(tokens.join(" "));
        }
    }
    out
}

/// 检查给 `command` 填的说法（已用 [`split_words`] 拆好）是否合规矩：字数、个数、重复、
/// 和别的指令撞、和快捷短语口令读音一样。空列表合规（就是关掉）。不合规返回原因。
pub fn check_words(
    command: VoiceCommand,
    words: &[String],
    options: &CommandOptions,
    snippet_triggers: &[&str],
) -> Result<(), String> {
    if words.len() > MAX_WORDS {
        return Err(format!("最多 {MAX_WORDS} 个说法"));
    }
    for (i, word) in words.iter().enumerate() {
        let key = plain(word);
        if !WORD_CHARS.contains(&key.chars().count()) || word.chars().count() > 20 {
            return Err(format!(
                "「{word}」不行：说法要 {}～{} 个字",
                WORD_CHARS.start(),
                WORD_CHARS.end()
            ));
        }
        if words[..i].iter().any(|w| plain(w) == key) {
            return Err(format!("「{word}」写了两遍"));
        }
        if let Some(other) = VoiceCommand::ALL
            .into_iter()
            .filter(|c| *c != command)
            .find(|c| options.words(*c).iter().any(|w| plain(w) == key))
        {
            return Err(format!("「{word}」已经是「{}」的说法了", other.name()));
        }
        if let Some(trigger) = snippet_triggers
            .iter()
            .find(|t| plain(t) == key || crate::hotwords::sounds_like(t, word))
        {
            return Err(format!("「{word}」和快捷短语「{trigger}」读音一样，换一个"));
        }
    }
    Ok(())
}

/// 只留字母、数字和汉字，英文转小写：「Send。」和「send」、「撤销！」和「撤销」一样
pub(crate) fn plain(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn is_han(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{f900}'..='\u{faff}')
}
