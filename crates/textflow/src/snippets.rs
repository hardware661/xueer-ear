//! 快捷短语：整句只说一个口令（前后可以带口癖和句末标点）时，换成事先写好的一段文字，
//! 比如「我的邮箱」→ 邮箱地址、「收货地址」→ 一整段地址。
//!
//! - 文件 `snippets.txt` 一行一条「口令=文字」，`#` 开头是注释；文字里 `\n` 是换行，`\\` 是反斜杠本身。
//! - 口令按读音认（同热词：平翘舌、前后鼻音算同音，两个字以上才按读音比；英文不分大小写，空格标点不算）。
//! - 只认整句：句子里提到口令（「把我的邮箱发给他」）照常当文字。
//! - 语音指令优先：口令和指令词读音一样时指令赢，加口令时也不让这样加（[`set_snippet_line`]）。
//! - 换出来的文字原样上屏，不再过热词、数字、标点整理。

use crate::{CommandOptions, commands::plain, hotwords::sounds_like};

#[derive(Debug, Clone, Default)]
pub struct Snippets {
    entries: Vec<(String, String)>,
}

impl Snippets {
    pub fn parse(file: &str) -> Self {
        let mut entries: Vec<(String, String)> = Vec::new();
        for line in file.lines() {
            let Some((trigger, text)) = split_line(line) else {
                continue;
            };
            let text = unescape(text);
            if text.trim().is_empty() || entries.iter().any(|(t, _)| plain(t) == plain(trigger)) {
                continue;
            }
            entries.push((trigger.to_string(), text));
        }
        Self { entries }
    }

    /// （口令, 文字），按文件顺序
    pub fn entries(&self) -> &[(String, String)] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 整句就是某个口令时返回它的文字。先找字面一样的，再按读音找（「我的油箱」→「我的邮箱」）。
    pub fn lookup(&self, spoken: &str) -> Option<&str> {
        let key = plain(spoken);
        if key.is_empty() {
            return None;
        }
        self.entries
            .iter()
            .find(|(trigger, _)| plain(trigger) == key)
            .or_else(|| {
                self.entries
                    .iter()
                    .find(|(trigger, _)| sounds_like(trigger, spoken))
            })
            .map(|(_, text)| text.as_str())
    }
}

/// 加一条快捷短语（同一个口令已有就换成新的文字），返回新的文件内容；注释和别的行不动。
/// 口令为空、带 `=` 或换行、和语音指令的说法（现在用的或自带的）读音一样，文字为空，都不加，返回原因。
pub fn set_snippet_line(
    file: &str,
    trigger: &str,
    text: &str,
    commands: &CommandOptions,
) -> Result<String, String> {
    let trigger = trigger.trim();
    if plain(trigger).is_empty() {
        return Err("口令不能是空的".into());
    }
    if trigger.contains(['=', '\n', '\r']) || trigger.starts_with('#') {
        return Err("口令里不能有 = 或换行，也不能以 # 开头".into());
    }
    if let Some(word) = commands.clashing_word(trigger) {
        return Err(format!("这个口令和语音指令「{word}」撞了，换一个吧"));
    }
    if text.trim().is_empty() {
        return Err("文字不能是空的".into());
    }
    let line = format!("{trigger}={}", escape(text.trim_end_matches(['\r', '\n'])));
    let mut lines: Vec<String> = file.lines().map(str::to_string).collect();
    match lines.iter().position(|l| same_trigger(l, trigger)) {
        Some(i) => lines[i] = line,
        None => lines.push(line),
    }
    Ok(lines.join("\n") + "\n")
}

/// 删掉一个口令，返回新的文件内容；没有这个口令返回 None。
pub fn remove_snippet_line(file: &str, trigger: &str) -> Option<String> {
    let lines: Vec<&str> = file.lines().collect();
    let kept: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| !same_trigger(l, trigger))
        .collect();
    if kept.len() == lines.len() {
        return None;
    }
    if kept.is_empty() {
        return Some(String::new());
    }
    Some(kept.join("\n") + "\n")
}

/// 一行里的（口令, 文字原文）；注释、空行、没有 `=` 的行是 None
fn split_line(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (trigger, text) = line.split_once('=')?;
    let trigger = trigger.trim();
    (!plain(trigger).is_empty()).then_some((trigger, text.trim()))
}

fn same_trigger(line: &str, trigger: &str) -> bool {
    split_line(line).is_some_and(|(t, _)| plain(t) == plain(trigger))
}

/// `\n` → 换行，`\\` → 反斜杠；别的反斜杠原样留着（`C:\Users` 不用写成 `C:\\Users`）
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('\\', Some('n')) => {
                chars.next();
                out.push('\n');
            }
            ('\\', Some('\\')) => {
                chars.next();
                out.push('\\');
            }
            _ => out.push(c),
        }
    }
    out
}

fn escape(text: &str) -> String {
    text.replace('\r', "")
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
}
