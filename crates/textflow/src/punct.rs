//! 标点习惯，三项都默认关闭（默认不改识别结果）。

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PunctOptions {
    /// 去掉整段最后一个句号（中英文都算），问号叹号保留
    pub drop_final_period: bool,
    /// 中文和英文/数字之间加一个空格
    pub cjk_latin_space: bool,
    /// 紧挨中文的半角 , ? ! : ; 换成全角
    pub fullwidth_punct_in_cjk: bool,
}

pub fn apply_punctuation(text: &str, options: &PunctOptions) -> String {
    let mut s = text.to_string();
    if options.fullwidth_punct_in_cjk {
        s = fullwidth_near_cjk(&s);
    }
    if options.cjk_latin_space {
        s = space_cjk_latin(&s);
    }
    if options.drop_final_period
        && let Some(stripped) = s.strip_suffix('。').or_else(|| s.strip_suffix('.'))
    {
        s = stripped.to_string();
    }
    s
}

fn is_cjk(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{f900}'..='\u{faff}')
}

fn is_latin(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '%'
}

fn space_cjk_latin(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    let mut prev: Option<char> = None;
    for c in s.chars() {
        if let Some(p) = prev
            && ((is_cjk(p) && is_latin(c)) || (is_latin(p) && is_cjk(c)))
        {
            out.push(' ');
        }
        out.push(c);
        prev = Some(c);
    }
    out
}

fn fullwidth_near_cjk(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    chars
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let full = match c {
                ',' => '，',
                '?' => '？',
                '!' => '！',
                ':' => '：',
                ';' => '；',
                _ => return c,
            };
            let near_cjk =
                (i > 0 && is_cjk(chars[i - 1])) || chars.get(i + 1).is_some_and(|n| is_cjk(*n));
            if near_cjk { full } else { c }
        })
        .collect()
}
