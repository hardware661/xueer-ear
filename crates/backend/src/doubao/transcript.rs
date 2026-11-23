//! 豆包一句话里陆续给的识别结果，拼成这句现在的全文。
//!
//! 结果是 `{"results":[...]}`。老格式每条带 `index`（服务端按停顿分的小句）：同一小句会被反复改写，
//! 定稿（`extra.nonstream_result`，或 `is_interim=false` 且 `is_vad_finished`）以后还可能再被二遍识别改一次。
//! 新格式不带 `index`，`results[0]` 就是到目前为止的整段文字。展示 = 已定稿的小句按顺序 + 正在说的那一小句。
use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub struct Transcript {
    sentences: BTreeMap<i64, String>,
    current: String,
    next_index: i64,
}

impl Transcript {
    /// 吃一条结果 JSON。全文变了就返回现在的全文；心跳、没变化返回 None。
    pub fn update(&mut self, json: &str) -> Option<String> {
        let value: serde_json::Value = serde_json::from_str(json).ok()?;
        let results = value.get("results")?.as_array()?;
        let before = self.text();
        if results.iter().all(|r| r.get("index").is_none()) {
            let text = results.first()?.get("text")?.as_str()?;
            self.sentences.clear();
            self.current = text.to_string();
        } else {
            for result in results {
                self.apply(result);
            }
        }
        let after = self.text();
        (after != before).then_some(after)
    }

    fn apply(&mut self, result: &serde_json::Value) {
        let Some(text) = result.get("text").and_then(|t| t.as_str()) else {
            return;
        };
        let index = result.get("index").and_then(|i| i.as_i64());
        let finished = result["extra"]["nonstream_result"].as_bool() == Some(true)
            || (result["is_interim"].as_bool() == Some(false)
                && result["is_vad_finished"].as_bool() == Some(true));
        match index {
            // 已经定稿的小句又来了新结果（二遍识别改字）：就地改，不要当成新的一句再接一遍
            Some(index) if self.sentences.contains_key(&index) => {
                self.sentences.insert(index, text.to_string());
            }
            _ if finished => {
                let index = index.unwrap_or(self.next_index);
                self.sentences.insert(index, text.to_string());
                self.next_index = self.next_index.max(index + 1);
                self.current.clear();
            }
            _ => self.current = text.to_string(),
        }
    }

    /// 这句现在的全文
    pub fn text(&self) -> String {
        let mut out = String::new();
        for part in self
            .sentences
            .values()
            .map(String::as_str)
            .chain([self.current.as_str()])
        {
            join_into(&mut out, part);
        }
        out
    }
}

/// 接上一段：中文直接接；前面以英文字母、数字或英文标点结尾，后面以英文字母或数字开头时，中间补一个空格
fn join_into(out: &mut String, part: &str) {
    let part = part.trim();
    if part.is_empty() {
        return;
    }
    if let (Some(last), Some(first)) = (out.chars().last(), part.chars().next())
        && (last.is_ascii_alphanumeric() || ".,!?;:".contains(last))
        && first.is_ascii_alphanumeric()
    {
        out.push(' ');
    }
    out.push_str(part);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interims_grow_then_the_final_replaces_them() {
        let mut t = Transcript::default();
        assert_eq!(
            t.update(r#"{"results":[{"text":"今天","index":0,"is_interim":true}]}"#),
            Some("今天".into())
        );
        assert_eq!(
            t.update(r#"{"results":[{"text":"今天天气","index":0,"is_interim":true}]}"#),
            Some("今天天气".into())
        );
        assert_eq!(
            t.update(
                r#"{"results":[{"text":"今天天气很好。","index":0,"is_interim":false,"is_vad_finished":true}]}"#
            ),
            Some("今天天气很好。".into())
        );
        assert_eq!(t.text(), "今天天气很好。");
    }

    #[test]
    fn finished_sentences_and_the_open_one_are_joined_in_order() {
        let mut t = Transcript::default();
        t.update(
            r#"{"results":[{"text":"第一句。","index":0,"extra":{"nonstream_result":true}}]}"#,
        );
        t.update(r#"{"results":[{"text":"第二","index":1,"is_interim":true}]}"#);
        assert_eq!(t.text(), "第一句。第二");
    }

    #[test]
    fn a_two_pass_revision_rewrites_its_sentence_instead_of_repeating_it() {
        let mut t = Transcript::default();
        t.update(r#"{"results":[{"text":"天器不错。","index":0,"is_interim":false,"is_vad_finished":true}]}"#);
        t.update(r#"{"results":[{"text":"天气不错。","index":0,"is_interim":false}]}"#);
        assert_eq!(t.text(), "天气不错。");
    }

    #[test]
    fn the_newer_whole_text_format_replaces_everything() {
        let mut t = Transcript::default();
        t.update(r#"{"results":[{"text":"老的","index":0,"is_interim":true}]}"#);
        assert_eq!(
            t.update(r#"{"results":[{"text":"整段的新结果"},{"text":"候选"}]}"#),
            Some("整段的新结果".into())
        );
    }

    #[test]
    fn heartbeats_and_repeats_change_nothing() {
        let mut t = Transcript::default();
        assert_eq!(t.update(r#"{"extra":{"packet_number":3}}"#), None);
        assert_eq!(t.update("not json"), None);
        t.update(r#"{"results":[{"text":"你好","index":0,"is_interim":true}]}"#);
        assert_eq!(
            t.update(r#"{"results":[{"text":"你好","index":0,"is_interim":true}]}"#),
            None
        );
    }

    #[test]
    fn english_sentences_get_a_space_between_them() {
        let mut t = Transcript::default();
        t.update(
            r#"{"results":[{"text":"Hello there.","index":0,"extra":{"nonstream_result":true}}]}"#,
        );
        t.update(r#"{"results":[{"text":"How are you","index":1,"is_interim":true}]}"#);
        assert_eq!(t.text(), "Hello there. How are you");
    }
}
