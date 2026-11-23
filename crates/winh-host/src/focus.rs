//! 前台此刻有没有能打字的地方。拿不准一律当「能打字」（照常上屏），只有很确定没有输入框时，
//! 这一轮才不发按键、说完把文字复制到剪贴板：
//! 桌面、任务栏；焦点在按钮、列表项、菜单、树节点这类控件上；只读的文档（没点进输入框的网页）。
//! 不发按键还免得字母被当成快捷键（文件列表里按字母会跳着选文件、网页里有单键快捷键）。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Text,
    NoText,
}

/// UI 自动化读到的焦点控件：控件类型（UIA_*ControlTypeId 的值），值是不是只读（不支持值模式时 None）
#[derive(Debug, Clone, Copy)]
pub struct Control {
    pub kind: i32,
    pub read_only: Option<bool>,
}

const EDIT: i32 = 50004;
const DOCUMENT: i32 = 50030;
/// 肯定不是输入框的控件类型：按钮、日历、复选框、链接、图片、列表项、列表、菜单、菜单栏、菜单项、进度条、
/// 单选框、滚动条、滑块、状态栏、选项卡、选项卡项、工具栏、树、树节点、滑块头、数据项、拆分按钮、表头、
/// 表头项、标题栏、分隔线。下拉框、文字、窗格、窗口、自定义这些拿不准，不在里面
const NOT_TEXT: [i32; 27] = [
    50000, 50001, 50002, 50005, 50006, 50007, 50008, 50009, 50010, 50011, 50012, 50013, 50014,
    50015, 50017, 50018, 50019, 50021, 50023, 50024, 50027, 50029, 50031, 50034, 50035, 50037,
    50038,
];
/// 桌面和任务栏的窗口类名
const SHELL: [&str; 4] = [
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
];

pub fn classify(window_class: &str, control: Option<Control>) -> Focus {
    if SHELL.contains(&window_class) {
        return Focus::NoText;
    }
    let Some(control) = control else {
        return Focus::Text;
    };
    // 值能改的（列表项正在重命名、表格格子在编辑）一律算能打字
    if control.read_only == Some(false) {
        return Focus::Text;
    }
    match control.kind {
        EDIT | DOCUMENT if control.read_only == Some(true) => Focus::NoText,
        kind if NOT_TEXT.contains(&kind) => Focus::NoText,
        _ => Focus::Text,
    }
}

/// 一轮开始时在后台查焦点，第一次要上屏时再取结果，不拖慢开麦
#[cfg(windows)]
pub struct Probe {
    pending: Option<(std::sync::mpsc::Receiver<Focus>, std::time::Instant)>,
    result: Option<Focus>,
}

/// 查焦点最多等多久（UI 自动化跨进程，对方卡住时会一直等）
#[cfg(windows)]
const PATIENCE: std::time::Duration = std::time::Duration::from_millis(300);

#[cfg(windows)]
impl Probe {
    pub fn start() -> Self {
        let class = foreground_class();
        if SHELL.contains(&class.as_str()) {
            return Self {
                pending: None,
                result: Some(Focus::NoText),
            };
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(classify(&class, focused_control()));
        });
        Self {
            pending: Some((receiver, std::time::Instant::now())),
            result: None,
        }
    }

    /// 结果；还没查完就等到开始后 `PATIENCE` 为止，拿不到当「能打字」
    pub fn focus(&mut self) -> Focus {
        if self.result.is_none()
            && let Some((receiver, started)) = self.pending.take()
        {
            let left = PATIENCE.saturating_sub(started.elapsed());
            self.result = Some(receiver.recv_timeout(left).unwrap_or(Focus::Text));
        }
        self.result.unwrap_or(Focus::Text)
    }
}

/// 排查用：前台窗口类名、焦点控件类型、值是否只读，以及判断结果
#[cfg(windows)]
pub fn describe() -> String {
    let class = foreground_class();
    let control = focused_control();
    format!(
        "窗口类 {class}；焦点 {}；判断：{}",
        control.map_or("读不到".to_owned(), |c| format!(
            "控件类型 {}，只读 {:?}",
            c.kind, c.read_only
        )),
        match classify(&class, control) {
            Focus::Text => "能打字",
            Focus::NoText => "没有输入框（说完复制）",
        }
    )
}

#[cfg(windows)]
fn foreground_class() -> String {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow};
    let window = unsafe { GetForegroundWindow() };
    if window.is_null() {
        return "Progman".into();
    }
    let mut buffer = [0_u16; 128];
    let len = unsafe { GetClassNameW(window, buffer.as_mut_ptr(), buffer.len() as i32) }.max(0);
    String::from_utf16_lossy(&buffer[..len as usize])
}

/// 用 UI 自动化读焦点控件的类型和值是否只读；读不到返回 None
#[cfg(windows)]
fn focused_control() -> Option<Control> {
    use windows::{
        Win32::{
            System::Com::{
                CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize,
            },
            UI::Accessibility::{
                CUIAutomation, IUIAutomation, IUIAutomationValuePattern, UIA_ValuePatternId,
            },
        },
        core::Interface,
    };
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok().ok()?;
        let read = || -> Option<Control> {
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
            let element = automation.GetFocusedElement().ok()?;
            let kind = element.CurrentControlType().ok()?.0;
            let read_only = element
                .GetCurrentPattern(UIA_ValuePatternId)
                .ok()
                .and_then(|pattern| pattern.cast::<IUIAutomationValuePattern>().ok())
                .and_then(|value| value.CurrentIsReadOnly().ok())
                .map(|flag| flag.as_bool());
            Some(Control { kind, read_only })
        };
        let control = read();
        CoUninitialize();
        control
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn control(kind: i32, read_only: Option<bool>) -> Option<Control> {
        Some(Control { kind, read_only })
    }

    #[test]
    fn the_desktop_and_taskbar_have_no_input_box() {
        assert_eq!(classify("Progman", None), Focus::NoText);
        assert_eq!(
            classify("Shell_TrayWnd", control(EDIT, None)),
            Focus::NoText
        );
    }

    #[test]
    fn buttons_lists_and_menus_are_not_input_boxes() {
        for kind in [50000, 50007, 50011, 50024] {
            assert_eq!(
                classify("CabinetWClass", control(kind, None)),
                Focus::NoText
            );
        }
        // 只读的网页、只读的编辑框
        assert_eq!(
            classify("Chrome_WidgetWin_1", control(DOCUMENT, Some(true))),
            Focus::NoText
        );
        assert_eq!(
            classify("Notepad", control(EDIT, Some(true))),
            Focus::NoText
        );
    }

    #[test]
    fn anything_editable_or_unclear_counts_as_an_input_box() {
        assert_eq!(classify("Notepad", control(EDIT, Some(false))), Focus::Text);
        assert_eq!(classify("Notepad", control(EDIT, None)), Focus::Text);
        assert_eq!(classify("OpusApp", control(DOCUMENT, None)), Focus::Text);
        // 列表项正在重命名：值能改
        assert_eq!(
            classify("CabinetWClass", control(50007, Some(false))),
            Focus::Text
        );
        // 窗格、窗口、自定义、文字（终端常报这个）、下拉框：拿不准
        for kind in [50033, 50032, 50025, 50020, 50003] {
            assert_eq!(classify("Game", control(kind, None)), Focus::Text, "{kind}");
        }
        // 读不到焦点（对方卡住、权限不够）
        assert_eq!(classify("Qt51514QWindowIcon", None), Focus::Text);
    }
}
