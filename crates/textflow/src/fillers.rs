//! 口癖过滤：只删句首或单独成句的口癖，句中一律不动。
//!
//! 「嗯、呃、额」几乎不会是正经词，句首见到就删（「嗯哼」除外）；
//! 「那个、就是说」常常是正经词（「那个文件」「就是说服」），没有分词没法区分，
//! 所以只有后面紧跟标点或另一个口癖时才删。

const STRONG: &[&str] = &["嗯", "呃", "额"];
const WEAK: &[&str] = &["那个", "就是说", "然后呢"];
const SOFT_PUNCT: &[char] = &['，', ',', '、', ' ', '…'];
const TERMINATORS: &[char] = &['。', '！', '？', '!', '?', '；', ';', '\n'];

/// 删掉句首或单独成句的口癖（嗯、呃、那个、就是说……）。
pub fn strip_fillers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for sentence in split_keep_terminator(text) {
        let body_end = sentence.trim_end_matches(TERMINATORS).len();
        let (body, end) = sentence.split_at(body_end);
        let body = strip_leading(body);
        if body.trim_matches(SOFT_PUNCT).is_empty() {
            continue;
        }
        out.push_str(body);
        out.push_str(end);
    }
    out
}

fn split_keep_terminator(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if TERMINATORS.contains(&c) {
            let mut end = i + c.len_utf8();
            while let Some(&(j, d)) = chars.peek() {
                if !TERMINATORS.contains(&d) {
                    break;
                }
                end = j + d.len_utf8();
                chars.next();
            }
            parts.push(&text[start..end]);
            start = end;
        }
    }
    if start < text.len() {
        parts.push(&text[start..]);
    }
    parts
}

/// 反复剥掉句首口癖，返回剩下的部分。
fn strip_leading(mut s: &str) -> &str {
    loop {
        let before = s;
        if let Some(rest) = STRONG.iter().find_map(|f| s.strip_prefix(f)) {
            if !rest.starts_with('哼') {
                s = rest.trim_start_matches(SOFT_PUNCT);
            }
        } else if let Some(rest) = WEAK.iter().find_map(|f| s.strip_prefix(f)) {
            if rest.starts_with(SOFT_PUNCT) || rest.is_empty() {
                s = rest.trim_start_matches(SOFT_PUNCT);
            } else if strip_leading(rest) != rest {
                s = rest;
            }
        }
        if s == before {
            return s;
        }
    }
}
