use xueer_ear_textflow::{CommandOptions, VoiceCommand, parse_command};

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
    let on = CommandOptions { allow_send: true };
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
