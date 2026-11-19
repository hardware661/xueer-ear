//! Native popup interaction with the same typography and surfaces as the panel.
#![allow(unsafe_op_in_unsafe_fn)]
use std::{mem, ptr};
use windows_sys::Win32::{
    Foundation::HWND,
    Graphics::Gdi::*,
    UI::{Controls::*, WindowsAndMessaging::*},
};

/// 菜单底色：纯白（和面板的白卡片一致）
const BACKGROUND: u32 = 0x00ffffff;

struct Item {
    text: Vec<u16>,
    font: HFONT,
    dpi: u32,
    checked: bool,
    /// 命令号 0 且没有文字的项画成一条细线
    separator: bool,
    /// 只显示文字、不能点（托盘菜单顶上的状态行）
    label: bool,
}
fn scale(value: i32, dpi: u32) -> i32 {
    (value * dpi as i32 + 48) / 96
}

/// 在按钮下面弹出（左上角对齐）
pub unsafe fn popup(
    hwnd: HWND,
    x: i32,
    y: i32,
    choices: &[(usize, String, bool)],
    font: HFONT,
    dpi: u32,
) -> usize {
    show(hwnd, x, y, choices, font, dpi, TPM_LEFTALIGN | TPM_TOPALIGN)
}

/// 在按钮下面弹出（右上角对齐）
pub unsafe fn popup_right(
    hwnd: HWND,
    x: i32,
    y: i32,
    choices: &[(usize, String, bool)],
    font: HFONT,
    dpi: u32,
) -> usize {
    show(hwnd, x, y, choices, font, dpi, TPM_RIGHTALIGN | TPM_TOPALIGN)
}

/// 在鼠标处弹出（托盘用：任务栏在下面时往上长、在右边时往左长，由系统按屏幕边缘自动翻）
pub unsafe fn popup_at(
    hwnd: HWND,
    x: i32,
    y: i32,
    choices: &[(usize, String, bool)],
    font: HFONT,
    dpi: u32,
) -> usize {
    show(
        hwnd,
        x,
        y,
        choices,
        font,
        dpi,
        TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_RIGHTALIGN,
    )
}

unsafe fn show(
    hwnd: HWND,
    x: i32,
    y: i32,
    choices: &[(usize, String, bool)],
    font: HFONT,
    dpi: u32,
    align: TRACK_POPUP_MENU_FLAGS,
) -> usize {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return 0;
    }
    let items = choices
        .iter()
        .map(|(id, text, checked)| {
            Box::new(Item {
                text: text.encode_utf16().chain(Some(0)).collect(),
                font,
                dpi,
                checked: *checked,
                separator: text.is_empty(),
                label: *id == crate::prefs::NOTHING && !text.is_empty(),
            })
        })
        .collect::<Vec<_>>();
    for ((id, _, checked), item) in choices.iter().zip(&items) {
        let mut info: MENUITEMINFOW = mem::zeroed();
        info.cbSize = mem::size_of::<MENUITEMINFOW>() as u32;
        info.fMask = MIIM_FTYPE | MIIM_ID | MIIM_DATA | MIIM_STRING | MIIM_STATE;
        info.fType = if item.separator {
            MFT_OWNERDRAW | MFT_SEPARATOR
        } else {
            MFT_OWNERDRAW
        };
        info.wID = *id as u32;
        info.fState = if item.label {
            MFS_DISABLED
        } else if *checked {
            MFS_CHECKED
        } else {
            MFS_UNCHECKED
        };
        info.dwItemData = (&**item as *const Item) as usize;
        info.dwTypeData = item.text.as_ptr() as *mut _;
        info.cch = (item.text.len() - 1) as u32;
        InsertMenuItemW(menu, u32::MAX, 1, &info);
    }
    // 行之间、上下留白的地方也是白底
    let back = CreateSolidBrush(BACKGROUND);
    let mut info: MENUINFO = mem::zeroed();
    info.cbSize = mem::size_of::<MENUINFO>() as u32;
    info.fMask = MIM_BACKGROUND;
    info.hbrBack = back;
    SetMenuInfo(menu, &info);
    let chosen = TrackPopupMenu(menu, TPM_RETURNCMD | align, x, y, 0, hwnd, ptr::null());
    DestroyMenu(menu);
    DeleteObject(back);
    PostMessageW(hwnd, WM_NULL, 0, 0);
    chosen as usize
}

pub unsafe fn measure(hwnd: HWND, value: *mut MEASUREITEMSTRUCT) -> bool {
    let value = &mut *value;
    if value.CtlType != ODT_MENU || value.itemData == 0 {
        return false;
    }
    let item = &*(value.itemData as *const Item);
    if item.separator {
        value.itemHeight = scale(11, item.dpi) as u32;
        value.itemWidth = scale(170, item.dpi) as u32;
        return true;
    }
    let dc = GetDC(hwnd);
    let previous = SelectObject(dc, item.font);
    let mut size = mem::zeroed();
    GetTextExtentPoint32W(
        dc,
        item.text.as_ptr(),
        (item.text.len() - 1) as i32,
        &mut size,
    );
    SelectObject(dc, previous);
    ReleaseDC(hwnd, dc);
    value.itemHeight = scale(30, item.dpi) as u32;
    value.itemWidth =
        (size.cx + scale(56, item.dpi)).clamp(scale(180, item.dpi), scale(410, item.dpi)) as u32;
    true
}

pub unsafe fn draw(value: &DRAWITEMSTRUCT) -> bool {
    if value.CtlType != ODT_MENU || value.itemData == 0 {
        return false;
    }
    let item = &*(value.itemData as *const Item);
    let selected = value.itemState & ODS_SELECTED != 0 && !item.label;
    let brush = CreateSolidBrush(BACKGROUND);
    FillRect(value.hDC, &value.rcItem, brush);
    DeleteObject(brush);
    if item.separator {
        let line = CreateSolidBrush(0x00eae5e5);
        let middle = (value.rcItem.top + value.rcItem.bottom) / 2;
        let rect = windows_sys::Win32::Foundation::RECT {
            left: value.rcItem.left + scale(14, item.dpi),
            top: middle,
            right: value.rcItem.right - scale(14, item.dpi),
            bottom: middle + 1,
        };
        FillRect(value.hDC, &rect, line);
        DeleteObject(line);
        return true;
    }
    crate::view::menu_row(value.hDC, item.dpi, &value.rcItem, selected, item.checked);
    SetBkMode(value.hDC, TRANSPARENT as i32);
    // 只读的说明行灰字，能点的近黑字
    SetTextColor(value.hDC, if item.label { 0x00938e8e } else { 0x001e1c1c });
    let previous = SelectObject(value.hDC, item.font);
    let mut rect = value.rcItem;
    rect.left += scale(if item.label { 15 } else { 32 }, item.dpi);
    rect.right -= scale(15, item.dpi);
    DrawTextW(
        value.hDC,
        item.text.as_ptr(),
        (item.text.len() - 1) as i32,
        &mut rect,
        DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
    );
    SelectObject(value.hDC, previous);
    true
}

/// 离屏把一份菜单画成位图（核对外观用；真菜单只在弹出时存在，没法截）。返回位图和宽高。
pub unsafe fn render(
    choices: &[(usize, String, bool)],
    font: HFONT,
    dpi: u32,
    hover: usize,
) -> (HBITMAP, i32, i32) {
    let screen = GetDC(std::ptr::null_mut());
    let width = scale(240, dpi);
    let rows: Vec<(i32, bool)> = choices
        .iter()
        .map(|(_, text, _)| {
            let separator = text.is_empty();
            (scale(if separator { 11 } else { 30 }, dpi), separator)
        })
        .collect();
    let pad = scale(5, dpi);
    let height = rows.iter().map(|(h, _)| h).sum::<i32>() + pad * 2;
    let dc = CreateCompatibleDC(screen);
    let bitmap = CreateCompatibleBitmap(screen, width, height);
    let old = SelectObject(dc, bitmap);
    let background = CreateSolidBrush(BACKGROUND);
    let whole = windows_sys::Win32::Foundation::RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    FillRect(dc, &whole, background);
    DeleteObject(background);
    let mut top = pad;
    for (index, ((id, text, checked), (h, separator))) in choices.iter().zip(&rows).enumerate() {
        let item = Item {
            text: text.encode_utf16().chain(Some(0)).collect(),
            font,
            dpi,
            checked: *checked,
            separator: *separator,
            label: *id == crate::prefs::NOTHING && !text.is_empty(),
        };
        let mut draw: DRAWITEMSTRUCT = mem::zeroed();
        draw.CtlType = ODT_MENU;
        draw.hDC = dc;
        draw.rcItem = windows_sys::Win32::Foundation::RECT {
            left: 0,
            top,
            right: width,
            bottom: top + h,
        };
        draw.itemState = if index == hover { ODS_SELECTED } else { 0 };
        draw.itemData = (&item as *const Item) as usize;
        self::draw(&draw);
        top += h;
    }
    SelectObject(dc, old);
    DeleteDC(dc);
    ReleaseDC(std::ptr::null_mut(), screen);
    (bitmap, width, height)
}
