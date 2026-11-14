use xueer_ear_textflow::Hotwords;

fn hw(list: &str) -> Hotwords {
    Hotwords::parse(list)
}

#[test]
fn replaces_same_pinyin_with_hotword() {
    let h = hw("雪耳\n雪儿\n");
    assert_eq!(h.apply("打开雪而的设置"), "打开雪耳的设置");
}

#[test]
fn fuzzy_initials_and_finals() {
    // 平翘舌、前后鼻音按模糊音算同音
    let h = hw("甲骨文\n");
    assert_eq!(h.apply("登录假古文服务器"), "登录甲骨文服务器");
    let h = hw("深圳\n");
    assert_eq!(h.apply("森镇的天气"), "深圳的天气");
}

#[test]
fn latin_hotwords_fix_case_and_spacing() {
    let h = hw("New API\nClaude Code\nhdwde\n");
    assert_eq!(h.apply("重启new api"), "重启New API");
    assert_eq!(h.apply("在claude code里"), "在Claude Code里");
    assert_eq!(h.apply("HDWDE平台"), "hdwde平台");
}

#[test]
fn leaves_unrelated_text_alone() {
    let h = hw("雪耳\n");
    assert_eq!(h.apply("下雪了"), "下雪了");
    assert_eq!(h.apply("耳朵"), "耳朵");
}

#[test]
fn file_format_ignores_comments_and_rime_weights() {
    // 兼容雾凇 custom_phrase.txt：词<TAB>编码<TAB>权重，# 开头是注释
    let h = hw("# 注释\n甲骨文\tjgw\t1\n\n");
    assert_eq!(h.words(), ["甲骨文"]);
}

#[test]
fn longer_hotword_wins() {
    let h = hw("雪儿\n雪儿bot\n");
    assert_eq!(h.apply("重启雪而bot"), "重启雪儿bot");
}

#[test]
fn latin_hotword_must_be_a_whole_word() {
    let h = hw("CPA\nResin\n");
    assert_eq!(h.apply("cpa挂了"), "CPA挂了");
    assert_eq!(h.apply("ICPAB"), "ICPAB");
    assert_eq!(h.apply("resins"), "resins");
}

#[test]
fn exact_text_is_never_rewritten_to_a_sound_alike() {
    // 雪儿 和 雪耳 同音：原文已经是其中一个热词时保持原样
    let h = hw("雪耳\n雪儿\n");
    assert_eq!(h.apply("雪儿说话"), "雪儿说话");
    assert_eq!(h.apply("雪耳听写"), "雪耳听写");
}

#[test]
fn empty_list_is_identity() {
    let h = hw("");
    assert!(h.is_empty());
    assert_eq!(h.apply("任何文字 any text"), "任何文字 any text");
}
