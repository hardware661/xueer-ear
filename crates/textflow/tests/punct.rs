use xueer_ear_textflow::{apply_punctuation, PunctOptions};

fn opts(drop_final_period: bool, cjk_latin_space: bool, fullwidth: bool) -> PunctOptions {
    PunctOptions { drop_final_period, cjk_latin_space, fullwidth_punct_in_cjk: fullwidth }
}

#[test]
fn default_changes_nothing() {
    let s = "提交到GitHub上。";
    assert_eq!(apply_punctuation(s, &PunctOptions::default()), s);
}

#[test]
fn drops_only_the_final_period() {
    let o = opts(true, false, false);
    assert_eq!(apply_punctuation("先测试。再提交。", &o), "先测试。再提交");
    assert_eq!(apply_punctuation("真的吗？", &o), "真的吗？");
    assert_eq!(apply_punctuation("done.", &o), "done");
    assert_eq!(apply_punctuation("版本3.5", &o), "版本3.5");
}

#[test]
fn spaces_between_cjk_and_latin() {
    let o = opts(false, true, false);
    assert_eq!(apply_punctuation("用Claude Code写Rust代码", &o), "用 Claude Code 写 Rust 代码");
    assert_eq!(apply_punctuation("完成50%了", &o), "完成 50% 了");
    assert_eq!(apply_punctuation("用 Rust 写", &o), "用 Rust 写");
    assert_eq!(apply_punctuation("（Rust）", &o), "（Rust）");
}

#[test]
fn unifies_halfwidth_punct_next_to_cjk() {
    let o = opts(false, false, true);
    assert_eq!(apply_punctuation("好的,然后呢?", &o), "好的，然后呢？");
    assert_eq!(apply_punctuation("a,b and 3.5", &o), "a,b and 3.5");
}
