//! 弹出菜单：自己画的分层窗口（macOS 菜单式：白底圆角、柔和阴影、石墨色高亮、淡入），不用系统菜单。
//! 用法和 TrackPopupMenu 一样：弹出后在这里转消息循环，选中返回命令号，点外面、Esc 返回 0。
#![allow(unsafe_op_in_unsafe_fn)]
use crate::view::argb;
use std::{mem, ptr, time::Instant};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
    Graphics::{Gdi::*, GdiPlus as gp},
    System::LibraryLoader::GetModuleHandleW,
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

const CLASS: &str = "XueerEar.Menu";
/// 下面这些都是 96 DPI 单位：阴影留边、卡片上下留白、行高、分隔线高、宽度上下限
const SHADOW: f32 = 16.;
const PAD: f32 = 6.;
const ROW: f32 = 30.;
const LINE: f32 = 11.;
const MIN_WIDTH: f32 = 200.;
const MAX_WIDTH: f32 = 420.;
/// 文字左边：有对勾位置的行 / 只读说明行
const TEXT_LEFT: f32 = 30.;
const LABEL_LEFT: f32 = 16.;
const TEXT_RIGHT: f32 = 18.;
/// 淡入多久（毫秒）
const FADE_MS: f32 = 140.;
/// GDI+ 预乘透明 32 位
const PARGB: i32 = 0x000e_200b;

struct Row {
    id: usize,
    text: Vec<u16>,
    checked: bool,
    /// 没有文字的项画成一条细线
    separator: bool,
    /// 命令号是 NOTHING 的只读说明（托盘顶上的状态、「服务：谷歌 → 智谱」）
    label: bool,
}

impl Row {
    fn height(&self) -> f32 {
        if self.separator { LINE } else { ROW }
    }
    fn clickable(&self) -> bool {
        !self.separator && !self.label
    }
}

#[derive(Clone, Copy)]
enum Anchor {
    /// 左上角对齐 (x, y)
    Left,
    /// 右上角对齐
    Right,
    /// 右下角对齐（托盘：任务栏在下面时往上长）
    Cursor,
}

struct Menu {
    rows: Vec<Row>,
    font: HFONT,
    dpi: u32,
    width: f32,
    height: f32,
    hover: Option<usize>,
    chosen: usize,
    done: bool,
    started: Instant,
    alpha: u8,
    /// 淡入时卡片离最终位置还差多少（96 DPI，正数是往下偏）
    lift: f32,
    /// 窗口左上角（像素）
    origin: POINT,
}

fn scale(value: f32, dpi: u32) -> f32 {
    value * dpi as f32 / 96.
}

impl Menu {
    unsafe fn new(choices: &[(usize, String, bool)], font: HFONT, dpi: u32) -> Self {
        let rows: Vec<Row> = choices
            .iter()
            .map(|(id, text, checked)| Row {
                id: *id,
                text: text.encode_utf16().collect(),
                checked: *checked,
                separator: text.is_empty(),
                label: *id == crate::prefs::NOTHING && !text.is_empty(),
            })
            .collect();
        // 量最长的一行定宽度
        let dc = CreateCompatibleDC(ptr::null_mut());
        let old = SelectObject(dc, font);
        let mut widest = 0.;
        for row in rows.iter().filter(|r| !r.separator) {
            let mut size: SIZE = mem::zeroed();
            GetTextExtentPoint32W(dc, row.text.as_ptr(), row.text.len() as i32, &mut size);
            let left = if row.label { LABEL_LEFT } else { TEXT_LEFT };
            widest = f32::max(
                widest,
                size.cx as f32 * 96. / dpi as f32 * 1.12 + left + TEXT_RIGHT,
            );
        }
        SelectObject(dc, old);
        DeleteDC(dc);
        let height = rows.iter().map(Row::height).sum::<f32>() + PAD * 2.;
        Self {
            rows,
            font,
            dpi,
            width: widest.clamp(MIN_WIDTH, MAX_WIDTH),
            height,
            hover: None,
            chosen: 0,
            done: false,
            started: Instant::now(),
            alpha: 0,
            lift: 0.,
            origin: POINT { x: 0, y: 0 },
        }
    }

    /// 整个窗口（含阴影）多大，像素
    fn size(&self) -> (i32, i32) {
        (
            scale(self.width + SHADOW * 2., self.dpi).ceil() as i32,
            scale(self.height + SHADOW * 2., self.dpi).ceil() as i32,
        )
    }

    /// 窗口里的像素坐标 → 哪一行
    fn row_at(&self, x: i32, y: i32) -> Option<usize> {
        let x = x as f32 * 96. / self.dpi as f32 - SHADOW;
        let mut top = y as f32 * 96. / self.dpi as f32 - SHADOW - PAD;
        if x < 0. || x >= self.width || top < 0. {
            return None;
        }
        for (i, row) in self.rows.iter().enumerate() {
            if top < row.height() {
                return Some(i);
            }
            top -= row.height();
        }
        None
    }

    /// 键盘上下移动高亮，跳过分隔线和说明行
    fn step(&mut self, down: bool) {
        let n = self.rows.len();
        if n == 0 {
            return;
        }
        let mut i = self.hover.unwrap_or(if down { n - 1 } else { 0 });
        for _ in 0..n {
            i = if down { (i + 1) % n } else { (i + n - 1) % n };
            if self.rows[i].clickable() {
                self.hover = Some(i);
                return;
            }
        }
    }
}

unsafe fn rounded_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> *mut gp::GpPath {
    let mut path = ptr::null_mut();
    if gp::GdipCreatePath(gp::FillModeAlternate, &mut path) != 0 {
        return ptr::null_mut();
    }
    let d = r * 2.;
    for (left, top, angle) in [
        (x, y, 180.),
        (x + w - d, y, 270.),
        (x + w - d, y + h - d, 0.),
        (x, y + h - d, 90.),
    ] {
        gp::GdipAddPathArc(path, left, top, d, d, angle, 90.);
    }
    gp::GdipClosePathFigure(path);
    path
}

unsafe fn fill_rounded(g: *mut gp::GpGraphics, x: f32, y: f32, w: f32, h: f32, r: f32, color: u32) {
    let path = rounded_path(x, y, w, h, r);
    if path.is_null() {
        return;
    }
    let mut brush = ptr::null_mut();
    if gp::GdipCreateSolidFill(color, &mut brush) == 0 {
        gp::GdipFillPath(g, brush.cast(), path);
        gp::GdipDeleteBrush(brush.cast());
    }
    gp::GdipDeletePath(path);
}

unsafe fn line(g: *mut gp::GpGraphics, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, color: u32) {
    let mut pen = ptr::null_mut();
    if gp::GdipCreatePen1(color, width, gp::UnitPixel, &mut pen) == 0 {
        gp::GdipSetPenStartCap(pen, gp::LineCapRound);
        gp::GdipSetPenEndCap(pen, gp::LineCapRound);
        gp::GdipDrawLine(g, pen, x1, y1, x2, y2);
        gp::GdipDeletePen(pen);
    }
}

/// 把菜单画进一张 32 位预乘透明位图（`backdrop` 给了就先铺底色，拍照用）
unsafe fn paint(menu: &Menu, backdrop: Option<u32>) -> (HBITMAP, i32, i32) {
    let (w, h) = menu.size();
    let mut info: BITMAPINFO = mem::zeroed();
    info.bmiHeader.biSize = mem::size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = w;
    info.bmiHeader.biHeight = -h;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    let mut bits = ptr::null_mut();
    let bitmap = CreateDIBSection(
        ptr::null_mut(),
        &info,
        DIB_RGB_COLORS,
        &mut bits,
        ptr::null_mut(),
        0,
    );
    if bitmap.is_null() {
        return (bitmap, w, h);
    }
    let mut image = ptr::null_mut();
    if gp::GdipCreateBitmapFromScan0(w, h, w * 4, PARGB, bits as *const u8, &mut image) != 0 {
        return (bitmap, w, h);
    }
    let mut g = ptr::null_mut();
    if gp::GdipGetImageGraphicsContext(image.cast(), &mut g) != 0 {
        gp::GdipDisposeImage(image.cast());
        return (bitmap, w, h);
    }
    if let Some(color) = backdrop {
        gp::GdipGraphicsClear(g, color);
    }
    gp::GdipSetSmoothingMode(g, gp::SmoothingModeAntiAlias8x8);
    gp::GdipSetTextRenderingHint(g, gp::TextRenderingHintAntiAliasGridFit);
    let s = menu.dpi as f32 / 96.;
    gp::GdipScaleWorldTransform(g, s, s, gp::MatrixOrderPrepend);
    let (x, y, cw, ch) = (SHADOW, SHADOW, menu.width, menu.height);
    // 柔和阴影：一圈圈越来越淡，往下偏一点
    for i in 0..10 {
        let spread = 10. - i as f32;
        let alpha = 1 + i as u32 / 3;
        fill_rounded(
            g,
            x - spread,
            y - spread + 3.,
            cw + spread * 2.,
            ch + spread * 2.,
            10. + spread,
            alpha << 24,
        );
    }
    let p = crate::view::palette();
    fill_rounded(g, x - 0.5, y - 0.5, cw + 1., ch + 1., 10.5, p.hairline);
    fill_rounded(g, x, y, cw, ch, 10., argb(p.card));
    // 字用像素坐标画（字体本身已经按 DPI 定好大小）
    let dc = CreateCompatibleDC(ptr::null_mut());
    let old = SelectObject(dc, menu.font);
    let mut font = ptr::null_mut();
    gp::GdipCreateFontFromDC(dc, &mut font);
    SelectObject(dc, old);
    DeleteDC(dc);
    let mut format = ptr::null_mut();
    gp::GdipCreateStringFormat(gp::StringFormatFlagsNoWrap, 0, &mut format);
    gp::GdipSetStringFormatLineAlign(format, gp::StringAlignmentCenter);
    gp::GdipSetStringFormatTrimming(format, gp::StringTrimmingEllipsisCharacter);
    let mut top = y + PAD;
    for (i, row) in menu.rows.iter().enumerate() {
        let rh = row.height();
        if row.separator {
            line(
                g,
                x + 12.,
                top + rh / 2.,
                x + cw - 12.,
                top + rh / 2.,
                1.,
                argb(p.line),
            );
            top += rh;
            continue;
        }
        let hover = menu.hover == Some(i) && row.clickable();
        if hover {
            fill_rounded(g, x + 5., top, cw - 10., rh, 6., argb(p.hover));
        }
        let ink = argb(if hover {
            p.on_hover
        } else if row.label {
            p.muted
        } else {
            p.ink
        });
        if row.checked && !row.label {
            let m = top + rh / 2.;
            line(g, x + 12., m, x + 15.5, m + 3.5, 1.7, ink);
            line(g, x + 15.5, m + 3.5, x + 21.5, m - 4., 1.7, ink);
        }
        if !font.is_null() {
            let left = if row.label { LABEL_LEFT } else { TEXT_LEFT };
            let rect = gp::RectF {
                X: (x + left) * s,
                Y: top * s,
                Width: (cw - left - TEXT_RIGHT + 8.) * s,
                Height: rh * s,
            };
            let mut brush = ptr::null_mut();
            if gp::GdipCreateSolidFill(ink, &mut brush) == 0 {
                gp::GdipResetWorldTransform(g);
                gp::GdipDrawString(
                    g,
                    row.text.as_ptr(),
                    row.text.len() as i32,
                    font,
                    &rect,
                    format,
                    brush.cast(),
                );
                gp::GdipScaleWorldTransform(g, s, s, gp::MatrixOrderPrepend);
                gp::GdipDeleteBrush(brush.cast());
            }
        }
        top += rh;
    }
    if !format.is_null() {
        gp::GdipDeleteStringFormat(format);
    }
    if !font.is_null() {
        gp::GdipDeleteFont(font);
    }
    gp::GdipDeleteGraphics(g);
    gp::GdipDisposeImage(image.cast());
    (bitmap, w, h)
}

/// 把当前样子（透明度、淡入位移）贴到分层窗口上
unsafe fn present(window: HWND, menu: &Menu) {
    let (bitmap, w, h) = paint(menu, None);
    if bitmap.is_null() {
        return;
    }
    let screen = GetDC(ptr::null_mut());
    let memory = CreateCompatibleDC(screen);
    let old = SelectObject(memory, bitmap);
    let position = POINT {
        x: menu.origin.x,
        y: menu.origin.y + scale(menu.lift, menu.dpi).round() as i32,
    };
    let size = SIZE { cx: w, cy: h };
    let source = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: menu.alpha,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    UpdateLayeredWindow(
        window, screen, &position, &size, memory, &source, 0, &blend, ULW_ALPHA,
    );
    SelectObject(memory, old);
    DeleteDC(memory);
    ReleaseDC(ptr::null_mut(), screen);
    DeleteObject(bitmap);
}

unsafe fn finish(menu: &mut Menu, chosen: usize) {
    if !menu.done {
        menu.done = true;
        menu.chosen = chosen;
    }
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
    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Menu;
    if raw.is_null() {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    let menu = &mut *raw;
    let point = || {
        (
            (lparam & 0xffff) as i16 as i32,
            ((lparam >> 16) & 0xffff) as i16 as i32,
        )
    };
    match message {
        WM_MOUSEACTIVATE => MA_NOACTIVATE as isize,
        WM_MOUSEMOVE => {
            let (x, y) = point();
            let hover = menu.row_at(x, y).filter(|&i| menu.rows[i].clickable());
            if hover != menu.hover {
                menu.hover = hover;
                present(hwnd, menu);
            }
            0
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
            let (x, y) = point();
            // 点在菜单外面：收起来，这一下不传给下面的窗口
            if menu.row_at(x, y).is_none() {
                finish(menu, 0);
            }
            0
        }
        WM_LBUTTONUP | WM_RBUTTONUP => {
            let (x, y) = point();
            if let Some(i) = menu.row_at(x, y).filter(|&i| menu.rows[i].clickable()) {
                finish(menu, menu.rows[i].id);
            }
            0
        }
        WM_KEYDOWN => {
            match wparam as u16 {
                VK_ESCAPE | VK_MENU => finish(menu, 0),
                VK_UP => {
                    menu.step(false);
                    present(hwnd, menu);
                }
                VK_DOWN => {
                    menu.step(true);
                    present(hwnd, menu);
                }
                VK_RETURN | VK_SPACE => {
                    if let Some(i) = menu.hover {
                        finish(menu, menu.rows[i].id);
                    }
                }
                _ => {}
            }
            0
        }
        // 被别的窗口抢走前台或鼠标：收起来
        WM_ACTIVATE if wparam & 0xffff == WA_INACTIVE as usize => {
            finish(menu, 0);
            0
        }
        WM_CAPTURECHANGED if lparam as HWND != hwnd => {
            finish(menu, 0);
            0
        }
        WM_TIMER => {
            let t = (menu.started.elapsed().as_secs_f32() * 1000. / FADE_MS).min(1.);
            let eased = 1. - (1. - t).powi(3);
            let lift = menu.lift.signum() * 6. * (1. - eased);
            menu.alpha = (255. * eased) as u8;
            menu.lift = if t >= 1. { 0. } else { lift };
            if t >= 1. {
                KillTimer(hwnd, 1);
            }
            present(hwnd, menu);
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

unsafe fn show(
    owner: HWND,
    x: i32,
    y: i32,
    choices: &[(usize, String, bool)],
    font: HFONT,
    dpi: u32,
    anchor: Anchor,
) -> usize {
    let module = GetModuleHandleW(ptr::null());
    let class_name: Vec<u16> = CLASS.encode_utf16().chain(Some(0)).collect();
    let class = WNDCLASSW {
        lpfnWndProc: Some(procedure),
        hInstance: module,
        hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
        lpszClassName: class_name.as_ptr(),
        ..mem::zeroed()
    };
    // 第二次注册会失败，不要紧
    RegisterClassW(&class);
    let mut menu = Box::new(Menu::new(choices, font, dpi));
    let (w, h) = menu.size();
    let (card_w, card_h) = (
        scale(menu.width, dpi) as i32,
        scale(menu.height, dpi) as i32,
    );
    let shadow = scale(SHADOW, dpi) as i32;
    // 卡片左上角，再按屏幕工作区收进来；下面放不下就翻到上面
    let mut monitor: MONITORINFO = mem::zeroed();
    monitor.cbSize = mem::size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(
        MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST),
        &mut monitor,
    );
    let work: RECT = monitor.rcWork;
    let (mut left, mut top) = match anchor {
        Anchor::Left => (x, y),
        Anchor::Right => (x - card_w, y),
        Anchor::Cursor => (x - card_w, y - card_h),
    };
    let mut upward = matches!(anchor, Anchor::Cursor);
    if top + card_h > work.bottom {
        top = (y - card_h - scale(40., dpi) as i32).max(work.top);
        upward = true;
    }
    left = left.clamp(work.left, (work.right - card_w).max(work.left));
    top = top.clamp(work.top, (work.bottom - card_h).max(work.top));
    menu.origin = POINT {
        x: left - shadow,
        y: top - shadow,
    };
    menu.lift = if upward { 6. } else { -6. };
    let raw: *mut Menu = &mut *menu;
    let window = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
        class_name.as_ptr(),
        ptr::null(),
        WS_POPUP,
        menu.origin.x,
        menu.origin.y,
        w,
        h,
        owner,
        ptr::null_mut(),
        module,
        raw.cast(),
    );
    if window.is_null() {
        return 0;
    }
    present(window, &menu);
    ShowWindow(window, SW_SHOW);
    SetForegroundWindow(window);
    SetCapture(window);
    SetTimer(window, 1, 15, None);
    let mut message: MSG = mem::zeroed();
    while !(*raw).done {
        let got = GetMessageW(&mut message, ptr::null_mut(), 0, 0);
        if got <= 0 {
            if got == 0 {
                PostQuitMessage(message.wParam as i32);
            }
            break;
        }
        TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    let chosen = (*raw).chosen;
    // 先把 done 定住，销毁窗口时的失焦、失去鼠标消息不会改结果
    (*raw).done = true;
    if GetCapture() == window {
        ReleaseCapture();
    }
    DestroyWindow(window);
    drop(menu);
    PostMessageW(owner, WM_NULL, 0, 0);
    chosen
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
    show(hwnd, x, y, choices, font, dpi, Anchor::Left)
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
    show(hwnd, x, y, choices, font, dpi, Anchor::Right)
}

/// 在鼠标处弹出（托盘用：往左上长，碰到屏幕边自动收进来）
pub unsafe fn popup_at(
    hwnd: HWND,
    x: i32,
    y: i32,
    choices: &[(usize, String, bool)],
    font: HFONT,
    dpi: u32,
) -> usize {
    show(hwnd, x, y, choices, font, dpi, Anchor::Cursor)
}

/// 离屏把一份菜单画成位图（核对外观用），垫页面灰底；`hover` 是高亮第几项。返回位图和宽高。
pub unsafe fn render(
    choices: &[(usize, String, bool)],
    font: HFONT,
    dpi: u32,
    hover: usize,
) -> (HBITMAP, i32, i32) {
    let mut menu = Menu::new(choices, font, dpi);
    menu.hover = Some(hover).filter(|&i| menu.rows.get(i).is_some_and(Row::clickable));
    // 离屏拍照时垫在下面的页面灰（真菜单是透明的）
    paint(&menu, Some(argb(crate::view::palette().page)))
}
