use xueer_ear_textflow::{
    Hotwords, Options, Output, VoiceCommand, join_for_terminal, process, terminal_append,
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
        process("嗯，换行。", &o),
        Output::Command(VoiceCommand::NewLine)
    );
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
fn trailing_command_after_text_is_split_out() {
    let o = Options::default();
    assert_eq!(
        process("这句是错的，删掉上一句。", &o),
        Output::TextThen("这句是错的，".into(), VoiceCommand::DeleteLastSentence)
    );
    assert_eq!(
        process("第一行，换行", &o),
        Output::TextThen("第一行，".into(), VoiceCommand::NewLine)
    );
    // 指令词不在句末单独一小句的，照样是普通文字
    assert_eq!(
        process("这里要换行吗", &o),
        Output::Text("这里要换行吗".into())
    );
    assert_eq!(
        process("撤销这次提交", &o),
        Output::Text("撤销这次提交".into())
    );
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
fn everyday_phrases_ending_in_a_delete_word_are_just_text() {
    // 审查 R2：「这些临时文件，全部删除」被当成指令，把这次听写打的字全删光
    let o = Options::default();
    for text in [
        "这些临时文件，全部删除",
        "缓存，清空",
        "日志太多了，删了",
        "这一步，撤销",
        "不要的文件，删除。",
    ] {
        assert!(
            matches!(process(text, &o), Output::Text(_)),
            "{text} 不该是指令"
        );
    }
    // 单独说照样是指令
    assert_eq!(
        process("全部删除", &o),
        Output::Command(VoiceCommand::DeleteAll)
    );
    assert_eq!(
        process("撤销。", &o),
        Output::Command(VoiceCommand::DeleteLastSentence)
    );
    // 明确说「删掉上一句」接在句尾仍然算（主人 2026-11-15 要的）
    assert_eq!(
        process("这句是错的，删除上一句话", &o),
        Output::TextThen("这句是错的，".into(), VoiceCommand::DeleteLastSentence)
    );
}

#[test]
fn translate_at_the_end_of_a_sentence() {
    let o = Options::default();
    assert_eq!(
        process("今天吃什么，翻译成英文。", &o),
        Output::TextThen("今天吃什么，".into(), VoiceCommand::Translate)
    );
    // 句尾只说「翻译」太常见（「这段帮我，翻译」也可能是正文），只认说全的
    assert_eq!(
        process("这一段，翻译", &o),
        Output::Text("这一段，翻译".into())
    );
}
