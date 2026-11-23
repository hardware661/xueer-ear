use xueer_ear_textflow::{Snippets, remove_snippet_line, set_snippet_line};

const FILE: &str = "# 我的快捷短语\n我的邮箱=xueer@example.com\n收货地址 = 某某路 1 号\\n张三收\n\nMy Sign=Best,\\nXueer\n路径=C:\\Users\\x\n反斜杠=a\\\\nb\n没有等号的行\n=没有口令\n空文字=\n";

#[test]
fn lines_are_trigger_equals_text_with_comments_and_escapes() {
    let s = Snippets::parse(FILE);
    assert_eq!(
        s.entries(),
        [
            ("我的邮箱".into(), "xueer@example.com".into()),
            ("收货地址".into(), "某某路 1 号\n张三收".into()),
            ("My Sign".into(), "Best,\nXueer".into()),
            ("路径".into(), "C:\\Users\\x".into()),
            ("反斜杠".into(), "a\\nb".into()),
        ]
    );
}

#[test]
fn the_first_of_two_same_triggers_wins() {
    let s = Snippets::parse("邮箱=a\n邮 箱=b\n");
    assert_eq!(s.entries().len(), 1);
    assert_eq!(s.lookup("邮箱"), Some("a"));
}

#[test]
fn only_a_whole_sentence_that_is_the_trigger_expands() {
    let s = Snippets::parse(FILE);
    assert_eq!(s.lookup("我的邮箱"), Some("xueer@example.com"));
    assert_eq!(s.lookup("我的邮箱。"), Some("xueer@example.com"));
    assert_eq!(s.lookup(" 我的 邮箱！"), Some("xueer@example.com"));
    assert_eq!(s.lookup("my sign."), Some("Best,\nXueer"));
    assert_eq!(s.lookup("把我的邮箱发给他"), None);
    assert_eq!(s.lookup("我的邮箱是什么"), None);
    assert_eq!(s.lookup("。"), None);
    assert_eq!(s.lookup(""), None);
}

#[test]
fn triggers_are_recognized_by_sound() {
    let s = Snippets::parse(FILE);
    // 邮/油同音；平翘舌、前后鼻音也算同音
    assert_eq!(s.lookup("我的油箱"), Some("xueer@example.com"));
    assert_eq!(s.lookup("收获地址"), Some("某某路 1 号\n张三收"));
    // 读音不一样不算
    assert_eq!(s.lookup("我的信箱"), None);
}

#[test]
fn a_single_character_trigger_needs_the_exact_character() {
    let s = Snippets::parse("嗨=Hi there\n");
    assert_eq!(s.lookup("嗨"), Some("Hi there"));
    assert_eq!(s.lookup("海"), None);
}

#[test]
fn adding_a_snippet_appends_or_replaces_and_keeps_other_lines() {
    let file = "# 注释\n邮箱=old\n地址=x\n";
    assert_eq!(
        set_snippet_line(file, "签名", "雪儿\n敬上").unwrap(),
        "# 注释\n邮箱=old\n地址=x\n签名=雪儿\\n敬上\n"
    );
    assert_eq!(
        set_snippet_line(file, " 邮箱 ", "new").unwrap(),
        "# 注释\n邮箱=new\n地址=x\n"
    );
    assert_eq!(
        set_snippet_line("", "路径", "C:\\new").unwrap(),
        "路径=C:\\\\new\n"
    );
    // 写进去再读出来还是原来的文字
    let written = set_snippet_line("", "多行", "第一行\n第二行 \\n 原样").unwrap();
    assert_eq!(
        Snippets::parse(&written).lookup("多行"),
        Some("第一行\n第二行 \\n 原样")
    );
}

#[test]
fn triggers_that_clash_with_voice_commands_are_refused() {
    assert!(set_snippet_line("", "换行", "x").is_err());
    assert!(set_snippet_line("", "发送。", "x").is_err());
    assert!(set_snippet_line("", "翻译成英文", "x").is_err());
    // 读音一样也不行（撤回 / 车回）
    assert!(set_snippet_line("", "车回", "x").is_err());
    assert!(set_snippet_line("", "", "x").is_err());
    assert!(set_snippet_line("", "。", "x").is_err());
    assert!(set_snippet_line("", "a=b", "x").is_err());
    assert!(set_snippet_line("", "#注释", "x").is_err());
    assert!(set_snippet_line("", "邮箱", "  ").is_err());
}

#[test]
fn removing_a_snippet_drops_only_its_line() {
    let file = "# 注释\n邮箱=a\n地址=b\n";
    assert_eq!(
        remove_snippet_line(file, "邮箱").unwrap(),
        "# 注释\n地址=b\n"
    );
    assert_eq!(remove_snippet_line(file, "签名"), None);
}
