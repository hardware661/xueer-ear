use xueer_ear_textflow::corrections;

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect()
}

#[test]
fn a_wrong_character_becomes_a_two_character_rule() {
    // 只改一个字时带上旁边一个字，免得「而=耳」把所有的「而」都改掉
    assert_eq!(
        corrections("我在用雪而写代码", "我在用雪耳写代码"),
        pairs(&[("雪而", "雪耳")])
    );
}

#[test]
fn english_fixes_cover_the_whole_word() {
    assert_eq!(
        corrections("用 Cloud Code 写", "用 Claude Code 写"),
        pairs(&[("Cloud", "Claude")])
    );
}

#[test]
fn a_misheard_phrase_maps_to_what_was_meant() {
    assert_eq!(
        corrections("克劳德扣的写的", "Claude Code写的"),
        pairs(&[("克劳德扣的", "Claude Code")])
    );
}

#[test]
fn separate_fixes_in_one_sentence_become_separate_rules() {
    assert_eq!(
        corrections("雪而和克劳德都好用", "雪耳和Claude都好用"),
        pairs(&[("雪而", "雪耳"), ("克劳德", "Claude")])
    );
}

#[test]
fn punctuation_spacing_and_added_or_dropped_words_are_not_rules() {
    assert!(corrections("好的。", "好的！").is_empty());
    assert!(corrections("我用写代码", "我用它写代码").is_empty());
    assert!(corrections("嗯我们走吧", "我们走吧").is_empty());
    assert!(corrections("new api", "New API").is_empty());
    assert!(corrections("一样", "一样").is_empty());
}

#[test]
fn rules_that_would_not_fit_the_hotword_list_are_skipped() {
    assert!(corrections("a=b 写错", "c 写错").is_empty());
    let long = "长".repeat(60);
    assert!(corrections(&long, "短的").is_empty());
}
