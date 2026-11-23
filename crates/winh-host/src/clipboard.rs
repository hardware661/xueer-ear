//! 剪贴板：没找到输入框时，这一轮说的话放这里（主人自己 Ctrl+V）；
//! 读不到的输入框（微信）全选复制来读时，先把主人原来的剪贴板存下来，读完原样放回去。
use std::ptr;
use windows_sys::Win32::{
    Foundation::{GlobalFree, HANDLE, HWND},
    System::{
        DataExchange::{
            CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
            GetClipboardSequenceNumber, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
        },
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
    },
};

const CF_UNICODETEXT: u32 = 13;
/// 存剪贴板最多存这么多（一张大截图十几 MB）；再大就不读输入框了，免得把主人的剪贴板弄丢
const MAX_SAVED_BYTES: usize = 64 << 20;

/// `owner` 是剪贴板的拥有者窗口（传空窗口时 SetClipboardData 会失败）。换行按 Windows 的 \r\n 放
pub fn copy(owner: usize, text: &str) -> Result<(), String> {
    let units: Vec<u16> = text
        .replace("\r\n", "\n")
        .replace('\n', "\r\n")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let bytes: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
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
    Ok(())
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
