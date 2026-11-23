use xueer_ear_textflow::{CommandOptions, VoiceCommand, parse_command};

/// 「发送」也开着
fn all_on() -> CommandOptions {
    CommandOptions::with_switches(|_| Some(true))
}

// 主人 2026-11-24：说法就是指令，一个指令只有一个说法
#[test]
fn each_command_has_exactly_one_phrase() {
    let o = all_on();
    for (said, command) in [
        ("删除", VoiceCommand::DeleteLastSentence),
        ("全部删除", VoiceCommand::DeleteAll),
        ("发送", VoiceCommand::Send),
        ("翻译成英文", VoiceCommand::TranslateToEnglish),
        ("翻译成中文", VoiceCommand::TranslateToChinese),
    ] {
        assert_eq!(parse_command(said, &o), Some(command), "{said}");
        assert_eq!(command.name(), said);
    }
    // 以前的别的说法、换行、单说「翻译」都不再是指令
    for said in [
        "删掉",
        "删了",
        "撤销",
        "撤回",
        "删掉上一句",
        "全部删掉",
        "清空",
        "换行",
        "翻译",
    ] {
        assert_eq!(parse_command(said, &o), None, "{said}");
    }
}

#[test]
fn punctuation_spaces_and_case_do_not_matter_but_extra_words_do() {
    let o = all_on();
    assert_eq!(
        parse_command("删除。", &o),
        Some(VoiceCommand::DeleteLastSentence)
    );
    assert_eq!(
        parse_command(" 全部 删除！", &o),
        Some(VoiceCommand::DeleteAll)
    );
    assert_eq!(parse_command("删除这个文件", &o), None);
    assert_eq!(parse_command("这些临时文件，全部删除", &o), None);
    assert_eq!(parse_command("今天吃什么，发送", &o), None);
}

#[test]
fn send_is_off_unless_switched_on() {
    let o = CommandOptions::default();
    assert_eq!(parse_command("发送", &o), None);
    assert!(!o.is_on(VoiceCommand::Send));
    assert!(o.is_on(VoiceCommand::DeleteLastSentence));
    assert_eq!(parse_command("发送", &all_on()), Some(VoiceCommand::Send));
}

#[test]
fn a_switched_off_command_is_plain_text() {
    let o = CommandOptions::with_switches(|c| (c == VoiceCommand::DeleteAll).then_some(false));
    assert_eq!(parse_command("全部删除", &o), None);
    assert_eq!(
        parse_command("删除", &o),
        Some(VoiceCommand::DeleteLastSentence)
    );
}

// 说到一半的指令先不打出来：中间结果只是某个指令的开头（「删」「全部」「翻译成」）
#[test]
fn the_start_of_a_command_is_recognised_while_it_is_being_said() {
    let o = all_on();
    for said in [
        "删",
        "删除",
        "全部",
        "全部删",
        "翻译",
        "翻译成",
        "翻译成英",
        "发",
    ] {
        assert!(o.starts_a_command(said), "{said}");
    }
    for said in ["", "。", "删除这", "今天", "翻译一下", "全部的"] {
        assert!(!o.starts_a_command(said), "{said}");
    }
    // 关着的指令不算
    let o = CommandOptions::default();
    assert!(!o.starts_a_command("发"));
}

#[test]
fn every_command_has_a_stable_id() {
    for command in VoiceCommand::ALL {
        assert_eq!(VoiceCommand::from_id(command.id()), Some(command));
    }
    assert_eq!(
        VoiceCommand::ALL.map(VoiceCommand::id),
        [
            "delete_last",
            "delete_all",
            "send",
            "to_english",
            "to_chinese"
        ]
    );
    assert_eq!(VoiceCommand::from_id("new_line"), None);
}
