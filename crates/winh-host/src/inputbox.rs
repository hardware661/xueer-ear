//! 说「翻译」时读焦点输入框里的全部文字（UI 自动化），整框翻译、整框换成译文
//! （主人 2026-11-24 选的「输入框里全部的字」：上一轮说的、手打的都算）。
//! 读不到、读到的不可信时退回去翻雪耳在这个框里打过的字（`Live::typed_text`，跨轮记着）。
//!
//! 只信这几种读法：值能改的输入框（值模式），或者控件类型是编辑框、带文字模式的（富文本、网页里的可编辑区）。
//! 读到的必须包含雪耳刚打的那句（不然多半读到了别的控件）；代码编辑器藏起来的输入区
//! （VS Code 的 Monaco、xterm）读到的只是光标附近一截，全选却会选中整个文件，一律不信。

/// 输入框里最多整框翻多少字：再多就不整框换了（打字要很久，也可能是一整篇文档）
pub const MAX_CHARS: usize = 2000;

/// 读到的整框文字能不能拿去翻。`typed` 是雪耳打过、还该在框里的字：框里得找得到它的最后一句。
/// 能翻返回整理过换行的原文；空的、对不上的返回 `Ok(None)`（退回去翻雪耳打的字）；太长的返回原因
pub fn usable(read: &str, typed: Option<&str>) -> Result<Option<String>, String> {
    let text = normalize(read);
    if text.trim().is_empty() {
        return Ok(None);
    }
    if let Some(typed) = typed
        && !contains_loosely(&text, last_line(typed))
    {
        return Ok(None);
    }
    if text.chars().count() > MAX_CHARS {
        return Err(format!("输入框里超过 {MAX_CHARS} 字，没翻；可以用翻译小窗"));
    }
    Ok(Some(text))
}

/// 换行统一成 \n（Windows 的输入框给的是 \r\n）
fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// 雪耳打的最后一行（框里的字可能被程序改过换行，只拿最后一行比）
fn last_line(typed: &str) -> &str {
    typed
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
}

/// 去掉空白后 `haystack` 里有没有 `needle`（有的程序会把句间空格、换行改掉）
fn contains_loosely(haystack: &str, needle: &str) -> bool {
    let squeeze = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let needle = squeeze(needle);
    needle.is_empty() || squeeze(haystack).contains(&needle)
}

/// 焦点控件的 HTML 类名里带这些的，是代码编辑器、网页终端藏起来的输入区：读到的不是整框
#[cfg(windows)]
const HIDDEN_INPUTS: [&str; 2] = ["inputarea", "xterm-helper-textarea"];

/// 控件类型：编辑框
#[cfg(windows)]
const EDIT: i32 = 50004;

/// 读一次最多等多久（UI 自动化跨进程，对方卡住时会一直等）
#[cfg(windows)]
const PATIENCE: std::time::Duration = std::time::Duration::from_millis(400);

/// 读焦点输入框的全部文字（换行统一成 \n）；读不到、超时、不可信的读法返回 None。
/// 隔一小会儿读两次，两次一样才算（刚发出去的退格、刚打的字可能还没落地）
#[cfg(windows)]
pub fn read() -> Option<String> {
    let mut previous: Option<String> = None;
    for _ in 0..3 {
        let current = normalize(&read_once()?);
        if previous.as_deref() == Some(current.as_str()) {
            return Some(current);
        }
        previous = Some(current);
        std::thread::sleep(std::time::Duration::from_millis(60));
    }
    None
}

#[cfg(windows)]
fn read_once() -> Option<String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(focused_text());
    });
    receiver.recv_timeout(PATIENCE).ok().flatten()
}

#[cfg(windows)]
fn focused_text() -> Option<String> {
    with_automation(|automation| unsafe { element_text(&automation.GetFocusedElement().ok()?) })
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

/// 一个控件里的全部文字：值能改的读值，编辑框读文字模式；别的、只读的、藏起来的输入区返回 None
#[cfg(windows)]
unsafe fn element_text(
    element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
) -> Option<String> {
    use windows::Win32::UI::Accessibility::{
        IUIAutomationTextPattern, IUIAutomationValuePattern, UIA_TextPatternId, UIA_ValuePatternId,
    };
    unsafe {
        let class = element
            .CurrentClassName()
            .map(|name| name.to_string())
            .unwrap_or_default();
        if HIDDEN_INPUTS.iter().any(|hidden| class.contains(hidden)) {
            return None;
        }
        if let Ok(value) =
            element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
        {
            if value.CurrentIsReadOnly().ok()?.as_bool() {
                return None;
            }
            return value.CurrentValue().ok().map(|text| text.to_string());
        }
        if element.CurrentControlType().ok()?.0 != EDIT {
            return None;
        }
        let pattern = element
            .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
            .ok()?;
        let text = pattern.DocumentRange().ok()?.GetText(-1).ok()?;
        Some(text.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_whole_box_is_used_when_it_holds_what_xueer_typed() {
        assert_eq!(
            usable("手打的，说的话。", Some("说的话。")),
            Ok(Some("手打的，说的话。".to_owned()))
        );
        // 主人只说了「翻译」、这轮没打字：框里手打的照样翻
        assert_eq!(usable("手打的", None), Ok(Some("手打的".to_owned())));
        // 换行统一成 \n；程序把句间空格、换行改了也认
        assert_eq!(
            usable("第一行\r\n第二行", Some("第一行\n第二行")),
            Ok(Some("第一行\n第二行".to_owned()))
        );
        assert_eq!(
            usable("Hello world. 你好", Some("Hello world.\n你好")),
            Ok(Some("Hello world. 你好".to_owned()))
        );
    }

    #[test]
    fn empty_or_unrelated_reads_fall_back_to_what_xueer_typed() {
        assert_eq!(usable("  \n ", Some("说的话")), Ok(None));
        // 读到的不是这个框（找不到雪耳刚打的那句）
        assert_eq!(usable("网页上别的字", Some("说的话。")), Ok(None));
    }

    #[test]
    fn a_huge_box_is_not_replaced() {
        let long = "字".repeat(MAX_CHARS + 1);
        assert!(usable(&long, None).unwrap_err().contains("翻译小窗"));
        assert!(usable(&"字".repeat(MAX_CHARS), None).unwrap().is_some());
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
            element_text(&element)
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
