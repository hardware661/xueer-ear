//! 往剪贴板放一段文字（没找到输入框时，这一轮说的话放这里，主人自己 Ctrl+V）
use std::ptr;
use windows_sys::Win32::{
    Foundation::{GlobalFree, HANDLE, HWND},
    System::{
        DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
    },
};

const CF_UNICODETEXT: u32 = 13;

/// `owner` 是剪贴板的拥有者窗口（传空窗口时 SetClipboardData 会失败）。换行按 Windows 的 \r\n 放
pub fn copy(owner: usize, text: &str) -> Result<(), String> {
    let units: Vec<u16> = text
        .replace("\r\n", "\n")
        .replace('\n', "\r\n")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    unsafe {
        // 剪贴板常被剪贴板工具短暂占着：隔一下多试几次再放弃
        let mut opened = false;
        for _ in 0..10 {
            if OpenClipboard(owner as HWND) != 0 {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        if !opened {
            return Err("剪贴板正被别的程序占用".to_owned());
        }
        let memory = GlobalAlloc(GMEM_MOVEABLE, units.len() * 2);
        let mut copied = false;
        if !memory.is_null() {
            let target = GlobalLock(memory) as *mut u16;
            if !target.is_null() {
                ptr::copy_nonoverlapping(units.as_ptr(), target, units.len());
                GlobalUnlock(memory);
                EmptyClipboard();
                // 成功后内存归剪贴板所有
                copied = !SetClipboardData(CF_UNICODETEXT, memory as HANDLE).is_null();
            }
            if !copied {
                GlobalFree(memory);
            }
        }
        CloseClipboard();
        if copied {
            Ok(())
        } else {
            Err("复制失败".to_owned())
        }
    }
}
