use xueer_ear_textflow::{parse_command, CommandOptions, VoiceCommand};

#[test]
fn whole_utterance_commands() {
    let o = CommandOptions::default();
    assert_eq!(parse_command("换行", &o), Some(VoiceCommand::NewLine));
    assert_eq!(parse_command("换行。", &o), Some(VoiceCommand::NewLine));
    assert_eq!(parse_command("删掉上一句", &o), Some(VoiceCommand::DeleteLastSentence));
    assert_eq!(parse_command("撤销！", &o), Some(VoiceCommand::Undo));
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
