//! 翻译小窗（主人 2026-11-19：面板右边的「译」按钮弹出）。上面一格输入，下面一格出译文；
//! 停手约半秒自动翻译，Ctrl+Enter 立刻翻；中文→英文，英文→中文；「复制」放进剪贴板；Esc 关闭。
//! 独立小窗口，用 `xueer-ear.exe --translate` 打开；已经开着就切到前面。
#![allow(unsafe_op_in_unsafe_fn)]
use std::{mem, ptr, sync::mpsc, thread};
use windows_sys::Win32::{
    Foundation::{GetLastError, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{
        CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
        GetStockObject, HDC, HFONT, OUT_DEFAULT_PRECIS, SetBkColor, SetTextColor, WHITE_BRUSH,
    },
    System::{LibraryLoader::GetModuleHandleW, Threading::CreateMutexW},
    UI::{
        HiDpi::{
            AdjustWindowRectExForDpi, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow,
            SetProcessDpiAwarenessContext,
        },
        Input::KeyboardAndMouse::{GetKeyState, SetFocus, VK_CONTROL},
        WindowsAndMessaging::*,
    },
};

const CLASS: &str = "XueerEar.TranslateWindow";
const SOURCE: usize = 401;
const RESULT: usize = 402;
const HINT: usize = 403;
const COPY: usize = 404;
const EN_CHANGE: u32 = 0x0300;
const EM_SETLIMITTEXT: u32 = 0x00C5;
const EM_SETCUEBANNER: u32 = 0x1501;
const ES_MULTILINE: u32 = 0x0004;
const ES_AUTOVSCROLL: u32 = 0x0040;
const ES_READONLY: u32 = 0x0800;
const ES_WANTRETURN: u32 = 0x1000;
/// 停手这么久就自动翻译
const SETTLE_MS: u32 = 600;
const SETTLE_TIMER: usize = 1;
/// 译文回来了（wparam = 这是第几次请求）
const DONE: u32 = WM_APP + 21;
const WIDTH: i32 = 460;
const HEIGHT: i32 = 380;

struct State {
    font: HFONT,
    /// 每次发请求加一；只认最新那次的结果
    asked: usize,
    replies: mpsc::Receiver<(usize, Result<String, String>)>,
    sender: mpsc::Sender<(usize, Result<String, String>)>,
    last_source: String,
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
    // 编辑框换行要 \r\n
    let text = text.replace("\r\n", "\n").replace('\n', "\r\n");
    SetWindowTextW(GetDlgItem(window, id as i32), wide(&text).as_ptr());
}

unsafe fn ask(window: HWND, state: &mut State) {
    let source = text_of(window, SOURCE);
    if source.trim().is_empty() {
        set_text(window, RESULT, "");
        set_text(window, HINT, "");
        state.last_source.clear();
        return;
    }
    if source == state.last_source {
        return;
    }
    state.last_source = source.clone();
    state.asked += 1;
    let (id, sender, target) = (state.asked, state.sender.clone(), window as usize);
    set_text(window, HINT, "翻译中…");
    thread::spawn(move || {
        let result = xueer_ear_backend::translate::translate(&source);
        let _ = sender.send((id, result));
        PostMessageW(target as HWND, DONE, id, 0);
    });
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
        WM_COMMAND => {
            let (id, code) = (wparam & 0xffff, (wparam >> 16) as u32 & 0xffff);
            if id == SOURCE && code == EN_CHANGE {
                SetTimer(hwnd, SETTLE_TIMER, SETTLE_MS, None);
            } else if id == COPY {
                let result = text_of(hwnd, RESULT);
                let hint = if result.is_empty() {
                    "还没有译文".to_string()
                } else {
                    match crate::app::copy_text(hwnd, &result) {
                        Ok(()) => "已复制".into(),
                        Err(reason) => reason,
                    }
                };
                set_text(hwnd, HINT, &hint);
            } else if id == 2 {
                DestroyWindow(hwnd);
            }
            0
        }
        WM_TIMER => {
            KillTimer(hwnd, SETTLE_TIMER);
            ask(hwnd, state);
            0
        }
        DONE => {
            while let Ok((id, result)) = state.replies.try_recv() {
                if id != state.asked {
                    continue;
                }
                match result {
                    Ok(text) => {
                        set_text(hwnd, RESULT, &text);
                        set_text(hwnd, HINT, "");
                    }
                    Err(reason) => set_text(hwnd, HINT, &format!("翻译失败：{reason}")),
                }
            }
            0
        }
        WM_CTLCOLORSTATIC => {
            let dc = wparam as HDC;
            SetBkColor(dc, 0x00ffffff);
            // 只读的译文框也走这里：译文用正文黑，提示行用灰
            let hint = lparam as HWND == GetDlgItem(hwnd, HINT as i32);
            SetTextColor(dc, if hint { 0x00938e8e } else { 0x00251e18 });
            GetStockObject(WHITE_BRUSH) as isize
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

/// 建好窗口（还没显示）。`at` 给了就放在那个屏幕坐标（离屏拍照用），否则放在鼠标所在屏幕的上方居中。
unsafe fn open(at: Option<(i32, i32)>) -> HWND {
    let module = GetModuleHandleW(ptr::null());
    let class_name = wide(CLASS);
    let class = WNDCLASSW {
        lpfnWndProc: Some(procedure),
        hInstance: module,
        hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
        hbrBackground: GetStockObject(WHITE_BRUSH),
        lpszClassName: class_name.as_ptr(),
        ..mem::zeroed()
    };
    RegisterClassW(&class);
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
    let extended = WS_EX_TOPMOST
        | if at.is_some() {
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE
        } else {
            0
        };
    let (sender, replies) = mpsc::channel();
    let state = Box::into_raw(Box::new(State {
        font: ptr::null_mut(),
        asked: 0,
        replies,
        sender,
        last_source: String::new(),
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
    let mut frame = RECT {
        left: 0,
        top: 0,
        right: scale(WIDTH, dpi),
        bottom: scale(HEIGHT, dpi),
    };
    AdjustWindowRectExForDpi(&mut frame, style, 0, extended, dpi);
    let (width, height) = (frame.right - frame.left, frame.bottom - frame.top);
    let (x, y) = at.unwrap_or_else(|| {
        let mut point = mem::zeroed();
        GetCursorPos(&mut point);
        let work = {
            use windows_sys::Win32::Graphics::Gdi::{
                GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
            };
            let mut info: MONITORINFO = mem::zeroed();
            info.cbSize = mem::size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST), &mut info);
            info.rcWork
        };
        // 听写面板在屏幕顶部居中，小窗放在它下面
        (
            work.left + (work.right - work.left - width) / 2,
            work.top + scale(130, dpi),
        )
    });
    SetWindowPos(
        window,
        ptr::null_mut(),
        x,
        y,
        width,
        height,
        SWP_NOZORDER | SWP_NOACTIVATE,
    );
    let font = CreateFontW(
        -scale(14, dpi),
        0,
        0,
        0,
        400,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        OUT_DEFAULT_PRECIS as u32,
        CLIP_DEFAULT_PRECIS as u32,
        CLEARTYPE_QUALITY as u32,
        DEFAULT_PITCH as u32,
        wide(crate::app::ui_font(window)).as_ptr(),
    );
    (*state).font = font;
    let edit = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | ES_MULTILINE | ES_AUTOVSCROLL;
    // （类名, 文字, 样式, 扩展样式, 控件号, x, y, 宽, 高）
    let controls: [(&str, &str, u32, u32, usize, i32, i32, i32, i32); 4] = [
        (
            "EDIT",
            "",
            edit | ES_WANTRETURN,
            WS_EX_CLIENTEDGE,
            SOURCE,
            16,
            16,
            428,
            150,
        ),
        (
            "EDIT",
            "",
            edit | ES_READONLY,
            WS_EX_CLIENTEDGE,
            RESULT,
            16,
            176,
            428,
            150,
        ),
        (
            "STATIC",
            "",
            WS_CHILD | WS_VISIBLE,
            0,
            HINT,
            16,
            342,
            320,
            22,
        ),
        (
            "BUTTON",
            "复制",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            0,
            COPY,
            364,
            336,
            80,
            30,
        ),
    ];
    for (class, text, style, extended, id, x, y, w, h) in controls {
        let control = CreateWindowExW(
            extended,
            wide(class).as_ptr(),
            wide(text).as_ptr(),
            style,
            scale(x, dpi),
            scale(y, dpi),
            scale(w, dpi),
            scale(h, dpi),
            window,
            id as HMENU,
            module,
            ptr::null(),
        );
        SendMessageW(control, WM_SETFONT, font as usize, 0);
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
        wide("输入或粘贴要翻译的文字").as_ptr() as isize,
    );
    window
}

/// `xueer-ear.exe --translate`：开翻译小窗，关掉就退出进程
pub fn run() -> Result<(), String> {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let mutex = CreateMutexW(ptr::null(), 0, wide("Local\\XueerEar.Translate").as_ptr());
        if !mutex.is_null() && GetLastError() == 183 {
            // 已经开着：切到前面
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
                // Esc 关闭；Ctrl+Enter 立刻翻译
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
            if IsDialogMessageW(window, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        Ok(())
    }
}

/// 离屏建一个翻译小窗拍照用（填好示例文字）；返回窗口句柄
pub unsafe fn offscreen() -> HWND {
    let window = open(Some((-32000, -32000)));
    if !window.is_null() {
        set_text(window, SOURCE, "今天吃什么呀，我觉得可以去公园散步。");
        set_text(
            window,
            RESULT,
            "What should we eat today? I think we could go for a walk in the park.",
        );
        ShowWindow(window, SW_SHOWNOACTIVATE);
    }
    window
}
