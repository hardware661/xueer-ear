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
/// 浏览器的窗口类名（Chrome、Edge、360 这类 Chromium 内核的，和 Firefox）。只读文档只在浏览器里才算
/// 「没点进输入框的网页」；别的程序的输入框万一也报成文档，不能因此不打字
const BROWSERS: [&str; 2] = ["Chrome_WidgetWin_1", "MozillaWindowClass"];

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
        EDIT if control.read_only == Some(true) => Focus::NoText,
        DOCUMENT if control.read_only == Some(true) && BROWSERS.contains(&window_class) => {
            Focus::NoText
        }
        kind if NOT_TEXT.contains(&kind) => Focus::NoText,
        _ => Focus::Text,
    }
}

/// 一轮开始时在后台查焦点，第一次要上屏时再取结果，不拖慢开麦
pub struct Probe {
    #[cfg(windows)]
    pending: Option<(std::sync::mpsc::Receiver<Focus>, std::time::Instant)>,
    result: Option<Focus>,
}

impl Probe {
    /// 已经知道结果的（测试用）
    #[cfg(test)]
    pub fn ready(focus: Focus) -> Self {
        Self {
            #[cfg(windows)]
            pending: None,
            result: Some(focus),
        }
    }

    /// 结果；还没查完就等到开始后 `PATIENCE` 为止，拿不到当「能打字」
    pub fn focus(&mut self) -> Focus {
        #[cfg(windows)]
        if self.result.is_none()
            && let Some((receiver, started)) = self.pending.take()
        {
            let left = PATIENCE.saturating_sub(started.elapsed());
            self.result = Some(receiver.recv_timeout(left).unwrap_or(Focus::Text));
        }
        self.result.unwrap_or(Focus::Text)
    }
}

/// 这一轮的字往哪儿打。第一次上屏时看焦点：没有输入框就不发按键，字攒在 `held` 里（同样的退格、换行），
/// 说完复制到剪贴板；主人中途点进了能打字的地方，后面的字照常打过去，攒下的照样复制
/// （动过键盘鼠标的那句已经冻结，只往后接字，不会退格删到新输入框里的东西）
pub struct Delivery {
    probe: Option<Probe>,
    held: Option<String>,
    holding: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    /// 照常发到屏幕上
    Screen,
    /// 攒起来了；`started` 是这一轮第一次攒（该在面板上说一声）
    Held { started: bool },
}

impl Delivery {
    /// `probe` 为 None 时一直照常打字（终端、录音文件）
    pub fn new(probe: Option<Probe>) -> Self {
        Self {
            probe,
            held: None,
            holding: false,
        }
    }

    pub fn route(&mut self, keys: &[crate::typing::Key]) -> Route {
        let mut started = false;
        if let Some(mut probe) = self.probe.take()
            && probe.focus() == Focus::NoText
        {
            self.holding = true;
            started = true;
        }
        if !self.holding {
            return Route::Screen;
        }
        crate::typing::apply_keys(self.held.get_or_insert_with(String::new), keys);
        Route::Held { started }
    }

    /// 主人动了键盘或鼠标：还没上过屏就重查；正在攒字的话，点进了能打字的地方就改回照常打字
    pub fn touched(&mut self, probe: impl FnOnce() -> Probe) {
        if self.probe.is_some() {
            self.probe = Some(probe());
        } else if self.holding && probe().focus() == Focus::Text {
            self.holding = false;
        }
    }

    /// 这一轮正在攒字（前台没有输入框）
    pub fn holding(&self) -> bool {
        self.holding
    }

    /// 这一轮攒下的字（没攒或者是空的就 None）
    pub fn held(&self) -> Option<&str> {
        self.held.as_deref().filter(|text| !text.trim().is_empty())
    }
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
        // 只读文档只在浏览器里算没有输入框
        assert_eq!(
            classify("Qt51514QWindowIcon", control(DOCUMENT, Some(true))),
            Focus::Text
        );
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

    use crate::typing::Key;

    fn text(t: &str) -> Vec<Key> {
        vec![Key::Text(t.into())]
    }

    #[test]
    fn with_an_input_box_or_no_check_everything_goes_to_the_screen() {
        let mut d = Delivery::new(None);
        assert_eq!(d.route(&text("a")), Route::Screen);
        let mut d = Delivery::new(Some(Probe::ready(Focus::Text)));
        assert_eq!(d.route(&text("a")), Route::Screen);
        assert_eq!(d.held(), None);
    }

    #[test]
    fn without_an_input_box_the_round_is_held_for_copying() {
        let mut d = Delivery::new(Some(Probe::ready(Focus::NoText)));
        assert_eq!(d.route(&text("第一句")), Route::Held { started: true });
        assert_eq!(
            d.route(&[Key::Backspace(1), Key::Enter]),
            Route::Held { started: false }
        );
        assert_eq!(d.held(), Some("第一\n"));
    }

    #[test]
    fn clicking_into_an_input_box_sends_the_rest_to_the_screen() {
        let mut d = Delivery::new(Some(Probe::ready(Focus::NoText)));
        d.route(&text("攒着的"));
        // 点了别处但还是没有输入框
        d.touched(|| Probe::ready(Focus::NoText));
        assert_eq!(d.route(&text("还攒")), Route::Held { started: false });
        d.touched(|| Probe::ready(Focus::Text));
        assert_eq!(d.route(&text("打出去")), Route::Screen);
        assert_eq!(d.held(), Some("攒着的还攒"), "攒下的照样复制");
    }

    #[test]
    fn a_click_before_anything_was_typed_checks_the_focus_again() {
        let mut d = Delivery::new(Some(Probe::ready(Focus::NoText)));
        d.touched(|| Probe::ready(Focus::Text));
        assert_eq!(d.route(&text("a")), Route::Screen);
    }
}
