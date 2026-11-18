//! 话没说完被停顿切断：主人说到「然后我的」停了一下，云端就当一句说完、补上句号，
//! 下一句「耳机放在左下方」接上时中间就多了句号（主人 2026-11-19 历史复查）。
//! 这里只认句尾明显悬着的连接词、介词、代词，正常说完的句子不动。

/// 这些词收尾的话一定没说完。故意不收「的」「是」「在」「这个」：「确实是有效果的」「不是」「情况在」「不给我显示这个」都是完整的。
const DANGLING: &[&str] = &[
    "然后我的",
    "然后我",
    "然后",
    "就是",
    "但是",
    "可是",
    "所以",
    "因为",
    "如果",
    "而且",
    "或者",
    "还有",
    "当我",
    "比方说",
    "也就是",
    "这个",
    "那个",
    "我的",
    "你的",
    "一个",
    "设为和",
    "把",
    "被",
    "给",
    "跟",
];

/// 句尾是不是悬着（不看句尾标点）
pub fn is_unfinished(text: &str) -> bool {
    let body = text.trim_end().trim_end_matches(['。', '.']);
    body.chars().count() >= 2 && DANGLING.iter().any(|w| body.ends_with(w))
}

/// 没说完的句子去掉云端补的句号
pub fn drop_false_full_stop(text: &str) -> String {
    if is_unfinished(text) {
        text.trim_end().trim_end_matches(['。', '.']).to_string()
    } else {
        text.to_string()
    }
}
