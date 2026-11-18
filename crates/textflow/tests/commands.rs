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

#[test]
fn a_command_said_twice_in_a_row_is_still_the_command() {
    // 主人历史：「发送发送。」「撤回撤回」被原样打了出来（2026-11-19 复查）
    let on = CommandOptions { allow_send: true };
    assert_eq!(parse_command("发送发送。", &on), Some(VoiceCommand::Send));
    assert_eq!(
        parse_command("撤回撤回", &on),
        Some(VoiceCommand::DeleteLastSentence)
    );
    assert_eq!(
        parse_command("撤回，撤回。", &on),
        Some(VoiceCommand::DeleteLastSentence)
    );
    assert_eq!(parse_command("换行换行", &on), Some(VoiceCommand::NewLine));
    // 两个不同的指令连说不算
    assert_eq!(parse_command("撤回发送", &on), None);
}

#[test]
fn translate_said_on_its_own() {
    let o = CommandOptions::default();
    for said in [
        "翻译",
        "翻译。",
        "英文",
        "翻译成英文",
        "说英文",
        "翻成英文",
        "翻译成英语",
    ] {
        assert_eq!(
            parse_command(said, &o),
            Some(VoiceCommand::Translate),
            "{said}"
        );
    }
    assert_eq!(parse_command("翻译一下这段代码", &o), None);
    assert_eq!(parse_command("英文不好", &o), None);
}
