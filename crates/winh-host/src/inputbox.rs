//! 语音指令管的是焦点所在输入框里的全部文字（主人 2026-11-24：采集整个输入框，不管谁打的）。
//!
//! 怎么读整个框：
//! - 普通输入框（记事本、浏览器里的）：UI 自动化读值模式或编辑框的文字模式，不碰剪贴板；
//! - 读不到的（微信 4 是 Qt，读不到）：全选、复制，读完马上把剪贴板还原，再按 Ctrl+End 取消选中；
//! - 终端（Windows Terminal）：读屏幕上看得见的字和光标位置，认出 Claude Code、Codex 的输入框那几行。
//!
//! 读到之后怎么改（纯逻辑，可在 WSL 测）：
//! - 删除：光标挪到最后，从末尾往前删到上一个句号、问号、感叹号或换行为止（末尾的空白一起删）；
//! - 全部删除：普通程序全选再退格；终端挪到最后、按字数退格；
//! - 翻译：普通程序全选再打译文；终端先清空再打（译文里的换行换成空格，终端里回车就是发送）。
use crate::typing::Key;
use unicode_segmentation::UnicodeSegmentation;
use xueer_ear_textflow::VoiceCommand;

/// 输入框里最多整框翻多少字：再多就不整框换了（打字要很久，也可能是一整篇文档）
pub const MAX_CHARS: usize = 2000;

/// 读到的整个输入框
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxText {
    /// 框里的全部文字（换行统一成 \n）
    pub text: String,
    /// 光标前面有几个字（按字形数）；不知道是 None
    pub caret: Option<usize>,
    pub kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// UI 自动化直接读到的
    Edit,
    /// 全选、复制读到的
    Copied,
    /// 终端屏幕上认出来的输入框
    Terminal,
}

/// 执行一个改输入框的指令要按的键，以及按完后框里剩下的字
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub keys: Vec<Key>,
    pub after: String,
}

impl BoxText {
    /// 删除、全部删除：要按的键；框是空的返回原因
    pub fn delete(&self, command: VoiceCommand) -> Result<Change, String> {
        if self.text.trim().is_empty() {
            return Err("输入框是空的".into());
        }
        match command {
            VoiceCommand::DeleteLastSentence => {
                let count = last_sentence(&self.text);
                let total = self.text.graphemes(true).count();
                let after: String = self.text.graphemes(true).take(total - count).collect();
                let mut keys = self.to_end();
                keys.push(Key::Backspace(count));
                Ok(Change { keys, after })
            }
            _ => Ok(Change {
                keys: self.clear(),
                after: String::new(),
            }),
        }
    }

    /// 整框换成 `text`（译文）。终端里回车就是提交，译文并成一行打；别的程序里多行的译文整段粘贴
    pub fn replace(&self, text: &str) -> Change {
        if self.kind == Kind::Terminal {
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            let mut keys = self.clear();
            keys.push(Key::Text(text.clone()));
            return Change { keys, after: text };
        }
        let text = normalize(text);
        let insert = if text.contains('\n') {
            Key::Paste(text.clone())
        } else {
            Key::Text(text.clone())
        };
        Change {
            keys: vec![Key::SelectAll, insert],
            after: text,
        }
    }

    /// 光标挪到框的最后：普通程序按 Ctrl+End；终端里按光标后面有几个字往右挪
    fn to_end(&self) -> Vec<Key> {
        match self.kind {
            Kind::Terminal => {
                let total = self.text.graphemes(true).count();
                let after = total.saturating_sub(self.caret.unwrap_or(total));
                if after > 0 {
                    vec![Key::Right(after)]
                } else {
                    vec![]
                }
            }
            _ => vec![Key::End],
        }
    }

    /// 清空整个框
    fn clear(&self) -> Vec<Key> {
        match self.kind {
            // 终端没有全选：挪到最后按字数退格。认出来的换行可能是自动折行（不占字），多退几下也删不到框外
            Kind::Terminal => {
                let mut keys = self.to_end();
                keys.push(Key::Backspace(self.text.graphemes(true).count() + 2));
                keys
            }
            _ => vec![Key::SelectAll, Key::Backspace(1)],
        }
    }
}

/// 最后一句有几个字（按字形数）：从末尾往前，删到上一个句号、问号、感叹号或换行为止，末尾的空白一起算。
/// 英文的 . ! ? 后面要跟着空白才算句末（「3.5」里的点不算）
pub fn last_sentence(text: &str) -> usize {
    let graphemes: Vec<&str> = text.graphemes(true).collect();
    let mut end = graphemes.len();
    while end > 0 && graphemes[end - 1].trim().is_empty() {
        end -= 1;
    }
    // 最后一句自己的句末标点也是它的
    while end > 0 && is_sentence_end(&graphemes, end - 1) && graphemes[end - 1] != "\n" {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && !is_sentence_end(&graphemes, start - 1) {
        start -= 1;
    }
    graphemes.len() - start
}

fn is_sentence_end(graphemes: &[&str], at: usize) -> bool {
    match graphemes[at] {
        "。" | "！" | "？" | "\n" | "…" => true,
        "." | "!" | "?" => graphemes
            .get(at + 1)
            .is_none_or(|next| next.trim().is_empty() || next.chars().any(is_cjk)),
        _ => false,
    }
}

fn is_cjk(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{f900}'..='\u{faff}')
}

/// 读到的文字：换行统一成 \n（Windows 的输入框给的是 \r\n）
pub fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// 从终端屏幕上认出光标所在的输入框（Claude Code、Codex）。`lines` 是屏幕上看得见的每一行，
/// 光标在第 `caret_line` 行第 `caret_column` 个字（按字数）。认不出返回 None。
///
/// - Claude Code：上下两条「─」横线夹着，第一行以「❯」开头（老版本外面还有「│」边框、提示符是「>」）；
/// - Codex：上下是空行，第一行以「›」开头；
/// - 接着的行缩进两格。
///
/// 分不清自动折行和真换行，几行一律接成一段：中文接中文不加空格，别的加一个空格。
/// 光标在第一个字前面、框里却有字的，多半是没输入时的灰色提示语，当成空框（删不到、也不会翻）
pub fn terminal_box(lines: &[String], caret_line: usize, caret_column: usize) -> Option<BoxText> {
    if caret_line >= lines.len() {
        return None;
    }
    let above = |i: usize| i.checked_sub(1).map(|up| lines[up].as_str());
    let mut top = None;
    for i in (caret_line.saturating_sub(40)..=caret_line).rev() {
        let line = unframed(&lines[i]);
        if prompt_rest(line, above(i)).is_some() {
            top = Some(i);
            break;
        }
        if is_rule(&lines[i]) || (i != caret_line && line.trim().is_empty()) {
            return None;
        }
    }
    let top = top?;
    let mut bottom = top;
    while bottom + 1 < lines.len() {
        let line = unframed(&lines[bottom + 1]);
        if is_rule(&lines[bottom + 1]) || line.trim().is_empty() || !line.starts_with("  ") {
            break;
        }
        bottom += 1;
    }
    if !(top..=bottom).contains(&caret_line) {
        return None;
    }
    let mut text = String::new();
    let mut caret = None;
    for i in top..=bottom {
        let raw = lines[i].as_str();
        let line = unframed(raw);
        let body = if i == top {
            prompt_rest(line, above(i))?
        } else {
            line.strip_prefix("  ").unwrap_or(line)
        };
        // 正文在这一行里从第几个字开始（前面是边框、提示符、缩进）
        let offset = raw[..body.as_ptr() as usize - raw.as_ptr() as usize]
            .chars()
            .count();
        let body = body.trim_end_matches(|c: char| c.is_whitespace() || c == '│');
        if i > top && !body.is_empty() && !text.is_empty() {
            let joins_cjk =
                text.chars().last().is_some_and(is_cjk) && body.chars().next().is_some_and(is_cjk);
            if !joins_cjk {
                text.push(' ');
            }
        }
        if i == caret_line {
            let before: String = body
                .chars()
                .take(caret_column.saturating_sub(offset))
                .collect();
            caret = Some(text.graphemes(true).count() + before.graphemes(true).count());
        }
        text.push_str(body);
    }
    if caret == Some(0) && !text.is_empty() {
        text.clear();
    }
    Some(BoxText {
        text,
        caret,
        kind: Kind::Terminal,
    })
}

/// 去掉一行两边的「│」边框（老版本 Claude Code 的输入框带框）；行首的缩进留着（接着的行靠它认）
fn unframed(line: &str) -> &str {
    let inner = match line.strip_prefix('│') {
        Some(rest) => rest.strip_prefix(' ').unwrap_or(rest),
        None => line,
    };
    inner.trim_end().strip_suffix('│').unwrap_or(inner)
}

/// 输入框第一行：「❯」「›」开头（「>」要求上一行是横线、边框或空行），返回提示符后面的正文
fn prompt_rest<'a>(line: &'a str, above: Option<&str>) -> Option<&'a str> {
    let mut chars = line.chars();
    let marker = chars.next()?;
    let ok = match marker {
        '❯' | '›' => true,
        '>' => above.is_none_or(|up| is_rule(up) || unframed(up).trim().is_empty()),
        _ => false,
    };
    if !ok {
        return None;
    }
    let rest = chars.as_str();
    match rest.chars().next() {
        None => Some(""),
        Some(' ' | '\u{a0}') => Some(&rest[rest.chars().next()?.len_utf8()..]),
        Some(_) => None,
    }
}

/// 整行都是框线（「─」「╭」「╰」这类）
fn is_rule(line: &str) -> bool {
    let line = line.trim();
    !line.is_empty() && line.chars().all(|c| ('\u{2500}'..='\u{257f}').contains(&c))
}

/// 读焦点所在的整个输入框。`terminal`：前台是终端；`press` 发按键（读不到的程序要全选、复制）；
/// `owner` 是剪贴板的拥有者窗口。读不了返回给主人看的原因
#[cfg(windows)]
pub fn read(terminal: bool, press: &dyn Fn(&[Key]), owner: usize) -> Result<BoxText, String> {
    if terminal {
        let (lines, line, column) = terminal_screen().ok_or("读不到终端屏幕上的字".to_owned())?;
        return terminal_box(&lines, line, column)
            .ok_or_else(|| "没认出终端里的输入框（只认 Claude Code、Codex）".to_owned());
    }
    match edit_text() {
        Some(Ok(text)) => Ok(BoxText {
            text,
            caret: None,
            kind: Kind::Edit,
        }),
        Some(Err(reason)) => Err(reason),
        None => copied(press, owner),
    }
}

/// 焦点控件的 HTML 类名里带这些的，是代码编辑器、网页终端藏起来的输入区：
/// 读到的只是光标附近一截，全选却会选中整个文件，复制在网页终端里还是中断（Ctrl+C）
#[cfg(windows)]
const HIDDEN_INPUTS: [&str; 2] = ["inputarea", "xterm-helper-textarea"];

/// 控件类型：编辑框
#[cfg(windows)]
const EDIT: i32 = 50004;

/// 读一次最多等多久（UI 自动化跨进程，对方卡住时会一直等）
#[cfg(windows)]
const PATIENCE: std::time::Duration = std::time::Duration::from_millis(400);

/// UI 自动化读焦点输入框：读到了 Some(Ok)；是藏起来的输入区 Some(Err)；这种控件读不了 None（去全选复制）。
/// 隔一小会儿读两次，两次一样才算（刚发出去的退格、刚打的字可能还没落地）
#[cfg(windows)]
fn edit_text() -> Option<Result<String, String>> {
    let mut previous: Option<String> = None;
    for _ in 0..3 {
        let current = match read_once()? {
            Ok(text) => normalize(&text),
            Err(reason) => return Some(Err(reason)),
        };
        if previous.as_deref() == Some(current.as_str()) {
            return Some(Ok(current));
        }
        previous = Some(current);
        std::thread::sleep(std::time::Duration::from_millis(60));
    }
    None
}

#[cfg(windows)]
fn read_once() -> Option<Result<String, String>> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(with_automation(|automation| unsafe {
            element_text(&automation.GetFocusedElement().ok()?)
        }));
    });
    receiver.recv_timeout(PATIENCE).ok().flatten()
}

/// 在这个线程上开好 COM、建 UI 自动化对象，用完收掉
#[cfg(windows)]
fn with_automation<T>(
    read: impl FnOnce(&windows::Win32::UI::Accessibility::IUIAutomation) -> Option<T>,
) -> Option<T> {
    use windows::Win32::{
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        },
        UI::Accessibility::CUIAutomation,
    };
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok().ok()?;
        let result = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
            .ok()
            .and_then(|automation| read(&automation));
        CoUninitialize();
        result
    }
}

/// 一个控件里的全部文字：值能改的读值，编辑框读文字模式；藏起来的输入区 Err，别的、只读的 None
#[cfg(windows)]
unsafe fn element_text(
    element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
) -> Option<Result<String, String>> {
    use windows::Win32::UI::Accessibility::{
        IUIAutomationTextPattern, IUIAutomationValuePattern, UIA_TextPatternId, UIA_ValuePatternId,
    };
    unsafe {
        let class = element
            .CurrentClassName()
            .map(|name| name.to_string())
            .unwrap_or_default();
        if HIDDEN_INPUTS.iter().any(|hidden| class.contains(hidden)) {
            return Some(Err("这个输入框读不了（代码编辑器、网页终端）".into()));
        }
        if let Ok(value) =
            element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
        {
            if value.CurrentIsReadOnly().ok()?.as_bool() {
                return None;
            }
            return value.CurrentValue().ok().map(|text| Ok(text.to_string()));
        }
        if element.CurrentControlType().ok()?.0 != EDIT {
            return None;
        }
        let pattern = element
            .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
            .ok()?;
        let text = pattern.DocumentRange().ok()?.GetText(-1).ok()?;
        Some(Ok(text.to_string()))
    }
}

/// Windows Terminal：屏幕上看得见的每一行，以及光标在第几行第几个字（2026-11-24 探针验证过：
/// 终端控件有文字模式，没选中时「选中范围」就是光标位置）
#[cfg(windows)]
fn terminal_screen() -> Option<(Vec<String>, usize, usize)> {
    use windows::Win32::UI::Accessibility::{
        IUIAutomationTextPattern, TextPatternRangeEndpoint_End, TextPatternRangeEndpoint_Start,
        UIA_TextPatternId,
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(with_automation(|automation| unsafe {
            let element = automation.GetFocusedElement().ok()?;
            let pattern = element
                .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
                .ok()?;
            let ranges = pattern.GetVisibleRanges().ok()?;
            if ranges.Length().ok()? < 1 {
                return None;
            }
            let visible = ranges.GetElement(0).ok()?;
            let caret = pattern.GetSelection().ok()?.GetElement(0).ok()?;
            // 光标得在看得见的这一屏里
            let after_start = caret
                .CompareEndpoints(
                    TextPatternRangeEndpoint_Start,
                    &visible,
                    TextPatternRangeEndpoint_Start,
                )
                .ok()?
                >= 0;
            let before_end = caret
                .CompareEndpoints(
                    TextPatternRangeEndpoint_Start,
                    &visible,
                    TextPatternRangeEndpoint_End,
                )
                .ok()?
                <= 0;
            if !after_start || !before_end {
                return None;
            }
            let head = visible.Clone().ok()?;
            head.MoveEndpointByRange(
                TextPatternRangeEndpoint_End,
                &caret,
                TextPatternRangeEndpoint_Start,
            )
            .ok()?;
            let text = normalize(&visible.GetText(-1).ok()?.to_string());
            let head = normalize(&head.GetText(-1).ok()?.to_string());
            let line = head.matches('\n').count();
            let column = head.rsplit('\n').next().map_or(0, |l| l.chars().count());
            Some((text.split('\n').map(str::to_owned).collect(), line, column))
        }));
    });
    receiver
        .recv_timeout(std::time::Duration::from_millis(800))
        .ok()
        .flatten()
}

/// 读不到的程序（微信）：先存好剪贴板，全选、复制，读出来，按 Ctrl+End 取消选中，再把剪贴板还原。
/// 复制后剪贴板没变就是框里没字
#[cfg(windows)]
fn copied(press: &dyn Fn(&[Key]), owner: usize) -> Result<BoxText, String> {
    use std::time::{Duration, Instant};
    let saved = crate::clipboard::save(owner)?;
    let before = crate::clipboard::sequence();
    press(&[Key::SelectAll, Key::Copy]);
    let deadline = Instant::now() + Duration::from_millis(800);
    let mut changed = false;
    while Instant::now() < deadline {
        if crate::clipboard::sequence() != before {
            changed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    let text = if changed {
        // 有的程序先清空再分几次放，等它放完
        std::thread::sleep(Duration::from_millis(40));
        crate::clipboard::text(owner)
    } else {
        Some(String::new())
    };
    press(&[Key::End]);
    if changed {
        crate::clipboard::restore(owner, &saved)?;
    }
    text.map(|text| BoxText {
        text: normalize(&text),
        caret: None,
        kind: Kind::Copied,
    })
    .ok_or_else(|| "没读到输入框里的字".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(text: &str) -> BoxText {
        BoxText {
            text: text.into(),
            caret: None,
            kind: Kind::Edit,
        }
    }

    fn apply(doc: &str, keys: &[Key]) -> String {
        let mut doc = doc.to_string();
        crate::typing::apply_keys(&mut doc, keys);
        doc
    }

    // 主人 2026-11-24 定：「最后一句」= 从末尾往前删到上一个句号、问号、感叹号或换行
    #[test]
    fn the_last_sentence_runs_back_to_the_previous_full_stop_or_line_break() {
        for (text, left) in [
            ("今天天气不错。我们去公园吧。", "今天天气不错。"),
            ("今天天气不错。我们去公园吧", "今天天气不错。"),
            ("第一句？第二句！第三句", "第一句？第二句！"),
            ("第一行\n第二行。  \n\n", "第一行\n"),
            ("只有一句，没有句号", ""),
            ("Hello world. This is a test.", "Hello world."),
            ("Version 3.5 is out", ""),
            ("手打的。👨‍👩‍👧好", "手打的。"),
        ] {
            let count = last_sentence(text);
            let kept: String = text
                .graphemes(true)
                .take(text.graphemes(true).count() - count)
                .collect();
            assert_eq!(kept, left, "{text}");
        }
    }

    #[test]
    fn deleting_moves_to_the_end_first_and_reports_what_is_left() {
        let b = edit("手打的。雪耳说的。");
        let change = b.delete(VoiceCommand::DeleteLastSentence).unwrap();
        assert_eq!(change.keys, [Key::End, Key::Backspace(5)]);
        assert_eq!(change.after, "手打的。");
        let change = b.delete(VoiceCommand::DeleteAll).unwrap();
        assert_eq!(change.keys, [Key::SelectAll, Key::Backspace(1)]);
        assert_eq!(change.after, "");
        assert_eq!(
            edit("  \n").delete(VoiceCommand::DeleteAll),
            Err("输入框是空的".into())
        );
    }

    #[test]
    fn translating_replaces_the_whole_box() {
        let change = edit("手打的，说的话。").replace("Typed by hand, then spoken.");
        assert_eq!(
            change.keys,
            [
                Key::SelectAll,
                Key::Text("Typed by hand, then spoken.".into())
            ]
        );
        assert_eq!(
            apply("手打的，说的话。", &change.keys),
            "Typed by hand, then spoken."
        );
    }

    // 多行的译文一个字一个字打，换行在微信里就是回车，会直接发出去：整段粘贴
    #[test]
    fn a_translation_with_line_breaks_is_pasted_not_typed() {
        for kind in [Kind::Edit, Kind::Copied] {
            let b = BoxText {
                text: "第一段。\n第二段。".into(),
                caret: None,
                kind,
            };
            let change = b.replace("First.\r\nSecond.");
            assert_eq!(
                change.keys,
                [Key::SelectAll, Key::Paste("First.\nSecond.".into())]
            );
            assert_eq!(change.after, "First.\nSecond.");
            assert_eq!(apply(&b.text, &change.keys), "First.\nSecond.");
        }
    }

    fn screen(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|l| l.to_string()).collect()
    }

    // 2026-11-24 探针读到的 Claude Code：两条横线夹着「❯」那行，❯ 后面是不换行空格
    #[test]
    fn a_claude_code_prompt_is_found_around_the_caret() {
        let lines = screen(&[
            "● 上一条回复",
            "",
            "────────────────────────────────────────",
            "❯\u{a0}帮我把这个仓库推到 GitHub 上，然后打一个新",
            "  的标签。",
            "────────────────────────────────────────",
            "  ? for shortcuts",
        ]);
        let b = terminal_box(&lines, 4, 7).unwrap();
        assert_eq!(b.text, "帮我把这个仓库推到 GitHub 上，然后打一个新的标签。");
        assert_eq!(b.caret, Some(b.text.graphemes(true).count()));
        assert_eq!(b.kind, Kind::Terminal);
        // 删最后一句：光标已经在最后，按字数退格
        let change = b.delete(VoiceCommand::DeleteLastSentence).unwrap();
        assert_eq!(
            change.keys,
            [Key::Backspace(b.text.graphemes(true).count())]
        );
        // 光标在中间：先往右挪到最后
        let b = terminal_box(&lines, 3, 4).unwrap();
        assert_eq!(b.caret, Some(2));
        let change = b.delete(VoiceCommand::DeleteAll).unwrap();
        let total = b.text.graphemes(true).count();
        assert_eq!(
            change.keys,
            [Key::Right(total - 2), Key::Backspace(total + 2)]
        );
    }

    // Codex：上下是空行，第一行以「›」开头（openai/codex 的 draft_composer 快照）
    #[test]
    fn a_codex_prompt_is_found_too() {
        let lines = screen(&[
            "• Ran tests",
            "",
            "› Explore the night sky",
            "  and the stars",
            "",
            "  100% context left",
        ]);
        let b = terminal_box(&lines, 3, 16).unwrap();
        assert_eq!(b.text, "Explore the night sky and the stars");
        assert_eq!(b.caret, Some(b.text.chars().count()));
        // 译文在终端里不带换行
        let change = b.replace("探索夜空\n和星星");
        assert_eq!(
            change.keys.last(),
            Some(&Key::Text("探索夜空 和星星".into()))
        );
    }

    #[test]
    fn an_empty_prompt_or_its_grey_hint_counts_as_empty() {
        let lines = screen(&["────", "❯\u{a0}", "────"]);
        let b = terminal_box(&lines, 1, 2).unwrap();
        assert_eq!(b.text, "");
        // 光标在提示语前面
        let lines = screen(&["", "› Ask Codex to do anything", ""]);
        let b = terminal_box(&lines, 1, 2).unwrap();
        assert_eq!(b.text, "");
        assert!(b.delete(VoiceCommand::DeleteAll).is_err());
    }

    #[test]
    fn a_framed_old_style_prompt_is_found() {
        let lines = screen(&[
            "╭──────────────────────╮",
            "│ > 第一行              │",
            "│   第二行              │",
            "╰──────────────────────╯",
        ]);
        let b = terminal_box(&lines, 2, 7).unwrap();
        assert_eq!(b.text, "第一行第二行");
    }

    #[test]
    fn a_plain_shell_or_a_caret_outside_any_prompt_is_not_a_box() {
        let lines = screen(&["user@host:~$ ls -la", ""]);
        assert_eq!(terminal_box(&lines, 0, 19), None);
        // 光标在输入框下面的提示行里
        let lines = screen(&["────", "❯\u{a0}你好", "────", "  ? for shortcuts"]);
        assert_eq!(terminal_box(&lines, 3, 5), None);
        // 「>」开头的普通输出不算（上一行不是横线或空行）
        let lines = screen(&["引用如下：", "> 这是一句引用"]);
        assert_eq!(terminal_box(&lines, 1, 4), None);
    }
}

/// 在 Windows 上真用 UI 自动化读系统输入框（窗口建在屏幕外、不显示，不抢焦点）
#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use windows_sys::Win32::{
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, DispatchMessageW, ES_MULTILINE, ES_READONLY,
            GetMessageW, MSG, PostThreadMessageW, WM_QUIT, WS_POPUP,
        },
    };

    /// 同一进程里几个测试同时用 UI 自动化读自己建的输入框会互相干扰（并行跑时偶尔读不到），一个一个来
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// 在单独的线程上建一个输入框并转消息循环（UI 自动化读它时要这个线程回消息），读完关掉
    fn read_edit(text: &str, style: u32) -> Option<String> {
        let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let (sender, receiver) = std::sync::mpsc::channel();
        let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let window = std::thread::spawn(move || unsafe {
            let class: Vec<u16> = "EDIT".encode_utf16().chain(Some(0)).collect();
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                text.as_ptr(),
                WS_POPUP | style,
                -32000,
                -32000,
                300,
                120,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            );
            let thread = windows_sys::Win32::System::Threading::GetCurrentThreadId();
            sender.send((hwnd as isize, thread)).unwrap();
            let mut message: MSG = std::mem::zeroed();
            while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                DispatchMessageW(&message);
            }
            DestroyWindow(hwnd);
        });
        let (hwnd, thread) = receiver.recv().unwrap();
        assert_ne!(hwnd, 0, "建不了输入框");
        let text = with_automation(|automation| unsafe {
            let element = automation
                .ElementFromHandle(windows::Win32::Foundation::HWND(hwnd as *mut _))
                .ok()?;
            element_text(&element)?.ok()
        });
        unsafe {
            PostThreadMessageW(thread, WM_QUIT, 0, 0);
        }
        window.join().unwrap();
        text.map(|t| normalize(&t))
    }

    #[test]
    fn a_single_line_box_is_read_through_its_value() {
        assert_eq!(
            read_edit("手打的，说的话。", 0).as_deref(),
            Some("手打的，说的话。")
        );
    }

    #[test]
    fn a_multi_line_box_is_read_through_its_text() {
        assert_eq!(
            read_edit("第一行\r\n第二行", ES_MULTILINE as u32).as_deref(),
            Some("第一行\n第二行")
        );
    }

    #[test]
    fn a_read_only_box_is_not_used() {
        assert_eq!(read_edit("只读的", ES_READONLY as u32), None);
    }
}
