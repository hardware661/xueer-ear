use xueer_ear_textflow::strip_fillers;

#[test]
fn removes_leading_fillers() {
    assert_eq!(strip_fillers("嗯，今天把代码提交了"), "今天把代码提交了");
    assert_eq!(strip_fillers("呃，那个，我们先测试"), "我们先测试");
    assert_eq!(strip_fillers("那个就是说，我们先测试"), "我们先测试");
}

#[test]
fn removes_fillers_at_each_sentence_start() {
    assert_eq!(
        strip_fillers("先跑测试。嗯，然后提交。"),
        "先跑测试。然后提交。"
    );
}

#[test]
fn removes_standalone_filler_sentences() {
    assert_eq!(strip_fillers("好的。嗯。下一步"), "好的。下一步");
    assert_eq!(strip_fillers("嗯嗯"), "");
}

#[test]
fn keeps_fillers_used_as_real_words() {
    assert_eq!(strip_fillers("打开那个文件"), "打开那个文件");
    assert_eq!(strip_fillers("那个文件在哪"), "那个文件在哪");
    assert_eq!(strip_fillers("就是说服不了他"), "就是说服不了他");
    assert_eq!(strip_fillers("嗯哼是个语气词"), "嗯哼是个语气词");
}

#[test]
fn e_that_starts_a_real_word_is_kept() {
    // 审查 R7：「额度用完了」被删成「度用完了」
    for text in ["额度用完了", "额外的工作", "额头疼", "额定功率是多少"] {
        assert_eq!(strip_fillers(text), text, "{text}");
    }
    // 真的口头禅照删
    assert_eq!(strip_fillers("额，今天提交"), "今天提交");
    assert_eq!(strip_fillers("额额，好吧"), "好吧");
    assert_eq!(strip_fillers("额。"), "");
}
