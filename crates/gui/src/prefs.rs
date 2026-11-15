//! 「文字与历史」菜单：选项列表和点选后的设置变化（纯逻辑，可在 WSL 测）。
use xueer_ear_backend::settings::Settings;

pub const OPEN_HISTORY: usize = 1100;
pub const OPEN_HOTWORDS: usize = 1101;
const BASE: usize = 1110;
const DAYS: [u32; 4] = [1, 7, 30, 90];

type Switch = (&'static str, fn(&mut Settings) -> &mut bool);

/// 开关类选项，顺序就是菜单顺序。
const SWITCHES: [Switch; 8] = [
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
    ("快捷键模式（按住右 Alt）", |s| {
        &mut s.hotkey_enabled
    }),
    ("记录历史", |s| &mut s.history_enabled),
];

/// 菜单项：（命令 ID, 文字, 是否打勾）
pub fn choices(s: &Settings) -> Vec<(usize, String, bool)> {
    let mut copy = s.clone();
    let mut out: Vec<_> = SWITCHES
        .iter()
        .enumerate()
        .map(|(i, (text, field))| (BASE + i, (*text).to_owned(), *field(&mut copy)))
        .collect();
    out.extend(DAYS.iter().enumerate().map(|(i, days)| {
        (
            BASE + SWITCHES.len() + i,
            format!("保留 {days} 天历史"),
            s.history_enabled && s.history_keep_days == *days,
        )
    }));
    out.push((OPEN_HISTORY, "查看历史记录".to_owned(), false));
    out.push((OPEN_HOTWORDS, "编辑热词表".to_owned(), false));
    out
}

/// 点了某项后的新设置；不是开关类（打开文件）或不认识的 ID 返回 None。
pub fn toggle(s: &Settings, id: usize) -> Option<Settings> {
    let index = id.checked_sub(BASE)?;
    let mut next = s.clone();
    if let Some((_, field)) = SWITCHES.get(index) {
        let value = field(&mut next);
        *value = !*value;
    } else {
        next.history_keep_days = *DAYS.get(index - SWITCHES.len())?;
        next.history_enabled = true;
    }
    Some(next)
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
        assert!(find(&c, "快捷键模式（按住右 Alt）").1);
        assert!(find(&c, "保留 7 天历史").1);
        assert!(!find(&c, "保留 30 天历史").1);
        assert!(c.iter().any(|(id, _, _)| *id == OPEN_HISTORY));
        assert!(c.iter().any(|(id, _, _)| *id == OPEN_HOTWORDS));
    }

    #[test]
    fn each_switch_flips_only_itself() {
        let s = Settings::default();
        for (id, text, on) in choices(&s) {
            if id == OPEN_HISTORY || id == OPEN_HOTWORDS || text.starts_with("保留") {
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
    fn history_days_and_off() {
        let s = Settings::default();
        let id = find(&choices(&s), "保留 30 天历史").0;
        let next = toggle(&s, id).unwrap();
        assert_eq!(next.history_keep_days, 30);
        assert!(next.history_enabled);
        let off = find(&choices(&s), "记录历史").0;
        assert!(!toggle(&s, off).unwrap().history_enabled);
        assert!(toggle(&s, OPEN_HISTORY).is_none());
        assert!(toggle(&s, 9999).is_none());
    }

    #[test]
    fn every_result_still_validates() {
        let s = Settings::default();
        for (id, _, _) in choices(&s) {
            if let Some(next) = toggle(&s, id) {
                next.validate().unwrap();
            }
        }
    }
}
