//! 控制面板各行的菜单：选项列表、点选后的设置变化、行上显示的摘要（纯逻辑，可在 WSL 测）。
//! 菜单项是（命令号, 文字, 是否打勾）；命令号 0 且文字为空表示分隔线。
use xueer_ear_backend::settings::Settings;
use xueer_ear_textflow::Hotwords;

/// 点了没反应的占位项（「还没有记录」）
pub const NOTHING: usize = 1099;
pub const OPEN_HISTORY: usize = 1100;
pub const OPEN_HOTWORDS: usize = 1101;
pub const ADD_HOTWORD: usize = 1102;
const BASE: usize = 1110;
const KEY_BASE: usize = 1200;
const HISTORY_BASE: usize = 1300;
/// 历史菜单里列最近多少句
pub const HISTORY_SHOWN: usize = 10;
const DAYS: [u32; 4] = [1, 7, 30, 90];
/// 快捷键听写一次最长听多久（分钟）
const LISTEN: [u32; 4] = [10, 30, 60, 120];
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
    ("说「发送」时按回车", |s| &mut s.text.allow_send),
    ("终端里说「换行」发 Ctrl+J", |s| {
        &mut s.text.terminal_newline
    }),
    ("提示音", |s| &mut s.sounds_enabled),
    ("同时接管 Win+H", |s| &mut s.winh_enabled),
    ("记录历史", |s| &mut s.history_enabled),
];
/// 文字整理 7 个、提示音和 Win+H 2 个；「记录历史」和保留天数一组
const GROUPS: [usize; 2] = [7, 9];

fn separator() -> Choice {
    (0, String::new(), false)
}

/// 「更多设置」菜单
pub fn choices(s: &Settings) -> Vec<Choice> {
    let mut copy = s.clone();
    let mut out = Vec::new();
    for (i, (text, field)) in SWITCHES.iter().enumerate() {
        if GROUPS.contains(&i) {
            out.push(separator());
        }
        out.push((BASE + i, (*text).to_owned(), *field(&mut copy)));
    }
    out.extend(DAYS.iter().enumerate().map(|(i, days)| {
        (
            BASE + SWITCHES.len() + i,
            format!("保留 {days} 天历史"),
            s.history_enabled && s.history_keep_days == *days,
        )
    }));
    out.push(separator());
    let listen_base = BASE + SWITCHES.len() + DAYS.len();
    out.extend(LISTEN.iter().enumerate().map(|(i, minutes)| {
        (
            listen_base + i,
            format!("一次最长听 {minutes} 分钟"),
            s.max_listen_minutes == *minutes,
        )
    }));
    out
}

/// 点了「更多设置」里某项后的新设置；不认识的命令号返回 None。
pub fn toggle(s: &Settings, id: usize) -> Option<Settings> {
    let index = id.checked_sub(BASE)?;
    let mut next = s.clone();
    if let Some((_, field)) = SWITCHES.get(index) {
        let value = field(&mut next);
        *value = !*value;
    } else if let Some(days) = DAYS.get(index - SWITCHES.len()) {
        next.history_keep_days = *days;
        next.history_enabled = true;
    } else {
        next.max_listen_minutes = *LISTEN.get(index - SWITCHES.len() - DAYS.len())?;
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

/// 「热词」那一行显示：有几个词（含替换规则），一个都没有就显示「添加」
pub fn hotword_summary(file: &str) -> String {
    let hotwords = Hotwords::parse(file);
    match hotwords.words().len() + hotwords.rules().len() {
        0 => "添加".to_owned(),
        count => format!("{count} 个"),
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

/// 「历史」菜单：最近说过的话（最新在上，点一句就复制），最后是打开全部记录
pub fn history_choices(texts: &[String]) -> Vec<Choice> {
    if texts.is_empty() {
        return vec![(NOTHING, "还没有记录".to_owned(), false)];
    }
    let mut out: Vec<_> = texts
        .iter()
        .enumerate()
        .map(|(i, text)| (HISTORY_BASE + i, history_label(text), false))
        .collect();
    out.push(separator());
    out.push((OPEN_HISTORY, "打开全部记录".to_owned(), false));
    out
}

/// 点了历史菜单里的哪一句（全文）
pub fn history_pick(texts: &[String], id: usize) -> Option<&str> {
    texts.get(id.checked_sub(HISTORY_BASE)?).map(String::as_str)
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
        assert!(!find(&c, "说「发送」时按回车").1);
        assert!(!find(&c, "同时接管 Win+H").1);
        assert!(find(&c, "提示音").1);
        assert!(find(&c, "保留 7 天历史").1);
        assert!(!find(&c, "保留 30 天历史").1);
        // 快捷键、热词、历史各有自己的一行，不在「更多设置」里
        assert!(!c.iter().any(|(_, t, _)| t.contains("快捷键")));
        assert!(!c.iter().any(|(_, t, _)| t.contains("热词")));
    }

    #[test]
    fn each_switch_flips_only_itself() {
        let s = Settings::default();
        for (id, text, on) in choices(&s) {
            if text.is_empty() || text.starts_with("保留") || text.starts_with("一次最长听")
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
        assert_eq!(separators, 3);
        assert!(c.iter().all(|(id, t, _)| t.is_empty() == (*id == 0)));
        assert!(!c.first().unwrap().1.is_empty() && !c.last().unwrap().1.is_empty());
        assert!(toggle(&s, 0).is_none());
    }

    #[test]
    fn history_days_and_off() {
        let s = Settings::default();
        let id = find(&choices(&s), "保留 30 天历史").0;
        let next = toggle(&s, id).unwrap();
        assert_eq!(next.history_keep_days, 30);
        assert!(next.history_enabled);
        let off = find(&choices(&s), "记录历史").0;
        assert!(!toggle(&s, off).unwrap().history_enabled);
        assert!(toggle(&s, 9999).is_none());
    }

    #[test]
    fn listen_length_choice() {
        let s = Settings::default();
        assert!(find(&choices(&s), "一次最长听 30 分钟").1);
        let id = find(&choices(&s), "一次最长听 120 分钟").0;
        assert_eq!(toggle(&s, id).unwrap().max_listen_minutes, 120);
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
        key.text.allow_send = true;
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
        let c = history_choices(&texts);
        assert_eq!(c[0].1, "最新");
        assert_eq!(history_pick(&texts, c[1].0), Some(long.as_str()));
        assert_eq!(history_pick(&texts, c[2].0), Some("最旧"));
        assert_eq!(c[3], (0, String::new(), false));
        assert_eq!(c[4], (OPEN_HISTORY, "打开全部记录".to_owned(), false));
        assert_eq!(history_pick(&texts, OPEN_HISTORY), None);
        // 没有记录时说一声
        let empty = history_choices(&[]);
        assert_eq!(empty, [(NOTHING, "还没有记录".to_owned(), false)]);
        assert_eq!(history_pick(&[], NOTHING), None);
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
}
