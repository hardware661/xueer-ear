//! 语音指令：整句只说一个指令（可带标点）才算指令，句中出现一律当普通文字。
//! 这里只负责认出指令；什么时候算「单独说」（前面停够、说完没再接着说）、真正按键由调用方定。
//!
//! 规则固定（主人 2026-11-24：说法就是指令，要规范）：
//! - 一个指令只有一个说法：删除、全部删除、发送、翻译成英文、翻译成中文。
//! - 比对时不分大小写、不管标点和空格，但不按读音猜。
//! - 指令比快捷短语优先；快捷短语的口令不能和这几个说法读音一样。
//! - 每个指令可以单独关掉；「发送」默认关，免得在终端里把半截话发出去。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceCommand {
    /// 删掉输入框里最后一句
    DeleteLastSentence,
    /// 清空整个输入框
    DeleteAll,
    /// 按回车
    Send,
    /// 整个输入框翻成英文，换掉原文
    TranslateToEnglish,
    /// 整个输入框翻成中文，换掉原文
    TranslateToChinese,
}

impl VoiceCommand {
    /// 控制面板里的顺序
    pub const ALL: [Self; 5] = [
        Self::DeleteLastSentence,
        Self::DeleteAll,
        Self::Send,
        Self::TranslateToEnglish,
        Self::TranslateToChinese,
    ];

    /// 是不是翻译指令
    pub fn is_translate(self) -> bool {
        matches!(self, Self::TranslateToEnglish | Self::TranslateToChinese)
    }

    /// 设置文件里的名字
    pub fn id(self) -> &'static str {
        match self {
            Self::DeleteLastSentence => "delete_last",
            Self::DeleteAll => "delete_all",
            Self::Send => "send",
            Self::TranslateToEnglish => "to_english",
            Self::TranslateToChinese => "to_chinese",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|command| command.id() == id)
    }

    /// 说法，也是控制面板上的名字
    pub fn name(self) -> &'static str {
        match self {
            Self::DeleteLastSentence => "删除",
            Self::DeleteAll => "全部删除",
            Self::Send => "发送",
            Self::TranslateToEnglish => "翻译成英文",
            Self::TranslateToChinese => "翻译成中文",
        }
    }

    /// 做什么（控制面板菜单里那一行，要短）
    pub fn effect(self) -> &'static str {
        match self {
            Self::DeleteLastSentence => "删掉输入框里最后一句",
            Self::DeleteAll => "清空整个输入框",
            Self::Send => "按回车",
            Self::TranslateToEnglish => "整个输入框翻成英文",
            Self::TranslateToChinese => "整个输入框翻成中文",
        }
    }

    /// 默认开着吗：只有「发送」默认关
    pub fn on_by_default(self) -> bool {
        self != Self::Send
    }
}

/// 哪些指令开着
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOptions {
    on: Vec<VoiceCommand>,
}

impl Default for CommandOptions {
    /// 「发送」关着，别的开着
    fn default() -> Self {
        Self::with_switches(|_| None)
    }
}

impl CommandOptions {
    /// `switch` 给出改过的开关；None 用默认（「发送」关，别的开）
    pub fn with_switches(switch: impl Fn(VoiceCommand) -> Option<bool>) -> Self {
        Self {
            on: VoiceCommand::ALL
                .into_iter()
                .filter(|&command| switch(command).unwrap_or(command.on_by_default()))
                .collect(),
        }
    }

    pub fn is_on(&self, command: VoiceCommand) -> bool {
        self.on.contains(&command)
    }

    /// 整句是哪个开着的指令
    fn lookup(&self, said: &str) -> Option<VoiceCommand> {
        let key = plain(said);
        if key.is_empty() {
            return None;
        }
        self.on
            .iter()
            .copied()
            .find(|command| plain(command.name()) == key)
    }

    /// 说到一半：到目前为止的整句是不是某个开着的指令的开头（或者就是它）。
    /// 是的话先不打出来，等说完再看是不是指令
    pub fn starts_a_command(&self, said: &str) -> bool {
        let key = plain(said);
        !key.is_empty()
            && self
                .on
                .iter()
                .any(|command| plain(command.name()).starts_with(&key))
    }

    /// 读音和 `trigger` 一样的指令说法（关着的也算：随时可能打开）
    pub(crate) fn clashing_word(&self, trigger: &str) -> Option<String> {
        VoiceCommand::ALL
            .into_iter()
            .map(VoiceCommand::name)
            .find(|word| {
                plain(word) == plain(trigger) || crate::hotwords::sounds_like(word, trigger)
            })
            .map(str::to_owned)
    }
}

/// 整句是开着的指令就返回它，否则返回 None（当普通文字上屏）。
pub fn parse_command(text: &str, options: &CommandOptions) -> Option<VoiceCommand> {
    options.lookup(text)
}

/// 只留字母、数字和汉字，英文转小写：「Send。」和「send」、「删除！」和「删除」一样
pub(crate) fn plain(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}
