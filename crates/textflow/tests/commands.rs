use xueer_ear_textflow::{
    CommandOptions, VoiceCommand, check_words, parse_command, parse_trailing_command, split_words,
};

/// 只改了某几个指令的说法（别的用默认）
fn custom(changed: &[(VoiceCommand, &[&str])]) -> CommandOptions {
    let changed: Vec<(VoiceCommand, Vec<String>)> = changed
        .iter()
        .map(|(c, words)| (*c, words.iter().map(|w| w.to_string()).collect()))
        .collect();
    CommandOptions::with_words(|command| {
        changed
            .iter()
            .find(|(c, _)| *c == command)
            .map(|(_, words)| words.clone())
    })
}

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|w| w.to_string()).collect()
}

#[test]
fn whole_utterance_commands() {
    let o = CommandOptions::default();
    assert_eq!(parse_command("换行", &o), Some(VoiceCommand::NewLine));
    assert_eq!(parse_command("换行。", &o), Some(VoiceCommand::NewLine));
    assert_eq!(
        parse_command("删掉上一句", &o),
        Some(VoiceCommand::DeleteLastSentence)
    );
    assert_eq!(
        parse_command("撤销！", &o),
        Some(VoiceCommand::DeleteLastSentence)
    );
}

#[test]
fn commands_inside_a_sentence_are_just_text() {
    let o = CommandOptions::default();
    assert_eq!(parse_command("这里要换行吗", &o), None);
    assert_eq!(parse_command("撤销这次提交", &o), None);
}

#[test]
fn send_is_off_by_default() {
    assert_eq!(parse_command("发送", &CommandOptions::default()), None);
    let on = custom(&[(VoiceCommand::Send, &["发送"])]);
    assert_eq!(parse_command("发送。", &on), Some(VoiceCommand::Send));
}

#[test]
fn common_ways_of_saying_delete_and_undo() {
    let o = CommandOptions::default();
    for said in [
        "删掉上一句话",
        "删除上一句话。",
        "删掉上一行",
        "删除上一行",
        "删掉上句话",
    ] {
        assert_eq!(
            parse_command(said, &o),
            Some(VoiceCommand::DeleteLastSentence),
            "{said}"
        );
    }
    assert_eq!(
        parse_command("撤回。", &o),
        Some(VoiceCommand::DeleteLastSentence)
    );
    // 带宾语的还是普通文字
    assert_eq!(parse_command("撤回这条消息", &o), None);
    assert_eq!(parse_command("删掉上一行的注释", &o), None);
}

#[test]
fn bare_delete_and_delete_all() {
    let o = CommandOptions::default();
    for said in ["删除", "删掉。", "删了", "撤销", "撤回"] {
        assert_eq!(
            parse_command(said, &o),
            Some(VoiceCommand::DeleteLastSentence),
            "{said}"
        );
    }
    for said in ["全部删除", "全部删掉。", "全都删除", "全删了", "清空"] {
        assert_eq!(
            parse_command(said, &o),
            Some(VoiceCommand::DeleteAll),
            "{said}"
        );
    }
    // 带宾语的是普通文字
    assert_eq!(parse_command("删除这个文件", &o), None);
    assert_eq!(parse_command("全部删除掉缓存", &o), None);
    assert_eq!(parse_command("清空回收站", &o), None);
}

#[test]
fn translate_said_on_its_own() {
    let o = CommandOptions::default();
    assert_eq!(parse_command("翻译", &o), Some(VoiceCommand::Translate));
    assert_eq!(parse_command("翻译。", &o), Some(VoiceCommand::Translate));
    assert_eq!(parse_command("翻译一下这段代码", &o), None);
}

#[test]
fn translate_into_english_or_chinese_names_the_direction() {
    // 主人 2026-11-21：中英混着时单说「翻译」有歧义，就说「翻译成英文 / 中文」；不做别的同义说法
    let o = CommandOptions::default();
    assert_eq!(
        parse_command("翻译成英文。", &o),
        Some(VoiceCommand::TranslateToEnglish)
    );
    assert_eq!(
        parse_command("翻译成中文", &o),
        Some(VoiceCommand::TranslateToChinese)
    );
    assert_eq!(parse_command("翻成英文", &o), None);
    assert_eq!(parse_command("翻译成英文的时候要注意", &o), None);
    assert_eq!(
        parse_trailing_command("翻译成中文", &o),
        Some(VoiceCommand::TranslateToChinese)
    );
}

#[test]
fn changed_words_replace_the_builtin_ones() {
    let o = custom(&[(VoiceCommand::DeleteLastSentence, &["撤回", "不要了"])]);
    assert_eq!(
        parse_command("不要了。", &o),
        Some(VoiceCommand::DeleteLastSentence)
    );
    assert_eq!(
        parse_command("撤回", &o),
        Some(VoiceCommand::DeleteLastSentence)
    );
    // 自带的说法去掉了，就是普通文字
    assert_eq!(parse_command("删除", &o), None);
    // 别的指令照旧
    assert_eq!(parse_command("换行", &o), Some(VoiceCommand::NewLine));
}

#[test]
fn an_empty_list_turns_a_command_off() {
    let o = custom(&[(VoiceCommand::Translate, &[])]);
    assert_eq!(parse_command("翻译", &o), None);
    assert_eq!(parse_trailing_command("翻译", &o), None);
    assert_eq!(
        parse_command("翻译成英文", &o),
        Some(VoiceCommand::TranslateToEnglish)
    );
    assert!(o.words(VoiceCommand::Translate).is_empty());
    assert!(
        CommandOptions::default()
            .words(VoiceCommand::Send)
            .is_empty()
    );
}

#[test]
fn matching_ignores_case_punctuation_and_spaces_but_not_sound() {
    let o = custom(&[(VoiceCommand::NewLine, &["new line", "下一行"])]);
    assert_eq!(parse_command("New line.", &o), Some(VoiceCommand::NewLine));
    assert_eq!(parse_command("newline", &o), Some(VoiceCommand::NewLine));
    assert_eq!(parse_command("下一行！", &o), Some(VoiceCommand::NewLine));
    // 不按读音猜：听错的写法要自己加成说法
    assert_eq!(parse_command("下一航", &o), None);
}

#[test]
fn trailing_rules_stay_fixed_for_changed_words() {
    let o = custom(&[
        (VoiceCommand::NewLine, &["下一行"]),
        (VoiceCommand::DeleteLastSentence, &["去掉上一句", "去掉"]),
        (VoiceCommand::DeleteAll, &["都不要了"]),
    ]);
    assert_eq!(
        parse_trailing_command("下一行", &o),
        Some(VoiceCommand::NewLine)
    );
    assert_eq!(
        parse_trailing_command("去掉上一句", &o),
        Some(VoiceCommand::DeleteLastSentence)
    );
    // 短的删除说法、全部删除只能单独说
    assert_eq!(parse_trailing_command("去掉", &o), None);
    assert_eq!(parse_trailing_command("都不要了", &o), None);
    assert_eq!(parse_command("都不要了", &o), Some(VoiceCommand::DeleteAll));
}

#[test]
fn typed_words_are_split_on_list_marks_and_between_chinese_words() {
    assert_eq!(
        split_words("撤回、撤销，删掉"),
        words(&["撤回", "撤销", "删掉"])
    );
    assert_eq!(
        split_words(" 撤回  撤销 / 不要了 "),
        words(&["撤回", "撤销", "不要了"])
    );
    assert_eq!(
        split_words("new line、下一行"),
        words(&["new line", "下一行"])
    );
    assert_eq!(split_words("  、，"), Vec::<String>::new());
}

#[test]
fn word_lists_are_checked_against_the_fixed_rules() {
    let o = CommandOptions::default();
    let delete = VoiceCommand::DeleteLastSentence;
    assert!(check_words(delete, &words(&["撤回", "不要了"]), &o, &[]).is_ok());
    assert!(check_words(delete, &[], &o, &[]).is_ok(), "清空就是关掉");
    let error = |list: &[&str], snippets: &[&str]| {
        check_words(delete, &words(list), &o, snippets).unwrap_err()
    };
    assert!(error(&["撤"], &[]).contains("2～10"));
    assert!(error(&["这是一个特别特别长的说法"], &[]).contains("2～10"));
    assert!(error(&["撤回", "撤回！"], &[]).contains("两遍"));
    assert!(error(&["换行"], &[]).contains("「换行」的说法"));
    assert!(error(&["我的邮箱"], &["我的油箱"]).contains("快捷短语"));
    let many: Vec<String> = (0..17).map(|i| format!("删除{i}")).collect();
    assert!(
        check_words(delete, &many, &o, &[])
            .unwrap_err()
            .contains("最多")
    );
    // 自带说法本来就合规
    for command in VoiceCommand::ALL {
        let builtin = words(command.builtin_words());
        assert!(
            check_words(command, &builtin, &o, &[]).is_ok(),
            "{command:?}"
        );
    }
}

#[test]
fn every_command_has_a_stable_id() {
    for command in VoiceCommand::ALL {
        assert_eq!(VoiceCommand::from_id(command.id()), Some(command));
        assert!(!command.name().is_empty() && !command.effect().is_empty());
    }
    assert_eq!(VoiceCommand::from_id("其他"), None);
}
