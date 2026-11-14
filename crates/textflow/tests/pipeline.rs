use xueer_ear_textflow::{Hotwords, Options, Output, VoiceCommand, join_for_terminal, process};

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
