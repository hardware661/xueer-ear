use xueer_ear_textflow::{
    Hotwords, Options, Output, Snippets, VoiceCommand, join_for_terminal, process, process_text,
    terminal_append,
};

#[test]
fn full_pipeline_in_order() {
    let o = Options {
        hotwords: Hotwords::parse("甲骨文\n"),
        ..Options::default()
    };
    // 口癖 → 热词 → 数字 → 标点
    assert_eq!(
        process("嗯，把假古文的内存加到百分之五十", &o),
        Output::Text("把甲骨文的内存加到50%".into())
    );
}

#[test]
fn commands_win_over_text() {
    let o = Options::default();
    assert_eq!(
        process("嗯，删除。", &o),
        Output::Command(VoiceCommand::DeleteLastSentence)
    );
    // 「换行」不再是指令（主人 2026-11-24 砍掉了）
    assert_eq!(process("换行", &o), Output::Text("换行".into()));
}

#[test]
fn same_input_same_output() {
    let o = Options {
        hotwords: Hotwords::parse("雪耳\n"),
        ..Options::default()
    };
    let a = process("雪而三点五版本", &o);
    assert_eq!(a, process("雪而三点五版本", &o));
    assert_eq!(a, Output::Text("雪耳3.5版本".into()));
}

#[test]
fn filler_only_utterance_is_empty() {
    assert_eq!(
        process("嗯。", &Options::default()),
        Output::Text(String::new())
    );
}

#[test]
fn terminal_join_never_adds_newline_or_enter() {
    let parts = ["先读 STATE.md。", "然后跑测试", "", "最后提交"];
    let joined = join_for_terminal(&parts);
    assert_eq!(joined, "先读 STATE.md。然后跑测试，最后提交");
    assert!(!joined.contains('\n') && !joined.contains('\r'));
}

#[test]
fn terminal_join_uses_space_between_latin_sentences() {
    assert_eq!(
        join_for_terminal(&["run tests", "commit"]),
        "run tests commit"
    );
    assert_eq!(join_for_terminal(&["换行\n测试", "好"]), "换行 测试，好");
}

#[test]
fn a_command_at_the_end_of_a_sentence_is_just_text() {
    // 主人 2026-11-24：指令要先停一下、单独说；跟在话后面一口气说出来的一律当普通文字
    let o = Options::default();
    for text in [
        "这句是错的，删除",
        "这些临时文件，全部删除",
        "今天吃什么，翻译成英文。",
        "删除这个文件",
    ] {
        assert!(
            matches!(process(text, &o), Output::Text(_)),
            "{text} 不该是指令"
        );
    }
}

#[test]
fn terminal_append_gives_only_the_new_piece_with_its_separator() {
    // 上一句是已经上屏的那段（可能带前导分隔符），返回值只是接在它后面要打的部分
    assert_eq!(terminal_append("", "先读文档"), "先读文档");
    assert_eq!(terminal_append("先读文档", "再跑测试"), "，再跑测试");
    assert_eq!(terminal_append("OK.", "好的。"), " 好的。");
    assert_eq!(terminal_append(" 好的。", "继续"), "继续");
    assert_eq!(terminal_append("Run tests.", "ok"), " ok");
    assert_eq!(terminal_append(" ok", "done"), " done");
    assert_eq!(terminal_append("好", "换行\n测试"), "，换行 测试");
    assert_eq!(terminal_append("好", "  "), "");
    // 和整段拼接结果一致
    let mut shown = String::new();
    let mut previous = String::new();
    for part in ["OK.", "好的。", "继续", "run", "tests"] {
        let piece = terminal_append(&previous, part);
        shown.push_str(&piece);
        previous = piece;
    }
    assert_eq!(
        shown,
        join_for_terminal(&["OK.", "好的。", "继续", "run", "tests"])
    );
}

#[test]
fn only_the_phrase_on_its_own_is_a_command() {
    let o = Options::default();
    assert_eq!(
        process("全部删除", &o),
        Output::Command(VoiceCommand::DeleteAll)
    );
    assert_eq!(
        process("翻译成中文。", &o),
        Output::Command(VoiceCommand::TranslateToChinese)
    );
    for text in ["撤销", "撤回", "清空", "删掉上一句", "翻译"] {
        assert!(
            matches!(process(text, &o), Output::Text(_)),
            "{text} 已经不是指令了"
        );
    }
}

#[test]
fn a_sentence_cut_off_mid_thought_gets_no_full_stop() {
    // 主人历史：「你可以理解我的笔记本放在左上方，然后我的。」「耳机放在左下方。」——停顿一下被切成两句
    let o = Options::default();
    assert_eq!(
        process("你可以理解我的笔记本放在左上方，然后我的。", &o),
        Output::Text("你可以理解我的笔记本放在左上方，然后我的".into())
    );
    assert_eq!(
        process("还有一个问题就是当我。", &o),
        Output::Text("还有一个问题就是当我".into())
    );
    // 正常说完的句子照旧
    assert_eq!(
        process("确实是有效果的。", &o),
        Output::Text("确实是有效果的。".into())
    );
    assert_eq!(process("不会。", &o), Output::Text("不会。".into()));
}

#[test]
fn terminal_joins_a_cut_off_sentence_without_a_comma() {
    assert_eq!(
        terminal_append("然后我的", "耳机放在左下方。"),
        "耳机放在左下方。"
    );
    assert_eq!(
        terminal_append("今天天气不错", "我们去公园"),
        "，我们去公园"
    );
}

#[test]
fn a_whole_sentence_snippet_trigger_becomes_its_text_untouched() {
    let o = Options {
        snippets: Snippets::parse(
            "我的邮箱=xueer@example.com\n报价=一百块。\\n谢谢\n删除=不会生效\n",
        ),
        ..Options::default()
    };
    assert_eq!(
        process("嗯，我的邮箱。", &o),
        Output::Snippet("xueer@example.com".into())
    );
    // 换出来的文字不再整理（不改数字、标点）
    assert_eq!(
        process("报价", &o),
        Output::Snippet("一百块。\n谢谢".into())
    );
    // 句子里提到口令照常是文字
    assert_eq!(
        process("把我的邮箱发给他", &o),
        Output::Text("把我的邮箱发给他".into())
    );
    // 指令优先
    assert_eq!(
        process("删除", &o),
        Output::Command(VoiceCommand::DeleteLastSentence)
    );
}

#[test]
fn recognized_text_never_carries_line_breaks_into_the_output() {
    // 雪耳从不往输入框里打换行：云端文字里的控制字符当空格
    let o = Options::default();
    assert_eq!(
        process("第一行\n第二行", &o),
        Output::Text("第一行 第二行".into())
    );
}

#[test]
fn a_command_phrase_can_be_kept_as_plain_text() {
    // 快捷键听写里前面没停够的「删除」：照样整理、照样打出来
    let o = Options::default();
    assert_eq!(
        process_text("嗯，删除。", &o),
        Output::Text("删除。".into())
    );
    let o = Options {
        snippets: Snippets::parse("我的邮箱=xueer@example.com\n"),
        ..Options::default()
    };
    assert_eq!(
        process_text("我的邮箱", &o),
        Output::Snippet("xueer@example.com".into())
    );
}
