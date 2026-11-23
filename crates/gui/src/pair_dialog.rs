//! 两格的小对话框：「添加热词」（正确的写法、常被听成什么）和「添加快捷短语」（口令、换成的文字）共用。
//! 回车添加、Esc 取消；填得不对就在下面用红字说哪里不对，不关窗口。
#![allow(unsafe_op_in_unsafe_fn)]
use crate::view::{colorref, dark, palette};
use std::{
    mem, ptr,
    sync::atomic::{AtomicIsize, Ordering},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{
        CreateSolidBrush, FillRect, HBRUSH, HDC, HFONT, InvalidateRect, SetBkColor, SetTextColor,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::AdjustWindowRectExForDpi,
        Input::KeyboardAndMouse::{EnableWindow, SetFocus},
        WindowsAndMessaging::*,
    },
};

const CLASS: &str = "XueerEar.PairDialog";
const ADD: usize = 1; // IDOK：回车
const CANCEL: usize = 2; // IDCANCEL：Esc
const FIRST: usize = 301;
const SECOND: usize = 302;
const HINT: usize = 303;
const EM_SETLIMITTEXT: u32 = 0x00C5;
const EM_SETCUEBANNER: u32 = 0x1501;
const ES_AUTOHSCROLL: u32 = 0x0080;
const BS_DEFPUSHBUTTON: u32 = 0x0001;
/// 客户区大小（96 DPI）
const WIDTH: i32 = 340;
const HEIGHT: i32 = 206;

/// 对话框里写什么、怎么检查
pub struct Form {
    pub title: &'static str,
    pub labels: [&'static str; 2],
    pub cues: [&'static str; 2],
    /// 每格最多几个字
    pub limits: [usize; 2],
    /// 两格填得对不对；不对返回原因，显示在下面的红字里
    pub check: fn(&str, &str) -> Result<(), String>,
}

pub const HOTWORD: Form = Form {
    title: "添加热词",
    labels: ["正确的写法", "常被听成"],
    cues: ["例如 Claude Code", "例如 Cloud Code，可不填"],
    limits: [40, 40],
    check: check_hotword,
};

pub const SNIPPET: Form = Form {
    title: "添加快捷短语",
    labels: ["口令（整句只说这几个字）", "换成的文字（打 \\n 表示换行）"],
    cues: ["例如 我的邮箱", "例如 someone@example.com"],
    limits: [20, 500],
    check: check_snippet,
};

fn check_hotword(word: &str, heard: &str) -> Result<(), String> {
    crate::prefs::hotword_line(word, heard)
        .map(drop)
        .map_err(str::to_owned)
}

fn check_snippet(trigger: &str, text: &str) -> Result<(), String> {
    crate::prefs::snippet_entry(trigger, text).map(drop)
}

struct State {
    done: bool,
    form: &'static Form,
    fields: Option<(String, String)>,
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

/// 对话框底色和输入框底色的画刷：浅色、深色各建一次，进程退出时回收。
/// 浅色是白底白框；深色是灰底，输入框再深一档
unsafe fn brush(field: bool) -> HBRUSH {
    static BRUSHES: [AtomicIsize; 4] = [const { AtomicIsize::new(0) }; 4];
    let slot = &BRUSHES[usize::from(dark()) * 2 + usize::from(field)];
    let mut brush = slot.load(Ordering::Relaxed);
    if brush == 0 {
        let p = palette();
        let color = if field && dark() { p.page } else { p.card };
        brush = CreateSolidBrush(colorref(color)) as isize;
        slot.store(brush, Ordering::Relaxed);
    }
    brush as HBRUSH
}

fn scale(value: i32, dpi: u32) -> i32 {
    (value * dpi as i32 + 48) / 96
}

unsafe fn text_of(dialog: HWND, id: usize, limit: usize) -> String {
    let control = GetDlgItem(dialog, id as i32);
    let mut buffer = vec![0_u16; limit * 2 + 2];
    let len = GetWindowTextW(control, buffer.as_mut_ptr(), buffer.len() as i32).max(0);
    String::from_utf16_lossy(&buffer[..len as usize])
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
            match wparam & 0xffff {
                ADD => {
                    let [first, second] = state.form.limits;
                    let fields = (text_of(hwnd, FIRST, first), text_of(hwnd, SECOND, second));
                    match (state.form.check)(&fields.0, &fields.1) {
                        Ok(()) => {
                            state.fields = Some(fields);
                            state.done = true;
                        }
                        Err(reason) => {
                            let hint = GetDlgItem(hwnd, HINT as i32);
                            SetWindowTextW(hint, wide(&reason).as_ptr());
                            InvalidateRect(hint, ptr::null(), 1);
                            SetFocus(GetDlgItem(hwnd, FIRST as i32));
                        }
                    }
                }
                CANCEL => state.done = true,
                _ => {}
            }
            0
        }
        WM_CLOSE => {
            state.done = true;
            0
        }
        WM_ERASEBKGND => {
            let mut rect: RECT = mem::zeroed();
            GetClientRect(hwnd, &mut rect);
            FillRect(wparam as HDC, &rect, brush(false));
            1
        }
        WM_CTLCOLORSTATIC => {
            let dc = wparam as HDC;
            let p = palette();
            SetBkColor(dc, colorref(p.card));
            let hint = lparam as HWND == GetDlgItem(hwnd, HINT as i32);
            SetTextColor(dc, colorref(if hint { p.danger } else { p.ink }));
            brush(false) as isize
        }
        WM_CTLCOLOREDIT => {
            let dc = wparam as HDC;
            let p = palette();
            SetBkColor(dc, colorref(if dark() { p.page } else { p.card }));
            SetTextColor(dc, colorref(p.ink));
            brush(true) as isize
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

/// 建好对话框并显示。`at` 给了就放在那个屏幕坐标（离屏核对外观用），否则居中在父窗口上。
unsafe fn open(
    parent: HWND,
    font: HFONT,
    dpi: u32,
    state: *mut State,
    at: Option<(i32, i32)>,
) -> HWND {
    let module = GetModuleHandleW(ptr::null());
    let class_name = wide(CLASS);
    let class = WNDCLASSW {
        lpfnWndProc: Some(procedure),
        hInstance: module,
        hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
        lpszClassName: class_name.as_ptr(),
        ..mem::zeroed()
    };
    // 第二次打开时类已经注册过，失败不要紧
    RegisterClassW(&class);
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
    let extended = WS_EX_DLGMODALFRAME
        | if at.is_some() {
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE
        } else {
            0
        };
    let mut frame = RECT {
        left: 0,
        top: 0,
        right: scale(WIDTH, dpi),
        bottom: scale(HEIGHT, dpi),
    };
    AdjustWindowRectExForDpi(&mut frame, style, 0, extended, dpi);
    let (width, height) = (frame.right - frame.left, frame.bottom - frame.top);
    let (x, y) = at.unwrap_or_else(|| {
        let mut owner: RECT = mem::zeroed();
        GetWindowRect(parent, &mut owner);
        (
            owner.left + (owner.right - owner.left - width) / 2,
            owner.top + (owner.bottom - owner.top - height) / 2,
        )
    });
    let form = (*state).form;
    let dialog = CreateWindowExW(
        extended,
        class_name.as_ptr(),
        wide(form.title).as_ptr(),
        style,
        x,
        y,
        width,
        height,
        parent,
        ptr::null_mut(),
        module,
        state.cast(),
    );
    if dialog.is_null() {
        return dialog;
    }
    // 深色模式：标题栏变深
    {
        use windows_sys::Win32::Graphics::Dwm::*;
        let on = u32::from(dark());
        DwmSetWindowAttribute(
            dialog,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            (&on as *const u32).cast(),
            4,
        );
    }
    let tab = WS_CHILD | WS_VISIBLE | WS_TABSTOP;
    // （类名, 文字, 样式, 扩展样式, 控件号, x, y, 宽, 高）
    let controls: [(&str, &str, u32, u32, usize, i32, i32, i32, i32); 7] = [
        (
            "STATIC",
            form.labels[0],
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            22,
            18,
            296,
            20,
        ),
        (
            "EDIT",
            "",
            tab | ES_AUTOHSCROLL,
            WS_EX_CLIENTEDGE,
            FIRST,
            22,
            42,
            296,
            28,
        ),
        (
            "STATIC",
            form.labels[1],
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            22,
            82,
            296,
            20,
        ),
        (
            "EDIT",
            "",
            tab | ES_AUTOHSCROLL,
            WS_EX_CLIENTEDGE,
            SECOND,
            22,
            106,
            296,
            28,
        ),
        (
            "STATIC",
            "",
            WS_CHILD | WS_VISIBLE,
            0,
            HINT,
            22,
            140,
            296,
            18,
        ),
        ("BUTTON", "取消", tab, 0, CANCEL, 148, 164, 80, 30),
        (
            "BUTTON",
            "添加",
            tab | BS_DEFPUSHBUTTON,
            0,
            ADD,
            238,
            164,
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
            dialog,
            id as HMENU,
            module,
            ptr::null(),
        );
        SendMessageW(control, WM_SETFONT, font as usize, 0);
        // 深色模式：按钮、输入框换成系统的深色样式（文件资源管理器、文件对话框用的那套）
        if dark() {
            let theme = match class {
                "BUTTON" => Some("DarkMode_Explorer"),
                "EDIT" => Some("DarkMode_CFD"),
                _ => None,
            };
            if let Some(theme) = theme {
                windows_sys::Win32::UI::Controls::SetWindowTheme(
                    control,
                    wide(theme).as_ptr(),
                    ptr::null(),
                );
            }
        }
    }
    for ((id, cue), limit) in [(FIRST, form.cues[0]), (SECOND, form.cues[1])]
        .into_iter()
        .zip(form.limits)
    {
        let edit = GetDlgItem(dialog, id as i32);
        SendMessageW(edit, EM_SETLIMITTEXT, limit, 0);
        SendMessageW(edit, EM_SETCUEBANNER, 1, wide(cue).as_ptr() as isize);
    }
    ShowWindow(
        dialog,
        if at.is_some() {
            SW_SHOWNOACTIVATE
        } else {
            SW_SHOW
        },
    );
    dialog
}

/// 弹出对话框等主人填。返回检查过的两格内容；取消返回 None。
pub unsafe fn ask(
    parent: HWND,
    font: HFONT,
    dpi: u32,
    form: &'static Form,
) -> Option<(String, String)> {
    let mut state = Box::new(State {
        done: false,
        form,
        fields: None,
    });
    let dialog = open(parent, font, dpi, &mut *state, None);
    if dialog.is_null() {
        return None;
    }
    EnableWindow(parent, 0);
    SetFocus(GetDlgItem(dialog, FIRST as i32));
    let mut message: MSG = mem::zeroed();
    while !state.done {
        let got = GetMessageW(&mut message, ptr::null_mut(), 0, 0);
        if got <= 0 {
            if got == 0 {
                // 程序要退出了：把退出消息还回去，让外层循环也收到
                PostQuitMessage(message.wParam as i32);
            }
            break;
        }
        if IsDialogMessageW(dialog, &message) == 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    // 先把父窗口恢复再销毁对话框，焦点才会回到父窗口
    EnableWindow(parent, 1);
    SetForegroundWindow(parent);
    DestroyWindow(dialog);
    state.fields.take()
}

/// 离屏建一个对话框拍照用（不抢焦点、不在屏幕里）；返回窗口句柄，用完 DestroyWindow。
pub unsafe fn offscreen(parent: HWND, font: HFONT, dpi: u32, form: &'static Form) -> HWND {
    // 只拍照、不交互，状态留给进程退出时回收
    let state = Box::into_raw(Box::new(State {
        done: false,
        form,
        fields: None,
    }));
    open(parent, font, dpi, state, Some((-32000, -32000)))
}
