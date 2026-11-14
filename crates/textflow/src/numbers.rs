//! 数字规范：只改「明确是数」的中文数字，宁可漏改也不误改。
//!
//! 会改的：带位（十百千万亿）且至少两个字的数（三百二十五、一万两千、十一）、
//! 三位以上逐位念的数（二零二六年、一二三四）、小数（三点五）、百分数（百分之五十）。
//! 不改的：单个数字（一下、一些、第一次、两个人）、单个「十」（十分好）、
//! 以位开头的词（千万别、万一）、两位约数和叠词（五六个、一一）。

const DIGITS: &[(char, u64)] = &[
    ('零', 0),
    ('〇', 0),
    ('一', 1),
    ('二', 2),
    ('两', 2),
    ('三', 3),
    ('四', 4),
    ('五', 5),
    ('六', 6),
    ('七', 7),
    ('八', 8),
    ('九', 9),
];
const SMALL_UNITS: &[(char, u64)] = &[('十', 10), ('百', 100), ('千', 1000)];
const BIG_UNITS: &[(char, u64)] = &[('万', 10_000), ('亿', 100_000_000)];

fn digit(c: char) -> Option<u64> {
    DIGITS.iter().find(|(d, _)| *d == c).map(|(_, v)| *v)
}
fn small_unit(c: char) -> Option<u64> {
    SMALL_UNITS.iter().find(|(d, _)| *d == c).map(|(_, v)| *v)
}
fn big_unit(c: char) -> Option<u64> {
    BIG_UNITS.iter().find(|(d, _)| *d == c).map(|(_, v)| *v)
}
fn is_num_char(c: char) -> bool {
    digit(c).is_some() || small_unit(c).is_some() || big_unit(c).is_some()
}

/// 把中文数字改成阿拉伯数字（保守规则，见模块说明）。
pub fn normalize_numbers(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if starts_with(&chars[i..], "百分之")
            && let Some((num, used)) = read_number(&chars[i + 3..], true)
        {
            out.push_str(&num);
            out.push('%');
            i += 3 + used;
            continue;
        }
        if is_num_char(chars[i])
            && (i == 0 || !is_num_char(chars[i - 1]))
            && let Some((num, used)) = read_number(&chars[i..], false)
        {
            out.push_str(&num);
            i += used;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn starts_with(chars: &[char], prefix: &str) -> bool {
    let mut it = chars.iter();
    prefix.chars().all(|p| it.next() == Some(&p))
}

/// 从开头读一个数；`force` 时单个数字也算（百分之五）。返回（阿拉伯数字, 用掉的字数）。
fn read_number(chars: &[char], force: bool) -> Option<(String, usize)> {
    let run = chars.iter().take_while(|c| is_num_char(**c)).count();
    if run == 0 {
        return None;
    }
    let int = &chars[..run];
    let frac_len = chars[run..]
        .iter()
        .skip(1)
        .take_while(|c| digit(**c).is_some() && **c != '两')
        .count();
    let is_decimal = chars.get(run) == Some(&'点') && frac_len > 0;
    // 小数和百分数的整数部分可以只有一位（三点五、百分之五）
    let mut text = if (is_decimal || force) && run == 1 && digit(int[0]).is_some() {
        char::from(b'0' + digit(int[0])? as u8).to_string()
    } else {
        integer_text(int)?
    };
    if !is_decimal && !force && run < 2 {
        return None;
    }
    let mut used = run;
    if is_decimal {
        text.push('.');
        for c in &chars[run + 1..run + 1 + frac_len] {
            text.push(char::from(b'0' + digit(*c)? as u8));
        }
        used += 1 + frac_len;
    }
    Some((text, used))
}

fn integer_text(int: &[char]) -> Option<String> {
    let has_unit = int
        .iter()
        .any(|c| small_unit(*c).is_some() || big_unit(*c).is_some());
    if !has_unit {
        // 逐位念：二零二六 → 2026；「两」不按逐位念（两两、两三个都不是数）
        // 少于三位的（五六个、一一检查）多半是约数或叠词，不改
        if int.len() < 3 || int.contains(&'两') {
            return None;
        }
        return int
            .iter()
            .map(|c| digit(*c).map(|d| char::from(b'0' + d as u8)))
            .collect();
    }
    // 必须以数字或「十」开头：千万别、万一、百万 这些是词不是数
    if digit(int[0]).is_none() && int[0] != '十' {
        return None;
    }
    with_units(int).map(|v| v.to_string())
}

/// 带位的数：三百二十五、一万两千、十一。省略写法（一万二 = 12000）按口语补位。
fn with_units(int: &[char]) -> Option<u64> {
    let mut total = 0u64;
    let mut section = 0u64;
    let mut pending: Option<u64> = None;
    let mut last_unit = 1u64;
    for &c in int {
        if let Some(d) = digit(c) {
            if pending.is_some() && d != 0 {
                return None;
            }
            pending = if d == 0 { None } else { Some(d) };
            if d == 0 {
                last_unit = 1;
            }
        } else if let Some(u) = small_unit(c) {
            section += pending.take().unwrap_or(1) * u;
            last_unit = u;
        } else if let Some(u) = big_unit(c) {
            section += pending.take().unwrap_or(0);
            total = (total + section) * u;
            section = 0;
            last_unit = u;
        }
    }
    if let Some(d) = pending {
        // 「一万二」「三百五」：末位数字省了位，按上一个位的十分之一补
        let implied = if last_unit >= 10 && !int.ends_with(&['零']) && int.len() >= 3 {
            last_unit / 10
        } else {
            1
        };
        section += d * implied;
    }
    Some(total + section)
}
