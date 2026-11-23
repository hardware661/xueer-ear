//! 控制面板各行的菜单：选项列表、点选后的设置变化、行上显示的摘要（纯逻辑，可在 WSL 测）。
//! 菜单项是（命令号, 文字, 是否打勾）；命令号 0 且文字为空表示分隔线。
use xueer_ear_backend::settings::{
    Engine, MAX_LISTEN_OPTIONS, PAUSE_MS_OPTIONS, QUIET_STOP_OPTIONS, SETTLE_MS_OPTIONS,
    SILENCE_SECONDS_OPTIONS, Settings,
};
use xueer_ear_textflow::{Hotwords, Snippets, VoiceCommand};

/// 点了没反应的占位项（「还没有记录」）
pub const NOTHING: usize = 1099;
pub const OPEN_HISTORY: usize = 1100;
pub const OPEN_HOTWORDS: usize = 1101;
pub const ADD_HOTWORD: usize = 1102;
pub const BUILTIN_HOTWORDS: usize = 1103;
pub const OPEN_TRANSLATOR: usize = 1104;
pub const OPEN_TRANSLATE_KEYS: usize = 1105;
pub const COPY_DIAGNOSTICS: usize = 1106;
pub const ADD_SNIPPET: usize = 1107;
pub const OPEN_SNIPPETS: usize = 1108;
const BASE: usize = 1110;
const KEY_BASE: usize = 1200;
const HISTORY_BASE: usize = 1300;
const SNIPPET_BASE: usize = 1400;
/// 历史菜单里的「记录历史」，后面跟着几个保留天数
pub const HISTORY_SWITCH: usize = 1500;
const HISTORY_DAYS: usize = 1501;
/// 「语音指令」菜单：每个指令（按 VoiceCommand::ALL 的顺序）、四个时间、翻译方向、终端换行、恢复默认
const COMMAND_BASE: usize = 1600;
const TIME_BASE: usize = 1610;
/// 「更多设置」里的三个听写时间
const DICTATION_TIME_BASE: usize = 1630;
/// 点了某个时间、翻译方向以后，第二个菜单里的选项
const OPTION_BASE: usize = 1700;
/// 快捷短语菜单里最多列几条（多了去表里改）
pub const SNIPPETS_SHOWN: usize = 12;
/// 历史菜单里列最近多少句
pub const HISTORY_SHOWN: usize = 10;
const DAYS: [u32; 4] = [1, 7, 30, 90];
/// 可选的快捷键（虚拟键码）；都是单独按不会打出字的键
const KEYS: [(&str, u32); 5] = [
    ("右 Alt", 0xA5),
    ("右 Ctrl", 0xA3),
    ("CapsLock", 0x14),
    ("F8", 0x77),
    ("F9", 0x78),
];

type Switch = (&'static str, fn(&mut Settings) -> &mut bool);
type Choice = (usize, String, bool);

/// 「更多设置」里的开关，顺序就是菜单顺序；`GROUPS` 说明每组几个，组之间画分隔线。
/// 菜单不能太长：150% 缩放时 1080p 屏幕放不下（「记录历史」和保留天数因此放在历史那一行的菜单里，
/// 「发送」、终端换行、一次最长听多久在「语音指令」那一行）
const SWITCHES: [Switch; 10] = [
    ("去掉口头禅", |s| &mut s.text.strip_fillers),
    ("中文数字转阿拉伯数字", |s| {
        &mut s.text.normalize_numbers
    }),
    ("去掉末尾句号", |s| &mut s.text.drop_final_period),
    ("中英文之间加空格", |s| &mut s.text.cjk_latin_space),
    ("中文旁用全角标点", |s| {
        &mut s.text.fullwidth_punct_in_cjk
    }),
    ("降噪（开风扇、空调时也能断好句）", |s| {
        &mut s.noise_reduction
    }),
    ("听写时调小别的声音", |s| &mut s.duck_others),
    ("提示音", |s| &mut s.sounds_enabled),
    ("同时接管 Win+H", |s| &mut s.winh_enabled),
    ("以管理员身份运行", |s| &mut s.run_as_admin),
];
/// 文字整理 5 个；降噪、调小别的声音、提示音 3 个；Win+H、管理员 2 个
const GROUPS: [usize; 2] = [5, 8];
/// 「更多设置」最上面两个单选：哪家是主通道（另一家一直是备用）
const ENGINES: [(Engine, &str); 2] = [
    (Engine::Doubao, "豆包为主，微信备用"),
    (Engine::Wetype, "微信为主，豆包备用"),
];
const ENGINE_BASE: usize = BASE + SWITCHES.len();

fn separator() -> Choice {
    (0, String::new(), false)
}

/// 「更多设置」菜单：最上面是哪家识别为主
pub fn choices(s: &Settings) -> Vec<Choice> {
    let mut copy = s.clone();
    let mut out: Vec<Choice> = ENGINES
        .iter()
        .enumerate()
        .map(|(i, (engine, text))| (ENGINE_BASE + i, (*text).to_owned(), s.engine == *engine))
        .collect();
    out.push(separator());
    out.extend(
        DICTATION_TIMINGS
            .iter()
            .enumerate()
            .map(|(i, timing)| (DICTATION_TIME_BASE + i, timing.label(s), false)),
    );
    out.push(separator());
    for (i, (text, field)) in SWITCHES.iter().enumerate() {
        if GROUPS.contains(&i) {
            out.push(separator());
        }
        out.push((BASE + i, (*text).to_owned(), *field(&mut copy)));
    }
    out.push(separator());
    out.push((COPY_DIAGNOSTICS, "复制诊断信息".to_owned(), false));
    out
}

/// 报问题时贴上的诊断信息：版本、系统、几项设置、后台状态、日志最后几行。
/// 日志里只有数字和错误原因，不记说过的话；用户目录换成环境变量名，不带出用户名
pub fn diagnostics(
    version: &str,
    windows: &str,
    s: &Settings,
    status: &str,
    log: &str,
    folders: &[(&str, &str)],
) -> String {
    let lines: Vec<&str> = log.lines().collect();
    let tail = lines[lines.len().saturating_sub(DIAGNOSTIC_LINES)..].join("\n");
    let on = |value: bool| if value { "开" } else { "关" };
    let options = s.command_options();
    let enabled = VoiceCommand::ALL
        .into_iter()
        .filter(|c| options.is_on(*c))
        .count();
    let mut out = format!(
        "雪耳 {version}\nWindows {windows}\n识别 {}；降噪 {}；调小别的声音 {}\n快捷键 {}；接管 Win+H {}；管理员身份 {}\n语音指令开着 {enabled} 个；指令前停 {} 秒；指令后等 {} 秒；停 {} 秒收句；{} 秒没新字结束；一次最长听 {} 分钟\n后台：{status}\n\n--- service.log 最后 {} 行 ---\n{tail}\n",
        match s.engine {
            Engine::Doubao => "豆包为主（微信备用）",
            Engine::Wetype => "微信为主（豆包备用）",
        },
        on(s.noise_reduction),
        on(s.duck_others),
        key_name(s.hotkey_vk),
        on(s.winh_enabled),
        on(s.run_as_admin),
        seconds(s.commands.pause_ms),
        seconds(s.commands.settle_ms),
        s.silence_seconds,
        s.quiet_stop_seconds,
        s.max_listen_minutes,
        DIAGNOSTIC_LINES,
    );
    // 长的先换（LOCALAPPDATA 在 USERPROFILE 里面）
    let mut folders = folders.to_vec();
    folders.sort_by_key(|(_, path)| std::cmp::Reverse(path.len()));
    for (name, path) in folders {
        if path.len() > 3 {
            out = out.replace(path, &format!("%{name}%"));
        }
    }
    out
}

/// 诊断信息里带几行日志
const DIAGNOSTIC_LINES: usize = 50;

/// 点了「更多设置」里某项后的新设置；不认识的命令号返回 None。
pub fn toggle(s: &Settings, id: usize) -> Option<Settings> {
    let mut next = s.clone();
    let index = id.checked_sub(BASE)?;
    if let Some((_, field)) = SWITCHES.get(index) {
        let value = field(&mut next);
        *value = !*value;
    } else {
        next.engine = ENGINES.get(index - SWITCHES.len())?.0;
    }
    Some(next)
}

/// 改了这些，要重启后台服务才生效；其余的从下一次听写开始生效。
pub fn needs_restart(old: &Settings, new: &Settings) -> bool {
    old.winh_enabled != new.winh_enabled
}

/// 快捷键的名字（「快捷键」那一行显示）
pub fn key_name(vk: u32) -> String {
    if let Some((name, _)) = KEYS.iter().find(|(_, key)| *key == vk) {
        return (*name).to_owned();
    }
    match vk {
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        _ => format!("键 {vk:#x}"),
    }
}

/// 「快捷键」菜单；设置文件里手写了表外的键，也列出来并打勾
pub fn key_choices(s: &Settings) -> Vec<Choice> {
    let mut out: Vec<_> = KEYS
        .iter()
        .enumerate()
        .map(|(i, (name, vk))| (KEY_BASE + i, (*name).to_owned(), s.hotkey_vk == *vk))
        .collect();
    if !KEYS.iter().any(|(_, vk)| *vk == s.hotkey_vk) {
        out.push((NOTHING, key_name(s.hotkey_vk), true));
    }
    out
}

pub fn pick_key(s: &Settings, id: usize) -> Option<Settings> {
    let (_, vk) = KEYS.get(id.checked_sub(KEY_BASE)?)?;
    let mut next = s.clone();
    next.hotkey_vk = *vk;
    Some(next)
}

/// 「添加热词」对话框的两格（正确的写法、常被听成什么）拼成热词表里的一行。
pub fn hotword_line(word: &str, heard: &str) -> Result<String, &'static str> {
    let (word, heard) = (word.trim(), heard.trim());
    if word.is_empty() {
        return Err("先填正确的写法");
    }
    for text in [word, heard] {
        if text.contains('=') || text.starts_with('#') || text.chars().any(char::is_control) {
            return Err("不能用 = 和 # 开头");
        }
        if text.chars().count() > 40 {
            return Err("太长了，最多 40 个字");
        }
    }
    if heard.is_empty() {
        Ok(word.to_owned())
    } else if heard == word {
        Err("两格写的是一样的")
    } else {
        Ok(format!("{heard}={word}"))
    }
}

/// 「热词」菜单：添加、打开热词表、自带词表开关
pub fn hotword_choices(s: &Settings) -> Vec<Choice> {
    vec![
        (ADD_HOTWORD, "添加热词…".to_owned(), false),
        (OPEN_HOTWORDS, "打开热词表".to_owned(), false),
        separator(),
        (
            BUILTIN_HOTWORDS,
            "使用自带热词".to_owned(),
            s.text.builtin_hotwords,
        ),
    ]
}

/// 「热词」那一行显示：有几个词（含替换规则），一个都没有就显示「添加」
pub fn hotword_summary(file: &str) -> String {
    let hotwords = Hotwords::parse(file);
    match hotwords.words().len() + hotwords.rules().len() {
        0 => "添加".to_owned(),
        count => format!("{count} 个"),
    }
}

/// 「添加快捷短语」对话框的两格（口令、换成的文字）。文字里打 `\n` 表示换行。
/// 返回（口令, 文字）；口令和语音指令的说法撞了、太长、空着都不行，说明原因。
pub fn snippet_entry(
    trigger: &str,
    text: &str,
    commands: &xueer_ear_textflow::CommandOptions,
) -> Result<(String, String), String> {
    let trigger = trigger.trim();
    let text = text.trim().replace("\\n", "\n");
    if trigger.chars().count() > 20 {
        return Err("口令太长了，最多 20 个字".to_owned());
    }
    if text.chars().count() > 500 {
        return Err("文字太长了，最多 500 个字".to_owned());
    }
    xueer_ear_textflow::set_snippet_line("", trigger, &text, commands)?;
    Ok((trigger.to_owned(), text))
}

/// 「快捷短语」菜单：添加、打开表；下面列出已有的（点一条可以删掉）
pub fn snippet_choices(file: &str) -> Vec<Choice> {
    let snippets = Snippets::parse(file);
    let mut out = vec![
        (ADD_SNIPPET, "添加快捷短语…".to_owned(), false),
        (OPEN_SNIPPETS, "打开快捷短语表".to_owned(), false),
    ];
    if !snippets.is_empty() {
        out.push(separator());
        out.push((NOTHING, "点一条可以删掉：".to_owned(), false));
        out.extend(
            snippets
                .entries()
                .iter()
                .take(SNIPPETS_SHOWN)
                .enumerate()
                .map(|(i, (trigger, text))| {
                    (
                        SNIPPET_BASE + i,
                        format!("{trigger} → {}", history_label(text)),
                        false,
                    )
                }),
        );
    }
    out
}

/// 点了快捷短语菜单里的哪一条：返回它的口令
pub fn snippet_pick(file: &str, id: usize) -> Option<String> {
    let index = id.checked_sub(SNIPPET_BASE)?;
    (index < SNIPPETS_SHOWN)
        .then(|| {
            Snippets::parse(file)
                .entries()
                .get(index)
                .map(|(t, _)| t.clone())
        })
        .flatten()
}

/// 「错=对」这条规则加进热词表的话，历史里会有几句跟着改
pub fn rule_impact(line: &str, texts: &[String]) -> usize {
    let rule = Hotwords::parse(line);
    texts
        .iter()
        .filter(|text| rule.apply(text) != **text)
        .count()
}

/// 「快捷短语」那一行显示：有几条，一条都没有就显示「添加」
pub fn snippet_summary(file: &str) -> String {
    match Snippets::parse(file).entries().len() {
        0 => "添加".to_owned(),
        count => format!("{count} 条"),
    }
}

/// 「历史」那一行显示
pub fn history_summary(s: &Settings) -> String {
    if s.history_enabled {
        format!("最近 {} 天", s.history_keep_days)
    } else {
        "已关闭".to_owned()
    }
}

/// 历史菜单里的一行：压成一行，太长就截断
pub fn history_label(text: &str) -> String {
    const MAX: usize = 24;
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= MAX {
        return flat;
    }
    flat.chars().take(MAX - 1).chain(Some('…')).collect()
}

/// 「历史」菜单：最近说过的话（最新在上，点一句就复制）、打开全部记录；最下面是记不记、留几天
pub fn history_choices(texts: &[String], s: &Settings) -> Vec<Choice> {
    let mut out = Vec::new();
    if s.history_enabled {
        if texts.is_empty() {
            out.push((NOTHING, "还没有记录".to_owned(), false));
        } else {
            out.extend(
                texts
                    .iter()
                    .enumerate()
                    .map(|(i, text)| (HISTORY_BASE + i, history_label(text), false)),
            );
            out.push(separator());
            out.push((OPEN_HISTORY, "打开全部记录".to_owned(), false));
        }
        out.push(separator());
    }
    out.push((HISTORY_SWITCH, "记录历史".to_owned(), s.history_enabled));
    out.extend(DAYS.iter().enumerate().map(|(i, days)| {
        (
            HISTORY_DAYS + i,
            format!("保留 {days} 天"),
            s.history_enabled && s.history_keep_days == *days,
        )
    }));
    out
}

/// 点了历史菜单里的「记录历史」或保留天数以后的新设置；点的是别的返回 None
pub fn history_setting(s: &Settings, id: usize) -> Option<Settings> {
    let mut next = s.clone();
    if id == HISTORY_SWITCH {
        next.history_enabled = !s.history_enabled;
        return Some(next);
    }
    next.history_keep_days = *DAYS.get(id.checked_sub(HISTORY_DAYS)?)?;
    next.history_enabled = true;
    Some(next)
}

/// 点了历史菜单里的哪一句（全文）
pub fn history_pick(texts: &[String], id: usize) -> Option<&str> {
    texts.get(id.checked_sub(HISTORY_BASE)?).map(String::as_str)
}

/// 「翻译」一行按钮上的字：火山、微软、B 站三家免费的总在，没有「未配置」
pub fn translate_summary(config: &Result<xueer_ear_backend::translate::Keys, String>) -> String {
    match config {
        Err(_) => "设置出错",
        Ok(_) => "打开",
    }
    .to_owned()
}

/// 「翻译」菜单：打开翻译小窗、查看服务、修改本机翻译设置。
pub fn translate_choices(
    config: &Result<xueer_ear_backend::translate::Keys, String>,
) -> Vec<Choice> {
    let status = match config {
        Err(reason) => reason.clone(),
        Ok(keys) => format!(
            "服务：{}，谁快先用谁",
            xueer_ear_backend::translate::services(keys)
                .into_iter()
                .map(|service| service.name())
                .collect::<Vec<_>>()
                .join("、")
        ),
    };
    vec![
        (OPEN_TRANSLATOR, "打开翻译小窗".to_owned(), false),
        separator(),
        (NOTHING, status, false),
        (
            NOTHING,
            "听写时说「翻译成英文 / 中文」：整框翻译".to_owned(),
            false,
        ),
        (
            NOTHING,
            "读不到输入框（终端）就翻雪耳打的字".to_owned(),
            false,
        ),
        separator(),
        (OPEN_TRANSLATE_KEYS, "打开翻译设置".to_owned(), false),
    ]
}

/// 托盘右键菜单：上面一行灰字说状态，中间是常用的几项，最后是退出。
/// 状态行和分隔线不能点（命令号 0 / NOTHING）。
pub fn tray_choices(
    headline: &str,
    hotkey_vk: u32,
    running: bool,
    busy: bool,
    open: usize,
    toggle: usize,
    exit: usize,
) -> Vec<Choice> {
    let status = if running {
        format!("{headline} · 点一下 {} 说话", key_name(hotkey_vk))
    } else {
        headline.to_owned()
    };
    let mut out = vec![(NOTHING, status, false), separator()];
    out.push((open, "打开控制面板".to_owned(), false));
    if !busy {
        out.push((
            toggle,
            if running {
                "暂停雪耳"
            } else {
                "开启雪耳"
            }
            .to_owned(),
            false,
        ));
    }
    out.push(separator());
    out.push((exit, "退出".to_owned(), false));
    out
}

/// 「语音指令」菜单里点了什么
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandPick {
    /// 开关这个指令
    Switch(VoiceCommand),
    /// 改某个时间（再弹一个菜单选几档）
    Time(Timing),
}

/// 能改的几个时间：前两个在「语音指令」里，后三个在「更多设置」里
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timing {
    /// 指令前要停多久才算单独说
    Pause,
    /// 说完指令后等多久执行
    Settle,
    /// 停多久算一句说完
    Silence,
    /// 多久没有新字自动结束
    QuietStop,
    /// 一次最长听多久
    Listen,
}

/// 「语音指令」菜单里的两个时间
const COMMAND_TIMINGS: [Timing; 2] = [Timing::Pause, Timing::Settle];
/// 「更多设置」菜单里的三个听写时间（主人 2026-11-24 从「语音指令」挪过来）
const DICTATION_TIMINGS: [Timing; 3] = [Timing::Silence, Timing::QuietStop, Timing::Listen];

/// 毫秒写成「0.6」「1」这样的秒数
fn seconds(ms: u32) -> String {
    if ms % 1000 == 0 {
        (ms / 1000).to_string()
    } else {
        format!("{:.1}", f64::from(ms) / 1000.)
    }
}

impl Timing {
    /// 菜单里那一行：带上现在的值
    fn label(self, s: &Settings) -> String {
        match self {
            Self::Pause => format!("指令前停 {} 秒…", seconds(s.commands.pause_ms)),
            Self::Settle => format!("说完指令后等 {} 秒执行…", seconds(s.commands.settle_ms)),
            Self::Silence => format!("停 {} 秒算一句说完…", s.silence_seconds),
            Self::QuietStop => format!("{} 秒没有新字自动结束…", s.quiet_stop_seconds),
            Self::Listen => format!("一次最长听 {} 分钟…", s.max_listen_minutes),
        }
    }

    /// 几档可选的值、默认值、现在的值
    fn options(self, s: &Settings) -> (&'static [u32], u32, u32) {
        let default = Settings::default();
        match self {
            Self::Pause => (
                PAUSE_MS_OPTIONS,
                default.commands.pause_ms,
                s.commands.pause_ms,
            ),
            Self::Settle => (
                SETTLE_MS_OPTIONS,
                default.commands.settle_ms,
                s.commands.settle_ms,
            ),
            Self::Silence => (
                SILENCE_SECONDS_OPTIONS,
                default.silence_seconds,
                s.silence_seconds,
            ),
            Self::QuietStop => (
                QUIET_STOP_OPTIONS,
                default.quiet_stop_seconds,
                s.quiet_stop_seconds,
            ),
            Self::Listen => (
                MAX_LISTEN_OPTIONS,
                default.max_listen_minutes,
                s.max_listen_minutes,
            ),
        }
    }

    fn value_label(self, value: u32) -> String {
        match self {
            Self::Pause | Self::Settle => format!("{} 秒", seconds(value)),
            Self::Silence | Self::QuietStop => format!("{value} 秒"),
            Self::Listen => format!("{value} 分钟"),
        }
    }

    fn set(self, s: &mut Settings, value: u32) {
        match self {
            Self::Pause => s.commands.pause_ms = value,
            Self::Settle => s.commands.settle_ms = value,
            Self::Silence => s.silence_seconds = value,
            Self::QuietStop => s.quiet_stop_seconds = value,
            Self::Listen => s.max_listen_minutes = value,
        }
    }
}

/// 「语音指令」那一行显示：开着几个
pub fn command_summary(s: &Settings) -> String {
    let options = s.command_options();
    let on = VoiceCommand::ALL
        .into_iter()
        .filter(|c| options.is_on(*c))
        .count();
    format!("开着 {on} 个")
}

/// 「语音指令」菜单（主人 2026-11-24：说法就是指令，要规范）：最上面一行灰字写规矩，
/// 下面 5 个指令打勾是开着、点一下开关，再下面是两个时间
pub fn command_choices(s: &Settings) -> Vec<Choice> {
    let options = s.command_options();
    let mut out = vec![(
        NOTHING,
        "先停够、单独说指令，再停一下才执行".to_owned(),
        false,
    )];
    out.extend(
        VoiceCommand::ALL
            .into_iter()
            .enumerate()
            .map(|(i, command)| {
                (
                    COMMAND_BASE + i,
                    format!("{}　{}", command.name(), command.effect()),
                    options.is_on(command),
                )
            }),
    );
    out.push(separator());
    out.extend(
        COMMAND_TIMINGS
            .iter()
            .enumerate()
            .map(|(i, timing)| (TIME_BASE + i, timing.label(s), false)),
    );
    out
}

/// 「语音指令」菜单里点的是什么；分隔线、灰字、别的菜单的命令号返回 None
pub fn command_pick(id: usize) -> Option<CommandPick> {
    if let Some(command) = id
        .checked_sub(COMMAND_BASE)
        .and_then(|i| VoiceCommand::ALL.get(i))
    {
        return Some(CommandPick::Switch(*command));
    }
    id.checked_sub(TIME_BASE)
        .and_then(|i| COMMAND_TIMINGS.get(i))
        .map(|timing| CommandPick::Time(*timing))
}

/// 开关一个指令（和默认一样就不单独记）
pub fn switch_command(s: &Settings, command: VoiceCommand) -> Settings {
    let mut next = s.clone();
    let on = !s.command_options().is_on(command);
    if on == command.on_by_default() {
        next.commands.switches.remove(command.id());
    } else {
        next.commands.switches.insert(command.id().to_owned(), on);
    }
    next
}

/// 「更多设置」里点的是不是听写时间
pub fn dictation_timing(id: usize) -> Option<Timing> {
    id.checked_sub(DICTATION_TIME_BASE)
        .and_then(|i| DICTATION_TIMINGS.get(i))
        .copied()
}

/// 点了某个时间以后的第二个菜单：几档，默认那档后面写「默认」；设置文件里手写了别的值也列出来并打勾
pub fn timing_choices(s: &Settings, timing: Timing) -> Vec<Choice> {
    let (values, default, current) = timing.options(s);
    let mut out: Vec<Choice> = values
        .iter()
        .enumerate()
        .map(|(i, value)| {
            let mut text = timing.value_label(*value);
            if *value == default {
                text.push_str("（默认）");
            }
            (OPTION_BASE + i, text, *value == current)
        })
        .collect();
    if !values.contains(&current) {
        out.push((NOTHING, timing.value_label(current), true));
    }
    out
}

/// 第二个菜单里选了一档以后的新设置
pub fn pick_timing(s: &Settings, timing: Timing, id: usize) -> Option<Settings> {
    let (values, _, _) = timing.options(s);
    let value = *values.get(id.checked_sub(OPTION_BASE)?)?;
    let mut next = s.clone();
    timing.set(&mut next, value);
    Some(next)
}

#[cfg(test)]
mod translate_menu_tests {
    use super::*;
    use xueer_ear_backend::translate::Keys;

    #[test]
    fn translate_menu_lists_the_free_services_then_configured_ones() {
        let configured = Ok(Keys {
            google_key: "g".into(),
            zhipu_key: "z".into(),
            ..Keys::default()
        });
        assert_eq!(translate_summary(&configured), "打开");
        let menu = translate_choices(&configured);
        assert_eq!(menu[0].0, OPEN_TRANSLATOR);
        assert!(
            menu.iter()
                .any(|(_, t, _)| t == "服务：火山、微软、B站、微信、谷歌、智谱，谁快先用谁"),
            "{menu:?}"
        );
        // 不用微信、也没配 Key：三家免费的照样能翻
        let disabled = Ok(Keys {
            wetype_enabled: false,
            ..Keys::default()
        });
        assert_eq!(translate_summary(&disabled), "打开");
        assert!(
            translate_choices(&disabled)
                .iter()
                .any(|(_, t, _)| t == "服务：火山、微软、B站，谁快先用谁")
        );
        let invalid = Err("翻译设置文件坏了".to_owned());
        assert_eq!(translate_summary(&invalid), "设置出错");
        assert!(
            translate_choices(&invalid)
                .iter()
                .any(|(_, t, _)| t == "翻译设置文件坏了")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(c: &[(usize, String, bool)], text: &str) -> (usize, bool) {
        c.iter()
            .find(|(_, t, _)| t == text)
            .map(|(id, _, on)| (*id, *on))
            .unwrap_or_else(|| panic!("{text}"))
    }

    #[test]
    fn defaults_are_reflected() {
        let c = choices(&Settings::default());
        assert!(find(&c, "去掉口头禅").1);
        assert!(find(&c, "中文数字转阿拉伯数字").1);
        assert!(!find(&c, "去掉末尾句号").1);
        assert!(!find(&c, "同时接管 Win+H").1);
        assert!(!find(&c, "以管理员身份运行").1);
        assert!(find(&c, "提示音").1);
        assert!(find(&c, "豆包为主，微信备用").1);
        assert!(!find(&c, "微信为主，豆包备用").1);
        // 快捷键、热词、历史、语音指令各有自己的一行，不在「更多设置」里；三个听写时间在这里
        assert!(find(&c, "一次最长听 30 分钟…").0 != 0);
        for moved in ["快捷键", "热词", "发送", "换行", "指令"] {
            assert!(!c.iter().any(|(_, t, _)| t.contains(moved)), "{moved}");
        }
    }

    #[test]
    fn each_switch_flips_only_itself() {
        let s = Settings::default();
        for (id, text, on) in choices(&s) {
            if text.is_empty()
                || text.contains("为主")
                || id == COPY_DIAGNOSTICS
                || dictation_timing(id).is_some()
            {
                continue;
            }
            let next = toggle(&s, id).unwrap_or_else(|| panic!("{text}"));
            assert_ne!(next, s, "{text}");
            let flipped = find(&choices(&next), &text).1;
            assert_eq!(flipped, !on, "{text}");
            assert_eq!(toggle(&next, id).unwrap(), s, "{text} 再点一次应还原");
        }
    }

    #[test]
    fn separators_group_the_menu_and_do_nothing() {
        let s = Settings::default();
        let c = choices(&s);
        let separators = c.iter().filter(|(_, t, _)| t.is_empty()).count();
        assert_eq!(separators, 5);
        // 150% 缩放时 1080p 屏幕要放得下：不能比 1.2 版（20 项、4 条分隔线）更高
        assert!(c.len() - separators <= 20 && c.len() <= 24, "{}", c.len());
        assert!(c.iter().all(|(id, t, _)| t.is_empty() == (*id == 0)));
        assert!(!c.first().unwrap().1.is_empty() && !c.last().unwrap().1.is_empty());
        assert!(toggle(&s, 0).is_none());
    }

    #[test]
    fn history_days_and_off_live_in_the_history_menu() {
        let s = Settings::default();
        let c = history_choices(&[], &s);
        assert!(find(&c, "记录历史").1);
        assert!(find(&c, "保留 7 天").1);
        let next = history_setting(&s, find(&c, "保留 30 天").0).unwrap();
        assert_eq!(next.history_keep_days, 30);
        assert!(next.history_enabled);
        let off = history_setting(&s, HISTORY_SWITCH).unwrap();
        assert!(!off.history_enabled);
        // 关了以后菜单里只剩开关和天数，选个天数就重新打开
        let c = history_choices(&[], &off);
        assert_eq!(c[0], (HISTORY_SWITCH, "记录历史".to_owned(), false));
        assert!(c.iter().all(|(_, _, on)| !on));
        assert!(
            history_setting(&off, find(&c, "保留 1 天").0)
                .unwrap()
                .history_enabled
        );
        assert!(history_setting(&s, OPEN_HISTORY).is_none());
        assert!(toggle(&s, 9999).is_none());
    }

    #[test]
    fn either_channel_can_be_the_main_one_with_the_other_as_backup() {
        let s = Settings::default();
        let (wechat, on) = find(&choices(&s), "微信为主，豆包备用");
        assert!(!on);
        let wetype = toggle(&s, wechat).unwrap();
        assert_eq!(wetype.engine, Engine::Wetype);
        assert!(find(&choices(&wetype), "微信为主，豆包备用").1);
        assert!(!find(&choices(&wetype), "豆包为主，微信备用").1);
        // 单选：再点一次已选的不会变
        assert_eq!(toggle(&wetype, wechat).unwrap(), wetype);
        let (doubao, _) = find(&choices(&wetype), "豆包为主，微信备用");
        assert_eq!(toggle(&wetype, doubao).unwrap(), s);
    }

    #[test]
    fn noise_reduction_and_ducking_switches_are_on_by_default() {
        let c = choices(&Settings::default());
        assert!(find(&c, "降噪（开风扇、空调时也能断好句）").1);
        assert!(find(&c, "听写时调小别的声音").1);
    }

    #[test]
    fn snippet_dialog_checks_the_trigger_and_turns_backslash_n_into_lines() {
        let commands = Settings::default().command_options();
        let entry = |trigger: &str, text: &str| snippet_entry(trigger, text, &commands);
        assert_eq!(
            entry(" 我的地址 ", "某某路 1 号\\n张三收"),
            Ok(("我的地址".to_owned(), "某某路 1 号\n张三收".to_owned()))
        );
        assert!(entry("删除", "x").unwrap_err().contains("删除"));
        // 「换行」已经不是指令了，可以当口令
        assert!(entry("换行", "x").is_ok());
        assert!(entry("", "x").is_err());
        assert!(entry("签名", "  ").is_err());
        assert!(entry(&"长".repeat(21), "x").is_err());
        assert!(entry("签名", &"字".repeat(501)).is_err());
        // 关着的指令的说法也不能当口令（随时可能打开）
        let off = switch_command(&Settings::default(), VoiceCommand::DeleteAll);
        assert!(snippet_entry("全部删除", "x", &off.command_options()).is_err());
    }

    #[test]
    fn snippet_menu_lists_entries_that_can_be_picked_for_deletion() {
        assert_eq!(snippet_summary(""), "添加");
        let file = "# 说明\n我的邮箱=xueer@example.com\n地址=某某路\\n张三收\n";
        assert_eq!(snippet_summary(file), "2 条");
        let c = snippet_choices(file);
        assert!(c.iter().any(|(id, _, _)| *id == ADD_SNIPPET));
        assert!(c.iter().any(|(id, _, _)| *id == OPEN_SNIPPETS));
        let (id, _) = find(&c, "地址 → 某某路 张三收");
        assert_eq!(snippet_pick(file, id).as_deref(), Some("地址"));
        assert_eq!(snippet_pick(file, ADD_SNIPPET), None);
        assert_eq!(snippet_choices("").len(), 2, "没有快捷短语时只有添加和打开");
    }

    #[test]
    fn command_menu_lists_the_five_commands_and_two_times() {
        let s = Settings::default();
        assert_eq!(command_summary(&s), "开着 4 个");
        let c = command_choices(&s);
        assert!(find(&c, "删除　删掉输入框里最后一句").1);
        assert!(find(&c, "全部删除　清空整个输入框").1);
        assert!(!find(&c, "发送　按回车").1);
        assert!(find(&c, "翻译成英文　整个输入框翻成英文").1);
        assert!(find(&c, "翻译成中文　整个输入框翻成中文").1);
        find(&c, "指令前停 1 秒…");
        find(&c, "说完指令后等 0.6 秒执行…");
        // 规矩是灰字，点了没反应
        let rules: Vec<_> = c.iter().filter(|(id, _, _)| *id == NOTHING).collect();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].1, "先停够、单独说指令，再停一下才执行");
        // 每一项都认得出来，和别的菜单不串
        let picks: Vec<_> = c
            .iter()
            .filter(|(id, t, _)| !t.is_empty() && *id != NOTHING)
            .map(|(id, _, _)| command_pick(*id).unwrap())
            .collect();
        assert_eq!(picks.len(), 5 + 2);
        for (id, _, _) in &c {
            assert!(toggle(&s, *id).is_none() && pick_key(&s, *id).is_none());
            assert!(dictation_timing(*id).is_none());
        }
        // 以前的说法、翻译方向、终端换行、恢复默认都没了
        for gone in ["换行", "只说「翻译」", "全部恢复默认", "停 3 秒算一句说完…"]
        {
            assert!(!c.iter().any(|(_, t, _)| t.contains(gone)), "{gone}");
        }
    }

    #[test]
    fn a_command_is_switched_on_and_off_with_one_click() {
        let s = Settings::default();
        let send = switch_command(&s, VoiceCommand::Send);
        assert!(send.command_options().is_on(VoiceCommand::Send));
        assert_eq!(command_summary(&send), "开着 5 个");
        // 关回去就和默认一样，不往设置文件里多写
        assert_eq!(switch_command(&send, VoiceCommand::Send), s);
        let off = switch_command(&s, VoiceCommand::DeleteAll);
        assert!(!off.command_options().is_on(VoiceCommand::DeleteAll));
        assert!(!find(&command_choices(&off), "全部删除　清空整个输入框").1);
        assert!(off.validate().is_ok());
    }

    #[test]
    fn times_are_picked_from_fixed_steps_with_the_default_marked() {
        let s = Settings::default();
        let pause = timing_choices(&s, Timing::Pause);
        assert_eq!(
            pause.iter().map(|(_, t, _)| t.as_str()).collect::<Vec<_>>(),
            ["0.5 秒", "1 秒（默认）", "1.5 秒", "2 秒"]
        );
        let longer = pick_timing(&s, Timing::Pause, find(&pause, "1.5 秒").0).unwrap();
        assert_eq!(longer.commands.pause_ms, 1500);
        let settle = timing_choices(&s, Timing::Settle);
        assert!(find(&settle, "0.6 秒（默认）").1);
        let slower = pick_timing(&s, Timing::Settle, find(&settle, "1 秒").0).unwrap();
        assert_eq!(slower.commands.settle_ms, 1000);
        let silence = timing_choices(&s, Timing::Silence);
        let quick = pick_timing(&s, Timing::Silence, find(&silence, "1 秒").0).unwrap();
        assert_eq!(quick.silence_seconds, 1);
        let quiet = timing_choices(&s, Timing::QuietStop);
        let patient = pick_timing(&s, Timing::QuietStop, find(&quiet, "30 秒").0).unwrap();
        assert_eq!(patient.quiet_stop_seconds, 30);
        let listen = timing_choices(&s, Timing::Listen);
        let long = pick_timing(&s, Timing::Listen, find(&listen, "120 分钟").0).unwrap();
        assert_eq!(long.max_listen_minutes, 120);
        // 设置文件里手写的值也显示出来
        let mut odd = s.clone();
        odd.commands.settle_ms = 800;
        assert!(find(&timing_choices(&odd, Timing::Settle), "0.8 秒").1);
        assert!(pick_timing(&s, Timing::Settle, NOTHING).is_none());
        // 菜单里那一行跟着变
        assert!(
            command_choices(&slower)
                .iter()
                .any(|(_, t, _)| t == "说完指令后等 1 秒执行…")
        );
    }

    // 主人 2026-11-24：三个听写时间从「语音指令」挪到「更多设置」
    #[test]
    fn the_three_dictation_times_live_in_more_settings() {
        let s = Settings::default();
        let c = choices(&s);
        for (text, timing) in [
            ("停 3 秒算一句说完…", Timing::Silence),
            ("10 秒没有新字自动结束…", Timing::QuietStop),
            ("一次最长听 30 分钟…", Timing::Listen),
        ] {
            let (id, ticked) = find(&c, text);
            assert!(!ticked);
            assert_eq!(dictation_timing(id), Some(timing));
            assert!(toggle(&s, id).is_none(), "不是开关");
        }
    }

    #[test]
    fn hotkey_row_shows_and_changes_the_key() {
        let s = Settings::default();
        assert_eq!(key_name(s.hotkey_vk), "右 Alt");
        assert_eq!(key_name(0x77), "F8");
        assert_eq!(key_name(0x7B), "F12");
        assert_eq!(key_name(0x91), "键 0x91");
        let c = key_choices(&s);
        assert!(find(&c, "右 Alt").1);
        let next = pick_key(&s, find(&c, "F8").0).unwrap();
        assert_eq!(next.hotkey_vk, 0x77);
        assert!(find(&key_choices(&next), "F8").1);
        assert!(!find(&key_choices(&next), "右 Alt").1);
        assert!(pick_key(&s, 0).is_none());
        // 两个菜单的命令号不能串
        for (id, _, _) in &c {
            assert!(toggle(&s, *id).is_none());
        }
    }

    #[test]
    fn only_takeover_changes_need_a_service_restart() {
        let s = Settings::default();
        let mut winh = s.clone();
        winh.winh_enabled = true;
        assert!(needs_restart(&s, &winh));
        // 快捷键、文字开关都是下一次听写就生效
        let mut key = s.clone();
        key.hotkey_vk = 0x77;
        key.text.drop_final_period = true;
        assert!(!needs_restart(&s, &key));
    }

    #[test]
    fn hotword_dialog_fields_become_one_line() {
        assert_eq!(hotword_line(" 雪耳 ", ""), Ok("雪耳".to_owned()));
        assert_eq!(
            hotword_line("Claude Code", " Cloud Code "),
            Ok("Cloud Code=Claude Code".to_owned())
        );
        assert!(hotword_line("", "").is_err());
        assert!(hotword_line("  ", "听错的").is_err());
        assert!(hotword_line("a=b", "").is_err());
        assert!(hotword_line("对", "错=了").is_err());
        assert!(hotword_line("#注释", "").is_err());
        assert!(hotword_line("一样", "一样").is_err());
        assert!(hotword_line(&"长".repeat(41), "").is_err());
    }

    #[test]
    fn history_menu_lists_recent_sentences_shortened() {
        assert_eq!(history_label("今天天气不错。"), "今天天气不错。");
        assert_eq!(history_label("第一行\n  第二行"), "第一行 第二行");
        let long = "这".repeat(40);
        let label = history_label(&long);
        assert_eq!(label.chars().count(), 24);
        assert!(label.ends_with('…'));
        // 最新的在最上面；点哪条就拿到哪条的全文
        let texts = vec!["最新".to_owned(), long.clone(), "最旧".to_owned()];
        let c = history_choices(&texts, &Settings::default());
        assert_eq!(c[0].1, "最新");
        assert_eq!(history_pick(&texts, c[1].0), Some(long.as_str()));
        assert_eq!(history_pick(&texts, c[2].0), Some("最旧"));
        assert_eq!(c[3], (0, String::new(), false));
        assert_eq!(c[4], (OPEN_HISTORY, "打开全部记录".to_owned(), false));
        assert_eq!(history_pick(&texts, OPEN_HISTORY), None);
        // 没有记录时说一声
        let empty = history_choices(&[], &Settings::default());
        assert_eq!(empty[0], (NOTHING, "还没有记录".to_owned(), false));
        assert_eq!(history_pick(&[], NOTHING), None);
    }

    #[test]
    fn hotword_menu_has_the_builtin_switch() {
        let s = Settings::default();
        let c = hotword_choices(&s);
        assert!(find(&c, "使用自带热词").1);
        assert!(c.iter().any(|(id, _, _)| *id == ADD_HOTWORD));
        assert!(c.iter().any(|(id, _, _)| *id == OPEN_HOTWORDS));
    }

    #[test]
    fn rows_show_a_short_summary() {
        assert_eq!(hotword_summary(""), "添加");
        assert_eq!(hotword_summary("# 说明\n\n"), "添加");
        assert_eq!(
            hotword_summary("# 说明\n雪耳\nClaude Code\nCloud Code=Claude Code\n"),
            "3 个"
        );
        let mut s = Settings::default();
        assert_eq!(history_summary(&s), "最近 7 天");
        s.history_keep_days = 30;
        assert_eq!(history_summary(&s), "最近 30 天");
        s.history_enabled = false;
        assert_eq!(history_summary(&s), "已关闭");
    }

    #[test]
    fn tray_menu_shows_state_and_plain_actions() {
        let c = tray_choices("待机", 0xA5, true, false, 1, 2, 3);
        assert_eq!(
            c[0],
            (NOTHING, "待机 · 点一下 右 Alt 说话".to_owned(), false)
        );
        assert_eq!(c[1].1, "");
        let names: Vec<&str> = c.iter().map(|(_, t, _)| t.as_str()).collect();
        assert_eq!(names[2..], ["打开控制面板", "暂停雪耳", "", "退出"]);
        // 没开时：开启雪耳；正在切换时不给开关
        let off = tray_choices("未开启", 0xA5, false, false, 1, 2, 3);
        assert_eq!(off[0].1, "未开启");
        assert!(off.iter().any(|(id, t, _)| *id == 2 && t == "开启雪耳"));
        let busy = tray_choices("正在开启", 0xA5, false, true, 1, 2, 3);
        assert!(!busy.iter().any(|(id, _, _)| *id == 2));
    }

    #[test]
    fn diagnostics_have_the_essentials_and_no_user_folder() {
        let log = (1..=60)
            .map(|i| format!("第 {i} 行"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n打不开 C:\\Users\\someone\\AppData\\Local\\XueerEar\\history.jsonl";
        let text = diagnostics(
            "1.2.0",
            "11 25H2（26200.7462）",
            &Settings::default(),
            "聆听中",
            &log,
            &[
                ("USERPROFILE", "C:\\Users\\someone"),
                ("LOCALAPPDATA", "C:\\Users\\someone\\AppData\\Local"),
            ],
        );
        assert!(text.starts_with("雪耳 1.2.0\nWindows 11 25H2"), "{text}");
        assert!(
            text.contains("快捷键 右 Alt；接管 Win+H 关；管理员身份 关"),
            "{text}"
        );
        assert!(
            text.contains("识别 豆包为主（微信备用）；降噪 开；调小别的声音 开"),
            "{text}"
        );
        assert!(
            text.contains("语音指令开着 4 个；指令前停 1 秒；指令后等 0.6 秒；停 3 秒收句；10 秒没新字结束；一次最长听 30 分钟"),
            "{text}"
        );
        assert!(text.contains("后台：聆听中"), "{text}");
        assert!(
            !text.contains("第 10 行\n") && text.contains("第 12 行"),
            "只带最后 50 行"
        );
        assert!(text.contains("%LOCALAPPDATA%\\XueerEar"), "{text}");
        assert!(!text.contains("someone"), "{text}");
    }

    #[test]
    fn every_result_still_validates() {
        let s = Settings::default();
        for (id, _, _) in choices(&s) {
            if let Some(next) = toggle(&s, id) {
                next.validate().unwrap();
            }
        }
        for (id, _, _) in key_choices(&s) {
            pick_key(&s, id).unwrap().validate().unwrap();
        }
    }

    #[test]
    fn a_new_rule_reports_how_many_past_sentences_it_would_change() {
        let history = vec![
            "雪而好用".to_owned(),
            "我在用雪而写代码".to_owned(),
            "今天天气不错".to_owned(),
        ];
        assert_eq!(rule_impact("雪而=雪耳", &history), 2);
        assert_eq!(rule_impact("克劳德=Claude", &history), 0);
    }
}
