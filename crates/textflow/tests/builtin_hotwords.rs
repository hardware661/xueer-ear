//! 自带通用热词表：只放「错=对」规则，读音容易听错、写法固定的 AI/编程词。
use xueer_ear_textflow::{BUILTIN_HOTWORDS, Hotwords};

fn builtin() -> Hotwords {
    Hotwords::parse(BUILTIN_HOTWORDS)
}

#[test]
fn the_builtin_list_only_contains_rules() {
    // 按读音猜的词会把同音的正常说法改掉（试过：「雪儿」被改成「雪耳」），自带表一律写明错法
    let h = builtin();
    assert!(
        h.words().is_empty(),
        "自带表里有按读音匹配的词：{:?}",
        h.words()
    );
    assert!(h.rules().len() >= 10, "规则太少：{}", h.rules().len());
    for (wrong, right) in h.rules() {
        assert_ne!(
            wrong.to_lowercase(),
            right.to_lowercase(),
            "{wrong}={right} 只改大小写"
        );
    }
}

#[test]
fn common_mishearings_are_corrected() {
    let h = builtin();
    for (heard, want) in [
        ("用克劳德写代码", "用Claude写代码"),
        ("打开 cloud code 看看", "打开 Claude Code 看看"),
        ("按温加H说话", "按Win+H说话"),
        ("本地跑欧拉玛", "本地跑Ollama"),
        ("推到吉特哈布上", "推到GitHub上"),
    ] {
        assert_eq!(h.apply(heard), want, "{heard}");
    }
}

#[test]
fn everyday_speech_is_left_alone() {
    // 主人实际说过的话（2026-11-15～16 的听写里挑的，不含个人信息）：大小写本来就对的不能被改
    let h = builtin();
    for text in [
        "按住右 ALT 说话",
        "我又发现了一个 bug",
        "雪儿，Speaker 停掉了",
        "直接点击了 enter",
        "今天天气不错，cloud 好多",
        "这个 output 不对",
        "Out of memory 了",
    ] {
        assert_eq!(h.apply(text), text, "{text}");
    }
}
