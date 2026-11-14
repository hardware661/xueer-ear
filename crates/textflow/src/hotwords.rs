//! 热词纠错：识别结果里和热词「读音相同」的片段换成热词本身。
//!
//! - 汉字按拼音比（含多音字），平翘舌（z/zh c/ch s/sh）和前后鼻音（an/ang en/eng in/ing）算同音；
//! - 英文和数字不分大小写、忽略中间空格（new api → New API），两头不能紧贴别的字母数字；
//! - 汉字部分至少两个字才做模糊匹配，单字热词只认原字，避免满屏误改；
//! - 同一位置多个热词都能对上时：最长的优先，一样长时原文本来就是的那个优先，再按文件顺序。
//!
//! 文件格式一行一个词，兼容雾凇 `custom_phrase.txt`（词<TAB>编码<TAB>权重，`#` 开头是注释）。

use pinyin::ToPinyinMulti;

#[derive(Debug, Clone, Default)]
pub struct Hotwords {
    entries: Vec<Entry>,
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
        for line in file.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let word = line.split('\t').next().unwrap_or("").trim();
            if word.is_empty() || entries.iter().any(|e| e.word == word) {
                continue;
            }
            let units: Vec<Unit> =
                units_of(word).into_iter().map(|(u, _)| u).filter(|u| *u != SPACE).collect();
            let han_count = units.iter().filter(|u| matches!(u, Unit::Han(..))).count();
            entries.push(Entry { word: word.to_string(), units, han_count });
        }
        Self { entries }
    }

    pub fn words(&self) -> Vec<&str> {
        self.entries.iter().map(|e| e.word.as_str()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn apply(&self, text: &str) -> String {
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
            let Some(end) = match_at(entry, units, i) else { continue };
            let exact = &text[units[i].1.start..units[end - 1].1.end] == entry.word;
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

#[derive(Debug, Clone)]
struct Span {
    start: usize,
    end: usize,
}

const SPACE: Unit = Unit::Other(' ');

fn match_at(entry: &Entry, units: &[(Unit, Span)], i: usize) -> Option<usize> {
    let first_latin = matches!(entry.units.first(), Some(Unit::Latin(_)));
    if first_latin && i > 0 && prev_non_space(units, i).is_some_and(|u| matches!(u, Unit::Latin(_))) {
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
    if last_latin && units.get(j).is_some_and(|(u, _)| matches!(u, Unit::Latin(_))) {
        return None;
    }
    Some(j)
}

fn prev_non_space(units: &[(Unit, Span)], i: usize) -> Option<&Unit> {
    units[..i].iter().rev().map(|(u, _)| u).find(|u| **u != SPACE)
}

fn units_of(s: &str) -> Vec<(Unit, Span)> {
    let mut v = Vec::new();
    for (start, c) in s.char_indices() {
        let span = Span { start, end: start + c.len_utf8() };
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
