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

#[test]
fn replacement_rules_fix_words_that_do_not_sound_alike() {
    // 「错=对」：读音纠不了的固定错法（今天主人的 Claude Code 被听成 Cloud Code）
    let h = hw("Cloud Code=Claude Code\n温加H=Win+H\nout=Alt\n");
    assert_eq!(h.apply("在cloud code里说话"), "在Claude Code里说话");
    assert_eq!(h.apply("我想要温加H的效果"), "我想要Win+H的效果");
    assert_eq!(h.apply("点击out的时候"), "点击Alt的时候");
}

#[test]
fn replacement_rules_need_whole_latin_words() {
    let h = hw("out=Alt\n");
    assert_eq!(
        h.apply("the output is without errors"),
        "the output is without errors"
    );
    assert_eq!(h.apply("press OUT now"), "press Alt now");
}

#[test]
fn rules_and_plain_hotwords_mix_and_list_separately() {
    let h = hw("# 注释\n雪耳\nout=Alt\n = 坏行\n空的=\n");
    assert_eq!(h.words(), ["雪耳"]);
    assert_eq!(h.rules(), [("out", "Alt")]);
    assert_eq!(h.apply("雪而的out键"), "雪耳的Alt键");
}

#[test]
fn rule_output_is_not_rewritten_again() {
    // 替换结果不再被别的规则或热词二次改写
    let h = hw("甲=乙\n乙=丙\n");
    assert_eq!(h.apply("甲乙"), "乙丙");
}

#[test]
fn adding_a_line_keeps_existing_content_and_skips_duplicates() {
    use xueer_ear_textflow::add_hotword_line;
    let file = "# 我的热词\n雪耳\nout=Alt";
    let added = add_hotword_line(file, "  Claude Code ").unwrap();
    assert_eq!(added, "# 我的热词\n雪耳\nout=Alt\nClaude Code\n");
    // 已有的不重复加
    assert_eq!(add_hotword_line(file, "雪耳"), None);
    assert_eq!(add_hotword_line(file, "out = Alt"), None);
    // 同一个「错」换一个「对」：改掉旧的那行
    assert_eq!(
        add_hotword_line(file, "out=右 Alt").unwrap(),
        "# 我的热词\n雪耳\nout=右 Alt\n"
    );
    // 空的、注释、带换行的都不收
    assert_eq!(add_hotword_line(file, "   "), None);
    assert_eq!(add_hotword_line(file, "# 不是词"), None);
    assert_eq!(add_hotword_line(file, "=只有右边"), None);
    assert_eq!(add_hotword_line("", "第一个").unwrap(), "第一个\n");
}
