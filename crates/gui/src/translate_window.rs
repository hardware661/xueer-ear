//! 翻译小窗（主人 2026-11-19：面板右边的「译」按钮弹出；第一版系统方框太丑，按新设计重画）。
//! 一张白色圆角卡片（系统圆角 + 淡阴影，没有标题栏），放在听写面板正下方：
//! 上半输入（无边框、淡灰提示字），中间细分隔线 + 「中 → 英」小标签（自动认语言），
//! 下半译文（字稍大稍深），翻译时三个跳动小点；右下「复制」文字按钮点完变「已复制 ✓」；
//! 右上小 ×，Esc 也能关；按住卡片空白处可以拖。停手约半秒自动翻译，Ctrl+Enter 立刻翻。
//! 用 `xueer-ear.exe --translate` 打开；已经开着就切到前面。
#![allow(unsafe_op_in_unsafe_fn)]
use std::{mem, ptr, sync::mpsc, thread};
use windows_sys::Win32::{
    Foundation::{GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::{
        BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreateSolidBrush,
        DEFAULT_CHARSET, DEFAULT_PITCH, DT_CENTER, DT_SINGLELINE, DT_VCENTER, DrawTextW, EndPaint,
        FillRect, HDC, HFONT, InvalidateRect, OUT_DEFAULT_PRECIS, PAINTSTRUCT, ScreenToClient,
        SelectObject, SetBkColor, SetBkMode, SetTextColor, TRANSPARENT,
    },
    System::{LibraryLoader::GetModuleHandleW, Threading::CreateMutexW},
    UI::{
        HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow,
            SetProcessDpiAwarenessContext,
        },
        Input::KeyboardAndMouse::{GetKeyState, SetFocus, VK_CONTROL},
        WindowsAndMessaging::*,
    },
};
use xueer_ear_backend::translate::Target;

const CLASS: &str = "XueerEar.TranslateWindow";
const SOURCE: usize = 401;
const RESULT: usize = 402;
const EN_CHANGE: u32 = 0x0300;
const EM_SETLIMITTEXT: u32 = 0x00C5;
const EM_SETCUEBANNER: u32 = 0x1501;
const EM_SETRECT: u32 = 0x00B3;
const ES_MULTILINE: u32 = 0x0004;
const ES_AUTOVSCROLL: u32 = 0x0040;
const ES_READONLY: u32 = 0x0800;
const ES_WANTRETURN: u32 = 0x1000;
/// 停手这么久就自动翻译
const SETTLE_MS: u32 = 600;
const SETTLE_TIMER: usize = 1;
/// 翻译中小点的动画
const DOTS_TIMER: usize = 2;
/// 「已复制 ✓」显示多久
const COPIED_TIMER: usize = 3;
/// 译文回来了
const DONE: u32 = WM_APP + 21;

/// 卡片尺寸（96 DPI）
const WIDTH: i32 = 440;
const HEIGHT: i32 = 330;
/// 输入区、分隔线、译文区、底栏
const PAD: i32 = 20;
const SOURCE_TOP: i32 = 44;
const DIVIDER: i32 = 168;
const RESULT_TOP: i32 = 184;
const FOOTER: i32 = 290;
/// 右上 ×、右下「复制」的点击区
const CLOSE_BOX: (i32, i32, i32, i32) = (WIDTH - 40, 8, 28, 28);
const COPY_BOX: (i32, i32, i32, i32) = (WIDTH - 110, FOOTER, 90, 28);

struct State {
    dpi: u32,
    fonts: [HFONT; 3],
    white: isize,
    asked: usize,
    replies: mpsc::Receiver<(usize, Result<String, String>)>,
    sender: mpsc::Sender<(usize, Result<String, String>)>,
    last_source: String,
    busy: bool,
    phase: f32,
    /// 底栏左边的一句提示（失败原因）
    note: String,
    copied: bool,
    target: Target,
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn scale(value: i32, dpi: u32) -> i32 {
    (value * dpi as i32 + 48) / 96
}

unsafe fn text_of(window: HWND, id: usize) -> String {
    let control = GetDlgItem(window, id as i32);
    let len = GetWindowTextLengthW(control).max(0) as usize;
    let mut buffer = vec![0_u16; len + 1];
    let got = GetWindowTextW(control, buffer.as_mut_ptr(), buffer.len() as i32).max(0);
    String::from_utf16_lossy(&buffer[..got as usize])
}

unsafe fn set_text(window: HWND, id: usize, text: &str) {
    let text = text.replace("\r\n", "\n").replace('\n', "\r\n");
    SetWindowTextW(GetDlgItem(window, id as i32), wide(&text).as_ptr());
}

fn inside(point: POINT, (x, y, w, h): (i32, i32, i32, i32), dpi: u32) -> bool {
    point.x >= scale(x, dpi)
        && point.x < scale(x + w, dpi)
        && point.y >= scale(y, dpi)
        && point.y < scale(y + h, dpi)
}

unsafe fn ask(window: HWND, state: &mut State) {
    let source = text_of(window, SOURCE);
    state.target = Target::for_text(&source);
    if source.trim().is_empty() {
        set_text(window, RESULT, "");
        state.last_source.clear();
        state.note.clear();
        state.busy = false;
        InvalidateRect(window, ptr::null(), 1);
        return;
    }
    if source == state.last_source {
        return;
    }
    state.last_source = source.clone();
    state.asked += 1;
    state.busy = true;
    state.note.clear();
    // 翻译中：收起译文框，露出底下跳动的小点
    ShowWindow(GetDlgItem(window, RESULT as i32), SW_HIDE);
    SetTimer(window, DOTS_TIMER, 60, None);
    InvalidateRect(window, ptr::null(), 1);
    let (id, sender, target) = (state.asked, state.sender.clone(), window as usize);
    thread::spawn(move || {
        let result = xueer_ear_backend::translate::translate(&source);
        let _ = sender.send((id, result));
        PostMessageW(target as HWND, DONE, id, 0);
    });
}

unsafe fn paint(window: HWND, state: &State) {
    let mut ps: PAINTSTRUCT = mem::zeroed();
    let dc = BeginPaint(window, &mut ps);
    let mut client: RECT = mem::zeroed();
    GetClientRect(window, &mut client);
    FillRect(dc, &client, state.white as _);
    let label = match state.target {
        Target::English => "中 → 英",
        Target::Chinese => "英 → 中",
    };
    let pill = ((WIDTH / 2 - 34) as f32, (DIVIDER - 11) as f32, 68., 22.);
    crate::view::translate_card(
        dc,
        state.dpi,
        client.right,
        client.bottom,
        DIVIDER as f32,
        pill,
        state.busy.then_some(state.phase),
    );
    SetBkMode(dc, TRANSPARENT as i32);
    let d = state.dpi;
    let draw = |text: &str, font: HFONT, color: u32, (x, y, w, h): (i32, i32, i32, i32)| {
        let old = SelectObject(dc, font);
        SetTextColor(dc, color);
        let mut rect = RECT {
            left: scale(x, d),
            top: scale(y, d),
            right: scale(x + w, d),
            bottom: scale(y + h, d),
        };
        let units: Vec<u16> = text.encode_utf16().collect();
        DrawTextW(
            dc,
            units.as_ptr(),
            units.len() as i32,
            &mut rect,
            DT_SINGLELINE | DT_VCENTER | if w < 120 { DT_CENTER } else { 0 },
        );
        SelectObject(dc, old);
    };
    // 颜色是 GDI 的 0x00BBGGRR
    draw("翻译", state.fonts[2], 0x001e1c1c, (PAD, 12, 120, 24));
    draw("×", state.fonts[0], 0x00938e8e, CLOSE_BOX);
    draw(
        label,
        state.fonts[1],
        0x00706c6c,
        (pill.0 as i32, pill.1 as i32, 68, 22),
    );
    if !state.note.is_empty() {
        draw(
            &state.note,
            state.fonts[1],
            0x003f39bd,
            (PAD, FOOTER, 280, 28),
        );
    }
    let copy = if state.copied {
        "已复制 ✓"
    } else {
        "复制"
    };
    draw(copy, state.fonts[1], 0x00403c3a, COPY_BOX);
    EndPaint(window, &ps);
}

unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let creation = &*(lparam as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, creation.lpCreateParams as isize);
    }
    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
    if raw.is_null() {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    let state = &mut *raw;
    match message {
        WM_NCCALCSIZE => 0,
        WM_NCHITTEST => {
            let mut point = POINT {
                x: (lparam as i16) as i32,
                y: ((lparam >> 16) as i16) as i32,
            };
            ScreenToClient(hwnd, &mut point);
            // ×、复制是按钮；其余空白处按住可以拖
            if inside(point, CLOSE_BOX, state.dpi) || inside(point, COPY_BOX, state.dpi) {
                HTCLIENT as isize
            } else {
                HTCAPTION as isize
            }
        }
        WM_LBUTTONUP => {
            let point = POINT {
                x: (lparam as i16) as i32,
                y: ((lparam >> 16) as i16) as i32,
            };
            if inside(point, CLOSE_BOX, state.dpi) {
                DestroyWindow(hwnd);
            } else if inside(point, COPY_BOX, state.dpi) {
                let result = text_of(hwnd, RESULT);
                if result.is_empty() {
                    state.note = "还没有译文".into();
                } else {
                    match crate::app::copy_text(hwnd, &result) {
                        Ok(()) => {
                            state.copied = true;
                            state.note.clear();
                            SetTimer(hwnd, COPIED_TIMER, 1200, None);
                        }
                        Err(reason) => state.note = reason,
                    }
                }
                InvalidateRect(hwnd, ptr::null(), 1);
            }
            0
        }
        WM_SETCURSOR => {
            let mut point = mem::zeroed();
            GetCursorPos(&mut point);
            ScreenToClient(hwnd, &mut point);
            if inside(point, CLOSE_BOX, state.dpi) || inside(point, COPY_BOX, state.dpi) {
                SetCursor(LoadCursorW(ptr::null_mut(), IDC_HAND));
                return 1;
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_COMMAND => {
            let (id, code) = (wparam & 0xffff, (wparam >> 16) as u32 & 0xffff);
            if id == SOURCE && code == EN_CHANGE {
                SetTimer(hwnd, SETTLE_TIMER, SETTLE_MS, None);
                let target = Target::for_text(&text_of(hwnd, SOURCE));
                if target != state.target {
                    state.target = target;
                    InvalidateRect(hwnd, ptr::null(), 1);
                }
            }
            0
        }
        WM_TIMER => {
            match wparam {
                SETTLE_TIMER => {
                    KillTimer(hwnd, SETTLE_TIMER);
                    ask(hwnd, state);
                }
                DOTS_TIMER => {
                    state.phase += 0.35;
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
                _ => {
                    KillTimer(hwnd, COPIED_TIMER);
                    state.copied = false;
                    InvalidateRect(hwnd, ptr::null(), 1);
                }
            }
            0
        }
        DONE => {
            while let Ok((id, result)) = state.replies.try_recv() {
                if id != state.asked {
                    continue;
                }
                state.busy = false;
                KillTimer(hwnd, DOTS_TIMER);
                ShowWindow(GetDlgItem(hwnd, RESULT as i32), SW_SHOWNOACTIVATE);
                match result {
                    Ok(text) => set_text(hwnd, RESULT, &text),
                    Err(reason) => state.note = format!("翻译失败：{reason}"),
                }
            }
            InvalidateRect(hwnd, ptr::null(), 1);
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint(hwnd, state);
            0
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            let dc = wparam as HDC;
            SetBkColor(dc, 0x00ffffff);
            // 译文比输入稍深
            let result = lparam as HWND == GetDlgItem(hwnd, RESULT as i32);
            SetTextColor(dc, if result { 0x001e1c1c } else { 0x00403c3a });
            state.white
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

unsafe fn font(size: i32, weight: i32, dpi: u32, family: &str) -> HFONT {
    CreateFontW(
        -scale(size, dpi),
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        OUT_DEFAULT_PRECIS as u32,
        CLIP_DEFAULT_PRECIS as u32,
        CLEARTYPE_QUALITY as u32,
        DEFAULT_PITCH as u32,
        wide(family).as_ptr(),
    )
}

/// 建好窗口（还没显示）。`at` 给了就放在那个屏幕坐标（离屏拍照用），否则放在听写面板正下方。
unsafe fn open(at: Option<(i32, i32)>) -> HWND {
    let module = GetModuleHandleW(ptr::null());
    let class_name = wide(CLASS);
    let class = WNDCLASSW {
        lpfnWndProc: Some(procedure),
        hInstance: module,
        hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
        lpszClassName: class_name.as_ptr(),
        ..mem::zeroed()
    };
    RegisterClassW(&class);
    let style = WS_POPUP | WS_SYSMENU;
    let extended = WS_EX_TOPMOST
        | if at.is_some() {
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE
        } else {
            0
        };
    let (sender, replies) = mpsc::channel();
    let state = Box::into_raw(Box::new(State {
        dpi: 96,
        fonts: [ptr::null_mut(); 3],
        white: CreateSolidBrush(0x00ffffff) as isize,
        asked: 0,
        replies,
        sender,
        last_source: String::new(),
        busy: false,
        phase: 0.,
        note: String::new(),
        copied: false,
        target: Target::English,
    }));
    let window = CreateWindowExW(
        extended,
        class_name.as_ptr(),
        wide("雪耳翻译").as_ptr(),
        style,
        0,
        0,
        100,
        100,
        ptr::null_mut(),
        ptr::null_mut(),
        module,
        state.cast(),
    );
    if window.is_null() {
        drop(Box::from_raw(state));
        return window;
    }
    let dpi = GetDpiForWindow(window).max(96);
    (*state).dpi = dpi;
    let family = crate::app::ui_font(window);
    (*state).fonts = [
        font(18, 400, dpi, family),
        font(12, 400, dpi, family),
        font(15, 600, dpi, family),
    ];
    let (width, height) = (scale(WIDTH, dpi), scale(HEIGHT, dpi));
    let (x, y) = at.unwrap_or_else(|| {
        let mut point = mem::zeroed();
        GetCursorPos(&mut point);
        use windows_sys::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
        };
        let mut info: MONITORINFO = mem::zeroed();
        info.cbSize = mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST), &mut info);
        let work = info.rcWork;
        // 听写面板在顶部居中（顶上 12 + 高 100），小窗接在它下面
        (
            work.left + (work.right - work.left - width) / 2,
            work.top + scale(122, dpi),
        )
    });
    SetWindowPos(
        window,
        ptr::null_mut(),
        x,
        y,
        width,
        height,
        SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
    );
    // 系统圆角 + 阴影（没有标题栏也有）
    {
        use windows_sys::Win32::Graphics::Dwm::*;
        let rounded = 2_u32;
        DwmSetWindowAttribute(
            window,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            (&rounded as *const u32).cast(),
            4,
        );
    }
    let edit = WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_MULTILINE | ES_AUTOVSCROLL;
    for (id, style, top, bottom) in [
        (SOURCE, edit | ES_WANTRETURN, SOURCE_TOP, DIVIDER - 16),
        (RESULT, edit | ES_READONLY, RESULT_TOP, FOOTER - 6),
    ] {
        let control = CreateWindowExW(
            0,
            wide("EDIT").as_ptr(),
            wide("").as_ptr(),
            style,
            scale(PAD, dpi),
            scale(top, dpi),
            scale(WIDTH - PAD * 2, dpi),
            scale(bottom - top, dpi),
            window,
            id as HMENU,
            module,
            ptr::null(),
        );
        let size = if id == RESULT { 1 } else { 0 };
        SendMessageW(
            control,
            WM_SETFONT,
            [font(15, 400, dpi, family), font(16, 400, dpi, family)][size] as usize,
            0,
        );
        // 文字离边框留点空
        let mut rect = RECT {
            left: 2,
            top: 2,
            right: scale(WIDTH - PAD * 2, dpi) - 2,
            bottom: scale(bottom - top, dpi),
        };
        SendMessageW(control, EM_SETRECT, 0, (&mut rect as *mut RECT) as isize);
    }
    let source = GetDlgItem(window, SOURCE as i32);
    SendMessageW(
        source,
        EM_SETLIMITTEXT,
        xueer_ear_backend::translate::MAX_CHARS,
        0,
    );
    SendMessageW(
        source,
        EM_SETCUEBANNER,
        1,
        wide("说点什么，或者粘贴进来").as_ptr() as isize,
    );
    window
}

/// `xueer-ear.exe --translate`：开翻译小窗，关掉就退出进程
pub fn run() -> Result<(), String> {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let mutex = CreateMutexW(ptr::null(), 0, wide("Local\\XueerEar.Translate").as_ptr());
        if !mutex.is_null() && GetLastError() == 183 {
            let existing = FindWindowW(wide(CLASS).as_ptr(), ptr::null());
            if !existing.is_null() {
                ShowWindow(existing, SW_RESTORE);
                SetForegroundWindow(existing);
            }
            return Ok(());
        }
        let window = open(None);
        if window.is_null() {
            return Err("打不开翻译小窗".into());
        }
        ShowWindow(window, SW_SHOW);
        SetForegroundWindow(window);
        SetFocus(GetDlgItem(window, SOURCE as i32));
        let mut message: MSG = mem::zeroed();
        while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {
            if message.message == WM_KEYDOWN {
                if message.wParam == 0x1B {
                    DestroyWindow(window);
                    continue;
                }
                if message.wParam == 0x0D && GetKeyState(VK_CONTROL as i32) < 0 {
                    KillTimer(window, SETTLE_TIMER);
                    let state = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut State;
                    if !state.is_null() {
                        ask(window, &mut *state);
                    }
                    continue;
                }
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        Ok(())
    }
}

/// 离屏建一个翻译小窗拍照用（填好示例文字）；`busy` 为真时画「翻译中」的小点
pub unsafe fn offscreen(busy: bool) -> HWND {
    let window = open(Some((-32000, -32000)));
    if !window.is_null() {
        set_text(window, SOURCE, "今天吃什么呀，我觉得可以去公园散步。");
        if busy {
            let state = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut State;
            (*state).busy = true;
            (*state).phase = 1.2;
            ShowWindow(GetDlgItem(window, RESULT as i32), SW_HIDE);
        } else {
            set_text(
                window,
                RESULT,
                "What should we eat today? I think we could go for a walk in the park.",
            );
        }
        ShowWindow(window, SW_SHOWNOACTIVATE);
    }
    window
}
