//! 热词纠错：识别结果里和热词「读音相同」的片段换成热词本身。
//!
//! - 汉字按拼音比（含多音字），平翘舌（z/zh c/ch s/sh）和前后鼻音（an/ang en/eng in/ing）算同音；
//! - 英文和数字不分大小写、忽略中间空格（new api → New API），两头不能紧贴别的字母数字；
//! - 汉字部分至少两个字才做模糊匹配，单字热词只认原字，避免满屏误改；
//! - 同一位置多个热词都能对上时：最长的优先，一样长时原文本来就是的那个优先，再按文件顺序。
//!
//! 文件格式一行一个词，兼容雾凇 `custom_phrase.txt`（词<TAB>编码<TAB>权重，`#` 开头是注释）。
//!
//! 读音纠不了的固定错法用「错=对」一行写成替换规则（`Cloud Code=Claude Code`、`温加H=Win+H`）：
//! 英文不分大小写、要求整词（`out=Alt` 不会动 output）；规则先于热词执行，替换出来的字不再被改写。

use pinyin::ToPinyinMulti;

#[derive(Debug, Clone, Default)]
pub struct Hotwords {
    entries: Vec<Entry>,
    rules: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
struct Entry {
    word: String,
    units: Vec<Unit>,
    han_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
enum Unit {
    /// 一个汉字：原字 + 所有读音（已做模糊归一）
    Han(char, Vec<String>),
    /// 一个英文字母或数字（小写）
    Latin(char),
    /// 其他字符，只认原字
    Other(char),
}

impl Hotwords {
    pub fn parse(file: &str) -> Self {
        let mut entries: Vec<Entry> = Vec::new();
        let mut rules: Vec<(String, String)> = Vec::new();
        for line in file.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((wrong, right)) = line.split_once('=') {
                let (wrong, right) = (wrong.trim(), right.trim());
                if !wrong.is_empty() && !right.is_empty() && !rules.iter().any(|(w, _)| w == wrong)
                {
                    rules.push((wrong.to_string(), right.to_string()));
                }
                continue;
            }
            let word = line.split('\t').next().unwrap_or("").trim();
            if word.is_empty() || entries.iter().any(|e| e.word == word) {
                continue;
            }
            let units: Vec<Unit> = units_of(word)
                .into_iter()
                .map(|(u, _)| u)
                .filter(|u| *u != SPACE)
                .collect();
            let han_count = units.iter().filter(|u| matches!(u, Unit::Han(..))).count();
            entries.push(Entry {
                word: word.to_string(),
                units,
                han_count,
            });
        }
        // 长的规则先试，免得「Cloud」抢在「Cloud Code」前面
        rules.sort_by_key(|(wrong, _)| std::cmp::Reverse(wrong.chars().count()));
        Self { entries, rules }
    }

    /// 替换规则（错, 对）
    pub fn rules(&self) -> Vec<(&str, &str)> {
        let mut listed: Vec<_> = self
            .rules
            .iter()
            .map(|(w, r)| (w.as_str(), r.as_str()))
            .collect();
        listed.sort();
        listed
    }

    pub fn words(&self) -> Vec<&str> {
        self.entries.iter().map(|e| e.word.as_str()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.rules.is_empty()
    }

    /// 先按替换规则切开，规则换出来的片段原样保留，其余片段再做读音热词纠错。
    pub fn apply(&self, text: &str) -> String {
        if self.rules.is_empty() {
            return self.apply_words(text);
        }
        let mut out = String::with_capacity(text.len());
        let mut plain_from = 0;
        let mut i = 0;
        while i < text.len() {
            let hit = self
                .rules
                .iter()
                .find_map(|(wrong, right)| rule_match(text, i, wrong).map(|end| (end, right)));
            match hit {
                Some((end, right)) => {
                    out.push_str(&self.apply_words(&text[plain_from..i]));
                    out.push_str(right);
                    // 被换掉的英文后面那个空格，原本是隔开英文和汉字用的；换成汉字收尾后，
                    // 后面紧跟汉字时这个空格就多余了（「雪儿 DAR 里面」→「雪耳里面」）
                    let mut end = end;
                    let rest = &text[end..];
                    if right.chars().next_back().is_some_and(is_han)
                        && rest.starts_with(' ')
                        && rest[1..].chars().next().is_some_and(is_han)
                    {
                        end += 1;
                    }
                    i = end;
                    plain_from = end;
                }
                None => i += text[i..].chars().next().map_or(1, char::len_utf8),
            }
        }
        out.push_str(&self.apply_words(&text[plain_from..]));
        out
    }

    fn apply_words(&self, text: &str) -> String {
        if self.entries.is_empty() {
            return text.to_string();
        }
        let units = units_of(text);
        let mut out = String::with_capacity(text.len());
        let mut copied = 0; // text 中已经输出到的字节位置
        let mut i = 0;
        while i < units.len() {
            if units[i].0 == SPACE {
                i += 1;
                continue;
            }
            match self.best_match(text, &units, i) {
                Some((entry, end)) => {
                    let start_byte = units[i].1.start;
                    let end_byte = units[end - 1].1.end;
                    out.push_str(&text[copied..start_byte]);
                    out.push_str(&entry.word);
                    copied = end_byte;
                    i = end;
                }
                None => i += 1,
            }
        }
        out.push_str(&text[copied..]);
        out
    }

    /// 在 text 的第 i 个单元起找最合适的热词，返回（热词, 匹配结束的单元下标）。
    fn best_match(&self, text: &str, units: &[(Unit, Span)], i: usize) -> Option<(&Entry, usize)> {
        let mut best: Option<(&Entry, usize, bool)> = None;
        for entry in &self.entries {
            let Some(end) = match_at(entry, units, i) else {
                continue;
            };
            let exact = text[units[i].1.start..units[end - 1].1.end] == entry.word;
            let better = match best {
                None => true,
                Some((b, _, b_exact)) => {
                    entry.units.len() > b.units.len()
                        || (entry.units.len() == b.units.len() && exact && !b_exact)
                }
            };
            if better {
                best = Some((entry, end, exact));
            }
        }
        best.map(|(e, end, _)| (e, end))
    }
}

/// 往热词表里加一行（一个词，或「错=对」），返回新的文件内容；不用改就返回 None。
/// 已有的词不重复加；同一个「错」已有规则时换成新的「对」；原有的注释和顺序不动。
pub fn add_hotword_line(file: &str, line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') || line.contains(['\n', '\r', '\t']) {
        return None;
    }
    let normalized = match line.split_once('=') {
        Some((wrong, right)) => {
            let (wrong, right) = (wrong.trim(), right.trim());
            if wrong.is_empty() || right.is_empty() {
                return None;
            }
            format!("{wrong}={right}")
        }
        None => line.to_string(),
    };
    let key = |l: &str| -> String {
        let l = l.trim();
        match l.split_once('=') {
            Some((wrong, _)) => format!("{}=", wrong.trim()),
            None => l.split('\t').next().unwrap_or("").trim().to_string(),
        }
    };
    let new_key = key(&normalized);
    let mut lines: Vec<String> = file.lines().map(str::to_string).collect();
    let existing = lines
        .iter()
        .position(|l| !l.trim_start().starts_with('#') && key(l) == new_key);
    match existing {
        Some(i) => {
            let same = match lines[i].trim().split_once('=') {
                Some((w, r)) => format!("{}={}", w.trim(), r.trim()) == normalized,
                None => true,
            };
            if same {
                return None;
            }
            lines[i] = normalized;
        }
        None => lines.push(normalized),
    }
    Some(lines.join("\n") + "\n")
}

/// text 从字节位置 at 起是不是 wrong（英文不分大小写；wrong 以字母数字开头/结尾的那一侧要求整词）。
/// 是就返回匹配结束的字节位置。
fn rule_match(text: &str, at: usize, wrong: &str) -> Option<usize> {
    let mut rest = text[at..].chars();
    let mut end = at;
    for want in wrong.chars() {
        let got = rest.next()?;
        if !got.eq_ignore_ascii_case(&want) {
            return None;
        }
        end += got.len_utf8();
    }
    let word_char = |c: char| c.is_ascii_alphanumeric();
    if wrong.chars().next().is_some_and(word_char)
        && text[..at].chars().next_back().is_some_and(word_char)
    {
        return None;
    }
    if wrong.chars().next_back().is_some_and(word_char)
        && text[end..].chars().next().is_some_and(word_char)
    {
        return None;
    }
    Some(end)
}

#[derive(Debug, Clone)]
struct Span {
    start: usize,
    end: usize,
}

const SPACE: Unit = Unit::Other(' ');

fn match_at(entry: &Entry, units: &[(Unit, Span)], i: usize) -> Option<usize> {
    let first_latin = matches!(entry.units.first(), Some(Unit::Latin(_)));
    if first_latin && i > 0 && prev_non_space(units, i).is_some_and(|u| matches!(u, Unit::Latin(_)))
    {
        return None;
    }
    let mut j = i;
    for (k, want) in entry.units.iter().enumerate() {
        // 文字里的空格在热词内部跳过（new api 对 NewAPI 也算）
        while k > 0 && j < units.len() && units[j].0 == SPACE {
            j += 1;
        }
        let (got, _) = units.get(j)?;
        let ok = match (want, got) {
            (Unit::Han(wc, wp), Unit::Han(gc, gp)) => {
                wc == gc || (entry.han_count >= 2 && wp.iter().any(|p| gp.contains(p)))
            }
            (Unit::Latin(a), Unit::Latin(b)) => a == b,
            (Unit::Other(a), Unit::Other(b)) => a == b,
            _ => false,
        };
        if !ok {
            return None;
        }
        j += 1;
    }
    let last_latin = matches!(entry.units.last(), Some(Unit::Latin(_)));
    if last_latin
        && units
            .get(j)
            .is_some_and(|(u, _)| matches!(u, Unit::Latin(_)))
    {
        return None;
    }
    Some(j)
}

fn prev_non_space(units: &[(Unit, Span)], i: usize) -> Option<&Unit> {
    units[..i]
        .iter()
        .rev()
        .map(|(u, _)| u)
        .find(|u| **u != SPACE)
}

fn units_of(s: &str) -> Vec<(Unit, Span)> {
    let mut v = Vec::new();
    for (start, c) in s.char_indices() {
        let span = Span {
            start,
            end: start + c.len_utf8(),
        };
        let unit = if c.is_ascii_alphanumeric() {
            Unit::Latin(c.to_ascii_lowercase())
        } else if c.is_whitespace() {
            SPACE
        } else if let Some(multi) = c.to_pinyin_multi() {
            let mut readings: Vec<String> = multi.into_iter().map(|p| fuzzy(p.plain())).collect();
            readings.dedup();
            Unit::Han(c, readings)
        } else {
            Unit::Other(c)
        };
        v.push((unit, span));
    }
    v
}

/// 模糊音归一：zh→z ch→c sh→s，ang→an eng→en ing→in。
fn fuzzy(p: &str) -> String {
    let mut s = p.to_string();
    for (from, to) in [("zh", "z"), ("ch", "c"), ("sh", "s")] {
        if let Some(rest) = s.strip_prefix(from) {
            s = format!("{to}{rest}");
            break;
        }
    }
    for (from, to) in [("ang", "an"), ("eng", "en"), ("ing", "in")] {
        if let Some(head) = s.strip_suffix(from) {
            s = format!("{head}{to}");
            break;
        }
    }
    s
}

fn is_han(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{f900}'..='\u{faff}')
}
