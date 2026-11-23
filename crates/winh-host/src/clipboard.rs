//! 剪贴板：没找到输入框时，这一轮说的话放这里（主人自己 Ctrl+V）；
//! 读不到的输入框（微信）全选复制来读时，先把主人原来的剪贴板存下来，读完原样放回去；
//! 多行的译文整段粘贴进输入框，粘贴完也把原来的放回去。
use std::{
    cell::{Cell, RefCell},
    ptr,
    sync::OnceLock,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{GlobalFree, HANDLE, HWND, LPARAM, LRESULT, WPARAM},
    System::{
        DataExchange::{
            CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
            GetClipboardSequenceNumber, GetOpenClipboardWindow, OpenClipboard,
            RegisterClipboardFormatW, SetClipboardData,
        },
        LibraryLoader::GetModuleHandleW,
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
    },
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
        GetWindowThreadProcessId, HWND_MESSAGE, MSG, PM_REMOVE, PeekMessageW, RegisterClassW,
        WM_RENDERALLFORMATS, WM_RENDERFORMAT, WNDCLASSW,
    },
};

const CF_UNICODETEXT: u32 = 13;
/// 存剪贴板最多存这么多（一张大截图十几 MB）；再大就不读输入框了，免得把主人的剪贴板弄丢
const MAX_SAVED_BYTES: usize = 64 << 20;

/// `owner` 是剪贴板的拥有者窗口（传空窗口时 SetClipboardData 会失败）。换行按 Windows 的 \r\n 放
pub fn copy(owner: usize, text: &str) -> Result<(), String> {
    let bytes = unicode_bytes(text);
    let _open = Open::new(owner)?;
    unsafe {
        EmptyClipboard();
    }
    if put(CF_UNICODETEXT, &bytes) {
        Ok(())
    } else {
        Err("复制失败".to_owned())
    }
}

/// 剪贴板上的文字：UTF-16、换行是 \r\n、末尾一个 0
fn unicode_bytes(text: &str) -> Vec<u8> {
    text.replace("\r\n", "\n")
        .replace('\n', "\r\n")
        .encode_utf16()
        .chain(Some(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// 粘贴没成
#[derive(Debug)]
pub enum PasteError {
    /// 还没按 Ctrl+V（剪贴板打不开、里面的东西太大存不下）：可以改成打字
    NotPressed(String),
    /// 按了 Ctrl+V，前台程序 2 秒内没来取字（主人原来的剪贴板已经放回去了）
    NotTaken,
}

/// 把 `text` 粘贴进前台的输入框（`press` 按 Ctrl+V）：先存下主人的剪贴板，放上这段字，按 Ctrl+V，
/// 等前台程序把字取走，再把原来的放回去。
///
/// 字不马上放上去（延迟提供）：前台程序来取时系统才叫我们给，这样就知道它什么时候取走了，
/// 不用猜要等多久——放回得早了，粘贴进去的就成了主人原来剪贴板里的东西（可能是密码）。
/// 要是别的程序（剪贴板工具）先来取了，之后前台程序再取就看不到了，只能按时间等
pub fn paste(text: &str, press: &dyn Fn()) -> Result<(), PasteError> {
    let window = OfferWindow::new().map_err(PasteError::NotPressed)?;
    let saved = save(window.0).map_err(PasteError::NotPressed)?;
    OFFER.set(Some(unicode_bytes(text)));
    TAKEN_BY.set(None);
    {
        let _open = Open::new(window.0).map_err(PasteError::NotPressed)?;
        unsafe {
            EmptyClipboard();
            SetClipboardData(CF_UNICODETEXT, ptr::null_mut());
        }
        exclude_from_history();
    }
    let target = unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(GetForegroundWindow(), &mut pid);
        pid
    };
    press();
    let pressed = Instant::now();
    let outcome = loop {
        window.pump();
        match TAKEN_BY.get() {
            // 取字的窗口不知道是谁（打开剪贴板时没给窗口）也算前台程序
            Some(pid) if pid == 0 || pid == target => break Ok(()),
            Some(_) => {
                while pressed.elapsed() < Duration::from_millis(800) {
                    window.pump();
                    std::thread::sleep(Duration::from_millis(5));
                }
                break Ok(());
            }
            None if pressed.elapsed() >= Duration::from_secs(2) => break Err(PasteError::NotTaken),
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    };
    OFFER.set(None);
    // 前台程序取完字可能还开着剪贴板：放不回去就隔一会儿再试，最多两三秒
    let mut restored = restore(window.0, &saved);
    for _ in 0..6 {
        if restored.is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
        restored = restore(window.0, &saved);
    }
    if let Err(reason) = restored {
        eprintln!("粘贴后没能放回原来的剪贴板：{reason}");
    }
    outcome
}

thread_local! {
    /// 这次粘贴要给出去的字
    static OFFER: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
    /// 来取字的是哪个进程（取字的窗口不知道是谁时是 0）
    static TAKEN_BY: Cell<Option<u32>> = const { Cell::new(None) };
}

/// 粘贴时剪贴板的拥有者：看不见的消息窗口，前台程序来取字时它把字给出去
struct OfferWindow(usize);

impl OfferWindow {
    fn new() -> Result<Self, String> {
        static CLASS: OnceLock<Vec<u16>> = OnceLock::new();
        let class = CLASS.get_or_init(|| {
            let name: Vec<u16> = "XueerEar.ClipboardOffer".encode_utf16().chain(Some(0)).collect();
            let class = WNDCLASSW {
                lpfnWndProc: Some(offer_procedure),
                hInstance: unsafe { GetModuleHandleW(ptr::null()) },
                lpszClassName: name.as_ptr(),
                ..unsafe { std::mem::zeroed() }
            };
            unsafe { RegisterClassW(&class) };
            name
        });
        let window = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                ptr::null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                ptr::null_mut(),
                GetModuleHandleW(ptr::null()),
                ptr::null(),
            )
        };
        if window.is_null() {
            return Err("开不了粘贴用的窗口".to_owned());
        }
        Ok(Self(window as usize))
    }

    /// 处理发给这个窗口的消息（别的程序来取字时，系统在这里叫我们给）
    fn pump(&self) {
        unsafe {
            let mut message: MSG = std::mem::zeroed();
            while PeekMessageW(&mut message, self.0 as HWND, 0, 0, PM_REMOVE) != 0 {
                DispatchMessageW(&message);
            }
        }
    }
}

impl Drop for OfferWindow {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.0 as HWND);
        }
    }
}

unsafe extern "system" fn offer_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_RENDERFORMAT if wparam as u32 == CF_UNICODETEXT => {
            OFFER.with_borrow(|bytes| {
                if let Some(bytes) = bytes {
                    put(CF_UNICODETEXT, bytes);
                }
            });
            let opener = unsafe { GetOpenClipboardWindow() };
            let mut pid = 0;
            if !opener.is_null() {
                unsafe { GetWindowThreadProcessId(opener, &mut pid) };
            }
            TAKEN_BY.set(Some(pid));
            0
        }
        // 窗口要关了、字还没给出去：不给，马上就把主人原来的剪贴板放回去
        WM_RENDERALLFORMATS => 0,
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

/// 剪贴板变了几次（系统计数，读剪贴板不改它）
pub fn sequence() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

/// 剪贴板里的文字（没有文字是 None）
pub fn text(owner: usize) -> Option<String> {
    let _open = Open::new(owner).ok()?;
    let bytes = read(CF_UNICODETEXT)?;
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .take_while(|&u| u != 0)
        .collect();
    Some(String::from_utf16_lossy(&units))
}

/// 存下来的剪贴板：每种格式的原始数据
pub struct Saved(Vec<(u32, Vec<u8>)>);

/// 把剪贴板里能原样存的格式都存下来（文字、图片、文件列表这些都是一块内存；
/// 位图句柄、调色板这类系统对象不存，系统会从存下的图片数据自己再生成）
pub fn save(owner: usize) -> Result<Saved, String> {
    let _open = Open::new(owner)?;
    let mut formats = Vec::new();
    let mut total = 0;
    let mut format = 0;
    loop {
        format = unsafe { EnumClipboardFormats(format) };
        if format == 0 {
            break;
        }
        if !plain_memory(format) {
            continue;
        }
        if let Some(bytes) = read(format) {
            total += bytes.len();
            if total > MAX_SAVED_BYTES {
                return Err("剪贴板里的东西太大，没读输入框".to_owned());
            }
            formats.push((format, bytes));
        }
    }
    Ok(Saved(formats))
}

/// 把存下的剪贴板放回去。标上「别进剪贴板历史、别同步」：这份内容本来就在历史里，不该再多一条
pub fn restore(owner: usize, saved: &Saved) -> Result<(), String> {
    let _open = Open::new(owner)?;
    unsafe {
        EmptyClipboard();
    }
    for (format, bytes) in &saved.0 {
        put(*format, bytes);
    }
    exclude_from_history();
    Ok(())
}

/// 剪贴板正开着时标上「别进剪贴板历史、别同步、剪贴板工具别管」
fn exclude_from_history() {
    for (name, value) in [
        ("CanIncludeInClipboardHistory", 0_u32),
        ("CanUploadToCloudClipboard", 0),
        ("ExcludeClipboardContentFromMonitorProcessing", 0),
    ] {
        let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let format = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
        if format != 0 {
            put(format, &value.to_le_bytes());
        }
    }
}

/// 这种格式的数据是不是一块普通内存（能原样存、原样放回）
fn plain_memory(format: u32) -> bool {
    // 位图、图元文件、调色板、增强图元文件、和它们对应的显示格式；自绘格式；私有格式（0x200～0x2FF）
    // 和 GDI 对象格式（0x300～0x3FF）的数据不是普通内存
    !matches!(format, 2 | 3 | 9 | 14 | 0x80..=0x8E | 0x200..=0x3FF)
}

/// 剪贴板正开着时读一种格式的数据
fn read(format: u32) -> Option<Vec<u8>> {
    unsafe {
        let memory = GetClipboardData(format);
        if memory.is_null() {
            return None;
        }
        let size = GlobalSize(memory);
        let source = GlobalLock(memory) as *const u8;
        if source.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(source, size).to_vec();
        GlobalUnlock(memory);
        Some(bytes)
    }
}

/// 剪贴板正开着时放一种格式的数据；成功后内存归剪贴板所有
fn put(format: u32, bytes: &[u8]) -> bool {
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1));
        if memory.is_null() {
            return false;
        }
        let target = GlobalLock(memory) as *mut u8;
        if target.is_null() {
            GlobalFree(memory);
            return false;
        }
        ptr::copy_nonoverlapping(bytes.as_ptr(), target, bytes.len());
        GlobalUnlock(memory);
        let placed = !SetClipboardData(format, memory as HANDLE).is_null();
        if !placed {
            GlobalFree(memory);
        }
        placed
    }
}

/// 开着剪贴板，离开时关上。剪贴板常被剪贴板工具短暂占着：隔一下多试几次再放弃
struct Open;

impl Open {
    fn new(owner: usize) -> Result<Self, String> {
        for _ in 0..10 {
            if unsafe { OpenClipboard(owner as HWND) } != 0 {
                return Ok(Self);
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        Err("剪贴板正被别的程序占用".to_owned())
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}
