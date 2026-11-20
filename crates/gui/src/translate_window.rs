//! 翻译小窗（主人 2026-11-19：面板右边的「译」按钮弹出；第一版系统方框太丑，按新设计重画）。
//! 一张白色圆角卡片（系统圆角 + 淡阴影，没有标题栏），放在听写面板正下方：
//! 上半输入（无边框、淡灰提示字），中间细分隔线 + 「中 → 英」小标签（自动认语言），
//! 下半译文（字稍大稍深），翻译时三个跳动小点；右下「复制」文字按钮点完变「已复制 ✓」；
//! 右上小 ×，Esc 也能关；按住卡片空白处可以拖。停手约半秒自动翻译，Ctrl+Enter 立刻翻。
//! 用 `xueer-ear.exe --translate` 打开；已经开着就切到前面。
#![allow(unsafe_op_in_unsafe_fn)]
use crate::view::{colorref, palette};
use std::{mem, ptr, sync::mpsc, thread};
use windows_sys::Win32::{
    Foundation::{GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::{
        BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreateSolidBrush,
        DEFAULT_CHARSET, DEFAULT_PITCH, DT_CENTER, DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE,
        DT_VCENTER, DrawTextW, EndPaint, GetDC, GetTextMetricsW, HDC, HFONT, InvalidateRect,
        OUT_DEFAULT_PRECIS, PAINTSTRUCT, RDW_ALLCHILDREN, RDW_INVALIDATE, RDW_UPDATENOW,
        RedrawWindow, ReleaseDC, ScreenToClient, SelectObject, SetBkColor, SetBkMode, SetTextColor,
        TEXTMETRICW, TRANSPARENT,
    },
    System::{LibraryLoader::GetModuleHandleW, Threading::CreateMutexW},
    UI::{
        HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow,
            SetProcessDpiAwarenessContext,
        },
        Input::KeyboardAndMouse::{
            GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_SHIFT,
        },
        WindowsAndMessaging::*,
    },
};
use xueer_ear_backend::translate::Target;

const CLASS: &str = "XueerEar.TranslateWindow";
const SOURCE: usize = 401;
const RESULT: usize = 402;
const EN_CHANGE: u32 = 0x0300;
const EN_VSCROLL: u32 = 0x0602;
const EM_GETFIRSTVISIBLELINE: u32 = 0x00CE;
const EM_GETLINECOUNT: u32 = 0x00BA;
const EM_LINESCROLL: u32 = 0x00B6;
const EM_SETSEL: u32 = 0x00B1;
/// 框里滚动了，重画细滚动条
const SCROLLED: u32 = WM_APP + 22;
const EM_SETLIMITTEXT: u32 = 0x00C5;
const EM_SETCUEBANNER: u32 = 0x1501;
const ES_MULTILINE: u32 = 0x0004;
const ES_AUTOVSCROLL: u32 = 0x0040;
const ES_READONLY: u32 = 0x0800;
const ES_WANTRETURN: u32 = 0x1000;
/// 边打字边自动翻译：离上一次翻译至少隔 `Pace::gap_ms()`（1→2→4→8 秒，主人 2026-11-19 定）
const SETTLE_TIMER: usize = 1;
/// 翻译中小点的动画
const DOTS_TIMER: usize = 2;
/// 「已复制 ✓」显示多久
const COPIED_TIMER: usize = 3;
/// 译文回来了
const DONE: u32 = WM_APP + 21;
/// 长文翻好了几段（wparam = 第几次请求，lparam = 已翻 << 16 | 一共几段）
const PROGRESS: u32 = WM_APP + 23;

/// 初始卡片尺寸（96 DPI）：上半白底原文，下半浅灰底译文。窗口可以拖边改大小、最大化，
/// 下面的位置都按当前大小算（`Layout`）
const WIDTH: i32 = 420;
const HEIGHT: i32 = 300;
const MIN_WIDTH: i32 = 320;
const MIN_HEIGHT: i32 = 240;
const PAD: i32 = 18;
/// 每半左上角的语言小字（「中文」「English」）
const CAPTION_H: i32 = 30;
/// 分隔线下面多留的距离（主人 2026-11-19：线和上下内容之间要有距离）
const LOWER: i32 = 8;
/// 拖这么宽的边改大小
const EDGE: i32 = 6;
/// 细滚动条：宽、离右边多远
const BAR_W: i32 = 4;
const BAR_GAP: i32 = 6;

/// 当前窗口大小下各块的位置（96 DPI 坐标：x, y, 宽, 高）
#[derive(Clone, Copy)]
struct Layout {
    w: i32,
    h: i32,
    divider: i32,
}

impl Layout {
    fn of(window: HWND, dpi: u32) -> Self {
        let mut client: RECT = unsafe { mem::zeroed() };
        unsafe { GetClientRect(window, &mut client) };
        let w = client.right * 96 / dpi as i32;
        let h = client.bottom * 96 / dpi as i32;
        // 上下各一半，上半稍小一点
        Self {
            w,
            h,
            divider: (h * 48 / 100).max(118),
        }
    }
    fn close(&self) -> (i32, i32, i32, i32) {
        (self.w - 38, 8, 28, 28)
    }
    fn maximize(&self) -> (i32, i32, i32, i32) {
        (self.w - 70, 8, 28, 28)
    }
    fn minimize(&self) -> (i32, i32, i32, i32) {
        (self.w - 102, 8, 28, 28)
    }
    fn copy(&self) -> (i32, i32, i32, i32) {
        (self.w - 40, self.h - 40, 30, 30)
    }
    /// 上半右下角的「翻译」按钮（主人 2026-11-19：加一个按钮，回车是快捷键）
    fn go(&self) -> (i32, i32, i32, i32) {
        (self.w - 18 - 58, self.divider - 12 - 28, 58, 28)
    }
    /// 翻译中小点：在下半「English」小字下面
    fn dots(&self) -> (i32, i32, i32, i32) {
        (PAD, self.divider + LOWER + CAPTION_H + 4, 60, 24)
    }
    fn captions(&self) -> [(i32, i32, i32, i32); 2] {
        [(PAD, 8, 200, 22), (PAD, self.divider + LOWER + 8, 200, 22)]
    }
    /// 分隔线中间的方向胶囊（点它反转「中 ⇄ 英」）
    fn swap(&self) -> (i32, i32, i32, i32) {
        (self.w / 2 - 52, self.divider - 13, 104, 26)
    }
    fn note(&self) -> (i32, i32, i32, i32) {
        (PAD, self.h - 38, self.copy().0 - PAD - 8, 26)
    }
    /// 原文框、译文框（右边留出细滚动条的位置）
    fn source(&self) -> (i32, i32, i32, i32) {
        (
            PAD,
            CAPTION_H + 6,
            self.w - PAD * 2 - BAR_GAP - BAR_W,
            self.divider - 46 - CAPTION_H - 6,
        )
    }
    fn result(&self) -> (i32, i32, i32, i32) {
        let top = self.divider + LOWER + CAPTION_H + 6;
        (
            PAD,
            top,
            self.w - PAD * 2 - BAR_GAP - BAR_W,
            self.h - 44 - top,
        )
    }
    fn buttons(&self) -> [(i32, i32, i32, i32); 6] {
        [
            self.swap(),
            self.close(),
            self.maximize(),
            self.minimize(),
            self.copy(),
            self.go(),
        ]
    }
}

struct State {
    dpi: u32,
    fonts: [HFONT; 3],
    /// 底色画刷（输入框、三个小点那一条），跟着深浅色走
    card: isize,
    asked: usize,
    in_flight: bool,
    queued: bool,
    replies: mpsc::Receiver<(usize, Result<String, String>)>,
    sender: mpsc::Sender<(usize, Result<String, String>)>,
    last_source: String,
    busy: bool,
    phase: f32,
    /// 底栏左边的一句提示（失败原因）
    note: String,
    copied: bool,
    target: Target,
    /// 主人点了 ⇄：这次按手动方向翻，不再按内容自动判断；原文一改就回到自动
    manual: bool,
    /// 离屏拍照用的窗口
    offscreen: bool,
    /// 正在拖哪个框的滚动条
    dragging: Option<usize>,
    /// 左键在哪按下的（按钮要按下、松开都在它上面才算点）
    pressed: Option<POINT>,
    /// 自动翻译的节奏、上一次自动翻译的时间、是否已经定了时
    pace: xueer_ear_backend::translate::Pace,
    last_auto: Option<std::time::Instant>,
    waiting: bool,
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

/// 翻译小窗关掉、出错时记一行到 %LOCALAPPDATA%\\XueerEar\\translate.log（主人 2026-11-19：窗口自己没了，查不到原因）
fn note_close(what: &str) {
    use std::io::Write;
    let Some(dir) = std::env::var_os("LOCALAPPDATA") else {
        return;
    };
    let path = std::path::PathBuf::from(dir)
        .join("XueerEar")
        .join("translate.log");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let _ = writeln!(file, "{at} {what}");
    }
}

/// 按当前窗口大小摆两个框
unsafe fn place_edits(window: HWND, state: &State) {
    let layout = Layout::of(window, state.dpi);
    for (id, (x, y, w, h)) in [(SOURCE, layout.source()), (RESULT, layout.result())] {
        let d = state.dpi;
        SetWindowPos(
            GetDlgItem(window, id as i32),
            ptr::null_mut(),
            scale(x, d),
            scale(y, d),
            scale(w, d).max(1),
            scale(h, d).max(1),
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW | SWP_NOCOPYBITS,
        );
    }
}

/// 框里一共几行、第一行是第几行、一屏能放几行
unsafe fn lines(window: HWND, id: usize, height_px: i32) -> (i32, i32, i32) {
    let edit = GetDlgItem(window, id as i32);
    let total = SendMessageW(edit, EM_GETLINECOUNT, 0, 0) as i32;
    let first = SendMessageW(edit, EM_GETFIRSTVISIBLELINE, 0, 0) as i32;
    let dc = GetDC(edit);
    let old = SelectObject(dc, SendMessageW(edit, WM_GETFONT, 0, 0) as _);
    let mut metrics: TEXTMETRICW = mem::zeroed();
    GetTextMetricsW(dc, &mut metrics);
    SelectObject(dc, old);
    ReleaseDC(edit, dc);
    let per_line = (metrics.tmHeight + metrics.tmExternalLeading).max(1);
    (total, first, (height_px / per_line).max(1))
}

/// 细滚动条滑块的位置（96 DPI），内容没超出框高就不画
unsafe fn scroll_thumb(
    window: HWND,
    id: usize,
    layout: Layout,
    dpi: u32,
) -> Option<(f32, f32, f32, f32)> {
    let (x, y, w, h) = if id == SOURCE {
        layout.source()
    } else {
        layout.result()
    };
    let edit = GetDlgItem(window, id as i32);
    if IsWindowVisible(edit) == 0 {
        return None;
    }
    let (total, first, visible) = lines(window, id, scale(h, dpi));
    if total <= visible {
        return None;
    }
    let track = h as f32;
    let size = (track * visible as f32 / total as f32).max(24.);
    let top = (track - size) * first as f32 / (total - visible) as f32;
    Some(((x + w + BAR_GAP) as f32, y as f32 + top, BAR_W as f32, size))
}

/// 拖滚动条：按鼠标在轨道里的位置滚到对应的行
unsafe fn drag_to(window: HWND, id: usize, y_px: i32, layout: Layout, dpi: u32) {
    let (_, y, _, h) = if id == SOURCE {
        layout.source()
    } else {
        layout.result()
    };
    let (total, first, visible) = lines(window, id, scale(h, dpi));
    if total <= visible {
        return;
    }
    let ratio = ((y_px - scale(y, dpi)) as f32 / scale(h, dpi) as f32).clamp(0., 1.);
    let want = (ratio * (total - visible) as f32).round() as i32;
    SendMessageW(
        GetDlgItem(window, id as i32),
        EM_LINESCROLL,
        0,
        (want - first) as isize,
    );
    InvalidateRect(window, ptr::null(), 0);
}

/// 马上翻译（按钮、回车）：不等停手；同样的原文再按一次也重新翻
unsafe fn translate_now(window: HWND, state: &mut State) {
    KillTimer(window, SETTLE_TIMER);
    state.waiting = false;
    state.pace.reset();
    state.last_source.clear();
    ask(window, state);
}

/// 打了字：离上一次翻译够久了就马上翻，不够就定个时到点再翻（期间再打字不重新计时）
unsafe fn schedule(window: HWND, state: &mut State) {
    if state.waiting {
        return;
    }
    let gap = state.pace.gap_ms();
    let since = state.last_auto.map_or(u32::MAX, |at| {
        at.elapsed().as_millis().min(u128::from(u32::MAX)) as u32
    });
    state.waiting = true;
    SetTimer(window, SETTLE_TIMER, gap.saturating_sub(since).max(1), None);
}

/// 反转翻译方向（像谷歌翻译的 ⇄）：两半的语言对调；已经有译文的话，译文搬到上面当原文，再翻回去
unsafe fn swap_direction(window: HWND, state: &mut State) {
    state.target = match state.target {
        Target::English => Target::Chinese,
        Target::Chinese => Target::English,
    };
    let result = text_of(window, RESULT);
    if !result.trim().is_empty() && !state.busy {
        set_text(window, SOURCE, &result);
        set_text(window, RESULT, "");
    }
    // 放在搬字之后：搬字会触发原文 EN_CHANGE，把手动标记清掉
    state.manual = true;
    InvalidateRect(window, ptr::null(), 0);
    translate_now(window, state);
}

/// 只让一小块重画（96 DPI 坐标）
unsafe fn invalidate(window: HWND, dpi: u32, (x, y, w, h): (i32, i32, i32, i32)) {
    let rect = RECT {
        left: scale(x, dpi),
        top: scale(y, dpi),
        right: scale(x + w, dpi),
        bottom: scale(y + h, dpi),
    };
    InvalidateRect(window, &rect, 0);
}

unsafe fn ask(window: HWND, state: &mut State) {
    let source = text_of(window, SOURCE);
    if source.trim().is_empty() {
        set_text(window, RESULT, "");
        state.last_source.clear();
        state.note.clear();
        state.busy = false;
        KillTimer(window, DOTS_TIMER);
        invalidate(window, state.dpi, Layout::of(window, state.dpi).dots());
        invalidate(window, state.dpi, Layout::of(window, state.dpi).note());
        return;
    }
    if source == state.last_source {
        return;
    }
    if state.in_flight {
        state.asked += 1;
        state.queued = true;
        state.note = "等上一条翻译结束…".into();
        invalidate(window, state.dpi, Layout::of(window, state.dpi).note());
        return;
    }
    state.in_flight = true;
    state.last_source = source.clone();
    state.asked += 1;
    state.busy = true;
    state.note.clear();
    // 先清空再藏：藏起来的瞬间露出的就是和它同色的灰底 + 小点，看不出变化
    set_text(window, RESULT, "");
    ShowWindow(GetDlgItem(window, RESULT as i32), SW_HIDE);
    SetTimer(window, DOTS_TIMER, 60, None);
    if std::env::var_os("XUEER_EAR_PACE_LOG").is_some() {
        note_close(&format!(
            "自动/手动翻译第 {} 次，{} 字",
            state.asked,
            source.chars().count()
        ));
    }
    let (id, sender, target) = (state.asked, state.sender.clone(), window as usize);
    // 默认按内容自动定方向：有中文就中译英，纯英文就英译中
    if !state.manual {
        let detected = Target::for_text(&source);
        if detected != state.target {
            state.target = detected;
            InvalidateRect(window, ptr::null(), 0);
        }
    }
    let target_language = state.target;
    thread::spawn(move || {
        // 长文分好几段同时翻：每翻好一段报一次，底栏写「已翻 3/15 段」，不让人干等
        let progress = |done: usize, total: usize| {
            if total > 1 {
                let packed = (done.min(0xffff) << 16) | total.min(0xffff);
                PostMessageW(target as HWND, PROGRESS, id, packed as LPARAM);
            }
        };
        let result = xueer_ear_backend::translate::translate_into_with_progress(
            &source,
            target_language,
            &progress,
        );
        let _ = sender.send((id, result));
        PostMessageW(target as HWND, DONE, id, 0);
    });
}

unsafe fn paint(window: HWND, state: &State) {
    use windows_sys::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, SRCCOPY,
    };
    let mut ps: PAINTSTRUCT = mem::zeroed();
    let screen = BeginPaint(window, &mut ps);
    let mut client: RECT = mem::zeroed();
    GetClientRect(window, &mut client);
    // 屏幕上先在内存里画完整张卡片再一次贴上去：直接往屏幕画会先看到白底、再灰底、再字，一闪一闪
    // （主人 2026-11-19）。离屏拍照时（PrintWindow）直接画，内存画布在那条路上是黑的。
    let (dc, buffer) = if state.offscreen {
        (screen, None)
    } else {
        let memory = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, client.right, client.bottom);
        let old = SelectObject(memory, bitmap);
        (memory, Some((bitmap, old)))
    };
    let layout = Layout::of(window, state.dpi);
    crate::view::translate_card(
        dc,
        state.dpi,
        client.right,
        client.bottom,
        layout.divider as f32,
        state.busy.then_some(state.phase),
        state.copied,
        IsZoomed(window) != 0,
    );
    // 两个框各自的细滚动条（内容超出框高才画）
    for id in [SOURCE, RESULT] {
        if let Some((x, y, w, h)) = scroll_thumb(window, id, layout, state.dpi) {
            crate::view::thumb(dc, state.dpi, x, y, w, h);
        }
    }
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
            DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
        SelectObject(dc, old);
    };
    // 语言小字（GDI 颜色 0x00BBGGRR）
    let (from, to) = match state.target {
        Target::English => ("中文", "English"),
        Target::Chinese => ("English", "中文"),
    };
    let p = palette();
    draw(
        from,
        state.fonts[1],
        colorref(p.muted),
        layout.captions()[0],
    );
    draw(to, state.fonts[1], colorref(p.muted), layout.captions()[1]);
    if !state.note.is_empty() {
        draw(
            &state.note,
            state.fonts[1],
            colorref(p.danger),
            layout.note(),
        );
    }
    // 分隔线中间：「中 ⇄ 英」，点了反转
    let (sx, sy, sw, sh) = layout.swap();
    crate::view::swap_pill(dc, d, sx as f32, sy as f32, sw as f32, sh as f32);
    let (left, right) = match state.target {
        Target::English => ("中", "英"),
        Target::Chinese => ("英", "中"),
    };
    {
        let old = SelectObject(dc, state.fonts[1]);
        SetTextColor(dc, colorref(p.ink_soft));
        for (text, x) in [(left, sx + 8), (right, sx + sw - 8 - 32)] {
            let mut rect = RECT {
                left: scale(x, d),
                top: scale(sy, d),
                right: scale(x + 32, d),
                bottom: scale(sy + sh, d),
            };
            let units: Vec<u16> = text.encode_utf16().collect();
            DrawTextW(
                dc,
                units.as_ptr(),
                units.len() as i32,
                &mut rect,
                DT_SINGLELINE | DT_VCENTER | DT_CENTER,
            );
        }
        SelectObject(dc, old);
    }
    // 上半右下角的「翻译」按钮：主色胶囊（浅色是深灰胶囊白字）
    let (gx, gy, gw, gh) = layout.go();
    crate::view::pill(dc, d, gx, gy, gw, gh);
    {
        let old = SelectObject(dc, state.fonts[1]);
        SetTextColor(dc, colorref(p.on_accent));
        let mut rect = RECT {
            left: scale(gx, d),
            top: scale(gy, d),
            right: scale(gx + gw, d),
            bottom: scale(gy + gh, d),
        };
        let units: Vec<u16> = "翻译".encode_utf16().collect();
        DrawTextW(
            dc,
            units.as_ptr(),
            units.len() as i32,
            &mut rect,
            DT_SINGLELINE | DT_VCENTER | DT_CENTER,
        );
        SelectObject(dc, old);
    }
    if state.busy {
        let (x, y, w, h) = layout.dots();
        let (px, py) = (scale(x, d), scale(y, d));
        crate::view::busy_dots_at(dc, d, px, py, scale(w, d), scale(h, d), state.phase);
    }
    if let Some((bitmap, old)) = buffer {
        BitBlt(screen, 0, 0, client.right, client.bottom, dc, 0, 0, SRCCOPY);
        SelectObject(dc, old);
        DeleteObject(bitmap);
        DeleteDC(dc);
    }
    EndPaint(window, &ps);
}

/// 只重画三个小点那一条：先画进内存再一次贴上去，不闪
unsafe fn paint_dots(window: HWND, state: &State) {
    use windows_sys::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        ReleaseDC, SRCCOPY,
    };
    let d = state.dpi;
    let (x, y, w, h) = Layout::of(window, d).dots();
    let (px, py, pw, ph) = (scale(x, d), scale(y, d), scale(w, d), scale(h, d));
    let screen = GetDC(window);
    let memory = CreateCompatibleDC(screen);
    let bitmap = CreateCompatibleBitmap(screen, pw, ph);
    let old = SelectObject(memory, bitmap);
    // 内存位图一开始是黑的，先铺成译文区的浅灰，小点边缘的抗锯齿才不会带黑边
    let all = RECT {
        left: 0,
        top: 0,
        right: pw,
        bottom: ph,
    };
    windows_sys::Win32::Graphics::Gdi::FillRect(memory, &all, state.card as _);
    crate::view::busy_dots(memory, d, pw, ph, state.phase);
    BitBlt(screen, px, py, pw, ph, memory, 0, 0, SRCCOPY);
    SelectObject(memory, old);
    DeleteObject(bitmap);
    DeleteDC(memory);
    ReleaseDC(window, screen);
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
        // 整个窗口都是自己画的，没有边框和标题栏。系统默认处理这两条时会沿窗口边画一圈旧式灰边框，
        // 点住边缘拖大小时最明显，要等下次重画才消失（主人 2026-11-21）。
        // lparam 传 -1：照常切换激活状态，但不画非客户区
        WM_NCACTIVATE => DefWindowProcW(hwnd, message, wparam, -1),
        WM_NCPAINT => 0,
        WM_NCHITTEST => {
            let mut point = POINT {
                x: (lparam as i16) as i32,
                y: ((lparam >> 16) as i16) as i32,
            };
            ScreenToClient(hwnd, &mut point);
            let layout = Layout::of(hwnd, state.dpi);
            // 四边、四角拖动改大小（最大化时不行）
            let e = scale(EDGE, state.dpi);
            let (cw, ch) = (scale(layout.w, state.dpi), scale(layout.h, state.dpi));
            if IsZoomed(hwnd) == 0 {
                let (l, r, t, b) = (
                    point.x < e,
                    point.x >= cw - e,
                    point.y < e,
                    point.y >= ch - e,
                );
                let edge = match (l, r, t, b) {
                    (true, _, true, _) => Some(HTTOPLEFT),
                    (_, true, true, _) => Some(HTTOPRIGHT),
                    (true, _, _, true) => Some(HTBOTTOMLEFT),
                    (_, true, _, true) => Some(HTBOTTOMRIGHT),
                    (true, ..) => Some(HTLEFT),
                    (_, true, ..) => Some(HTRIGHT),
                    (_, _, true, _) => Some(HTTOP),
                    (.., true) => Some(HTBOTTOM),
                    _ => None,
                };
                if let Some(edge) = edge {
                    return edge as isize;
                }
            }
            // 按钮和滚动条是控件区；其余空白处按住可以拖
            let on_bar = [SOURCE, RESULT].iter().any(|&id| {
                let (x, y, w, h) = if id == SOURCE {
                    layout.source()
                } else {
                    layout.result()
                };
                inside(point, (x + w, y, BAR_GAP + BAR_W + 4, h), state.dpi)
            });
            if on_bar
                || layout
                    .buttons()
                    .iter()
                    .any(|&b| inside(point, b, state.dpi))
            {
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
            let layout = Layout::of(hwnd, state.dpi);
            state.dragging = None;
            ReleaseCapture();
            // 按钮要按下和松开都在它上面才算（拖窗口、拖边、拖滚动条后在 × 上松手不会把窗口关掉）
            let pressed = state.pressed.take();
            let same = |b: (i32, i32, i32, i32)| {
                inside(point, b, state.dpi) && pressed.is_some_and(|p| inside(p, b, state.dpi))
            };
            if same(layout.close()) {
                note_close("点了 ×");
                DestroyWindow(hwnd);
            } else if same(layout.maximize()) {
                ShowWindow(
                    hwnd,
                    if IsZoomed(hwnd) != 0 {
                        SW_RESTORE
                    } else {
                        SW_MAXIMIZE
                    },
                );
            } else if same(layout.minimize()) {
                ShowWindow(hwnd, SW_MINIMIZE);
            } else if same(layout.swap()) {
                swap_direction(hwnd, state);
            } else if same(layout.go()) {
                translate_now(hwnd, state);
            } else if same(layout.copy()) {
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
                invalidate(hwnd, state.dpi, Layout::of(hwnd, state.dpi).copy());
                invalidate(hwnd, state.dpi, Layout::of(hwnd, state.dpi).note());
            }
            0
        }
        WM_SETCURSOR => {
            let mut point = mem::zeroed();
            GetCursorPos(&mut point);
            ScreenToClient(hwnd, &mut point);
            if Layout::of(hwnd, state.dpi)
                .buttons()
                .iter()
                .any(|&b| inside(point, b, state.dpi))
            {
                SetCursor(LoadCursorW(ptr::null_mut(), IDC_HAND));
                return 1;
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_COMMAND => {
            let (id, code) = (wparam & 0xffff, (wparam >> 16) as u32 & 0xffff);
            if code == EN_VSCROLL || code == EN_CHANGE {
                PostMessageW(hwnd, SCROLLED, 0, 0);
            }
            if id == SOURCE && code == EN_CHANGE {
                state.manual = false;
                state.asked += 1;
                state.queued = false;
                state.last_source.clear();
                state.note.clear();
                set_text(hwnd, RESULT, "");
                state.busy = false;
                KillTimer(hwnd, DOTS_TIMER);
                let layout = Layout::of(hwnd, state.dpi);
                invalidate(hwnd, state.dpi, layout.dots());
                invalidate(hwnd, state.dpi, layout.note());
                if text_of(hwnd, SOURCE).trim().is_empty() {
                    // 清空了：节奏从头算，译文也清掉
                    KillTimer(hwnd, SETTLE_TIMER);
                    state.waiting = false;
                    state.pace.reset();
                    state.last_auto = None;
                    ask(hwnd, state);
                } else {
                    schedule(hwnd, state);
                }
            }
            0
        }
        WM_TIMER => {
            match wparam {
                SETTLE_TIMER => {
                    KillTimer(hwnd, SETTLE_TIMER);
                    state.waiting = false;
                    let before = state.asked;
                    ask(hwnd, state);
                    if state.asked != before {
                        state.pace.auto();
                        state.last_auto = Some(std::time::Instant::now());
                    }
                }
                DOTS_TIMER => {
                    state.phase += 0.35;
                    paint_dots(hwnd, state);
                }
                _ => {
                    KillTimer(hwnd, COPIED_TIMER);
                    state.copied = false;
                    invalidate(hwnd, state.dpi, Layout::of(hwnd, state.dpi).copy());
                }
            }
            0
        }
        PROGRESS => {
            if wparam == state.asked && state.busy {
                let (done, total) = ((lparam as usize) >> 16, (lparam as usize) & 0xffff);
                state.note = format!("已翻 {done}/{total} 段…");
                invalidate(hwnd, state.dpi, Layout::of(hwnd, state.dpi).note());
            }
            0
        }
        DONE => {
            while let Ok((id, result)) = state.replies.try_recv() {
                state.in_flight = false;
                if id != state.asked {
                    continue;
                }
                state.busy = false;
                KillTimer(hwnd, DOTS_TIMER);
                ShowWindow(GetDlgItem(hwnd, RESULT as i32), SW_SHOWNOACTIVATE);
                state.note.clear();
                match result {
                    Ok(text) => set_text(hwnd, RESULT, &text),
                    Err(reason) => {
                        state.note = format!("翻译失败：{reason}");
                        note_close(&state.note);
                    }
                }
            }
            if std::mem::take(&mut state.queued) {
                ask(hwnd, state);
            }
            invalidate(hwnd, state.dpi, Layout::of(hwnd, state.dpi).dots());
            invalidate(hwnd, state.dpi, Layout::of(hwnd, state.dpi).note());
            0
        }
        WM_SIZE => {
            // 拖边改大小：两个框一次挪好（不各自重画），再让整窗按双缓冲重画一次。
            // 1.1.9 用 WM_SETREDRAW 关/开重画，结果新开的窗口背景没画上、整窗发黑（主人 2026-11-19），不能用
            place_edits(hwnd, state);
            RedrawWindow(
                hwnd,
                ptr::null(),
                ptr::null_mut(),
                RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_UPDATENOW,
            );
            0
        }
        WM_GETMINMAXINFO => {
            let info = &mut *(lparam as *mut MINMAXINFO);
            info.ptMinTrackSize.x = scale(MIN_WIDTH, state.dpi);
            info.ptMinTrackSize.y = scale(MIN_HEIGHT, state.dpi);
            // 最大化时铺满所在屏幕的工作区，不盖住任务栏（没有标题栏的窗口默认会铺满整个屏幕）
            {
                use windows_sys::Win32::Graphics::Gdi::{
                    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
                };
                let mut monitor: MONITORINFO = mem::zeroed();
                monitor.cbSize = mem::size_of::<MONITORINFO>() as u32;
                GetMonitorInfoW(
                    MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
                    &mut monitor,
                );
                let (work, whole) = (monitor.rcWork, monitor.rcMonitor);
                info.ptMaxPosition.x = work.left - whole.left;
                info.ptMaxPosition.y = work.top - whole.top;
                info.ptMaxSize.x = work.right - work.left;
                info.ptMaxSize.y = work.bottom - work.top;
            }
            0
        }
        // 按住细滚动条拖
        WM_LBUTTONDOWN => {
            let point = POINT {
                x: (lparam as i16) as i32,
                y: ((lparam >> 16) as i16) as i32,
            };
            let layout = Layout::of(hwnd, state.dpi);
            state.pressed = Some(point);
            for id in [SOURCE, RESULT] {
                let (x, y, w, h) = if id == SOURCE {
                    layout.source()
                } else {
                    layout.result()
                };
                if inside(point, (x + w, y, BAR_GAP + BAR_W + 4, h), state.dpi) {
                    state.dragging = Some(id);
                    SetCapture(hwnd);
                    drag_to(hwnd, id, point.y, layout, state.dpi);
                }
            }
            0
        }
        WM_MOUSEMOVE => {
            if let Some(id) = state.dragging {
                let y = ((lparam >> 16) as i16) as i32;
                drag_to(hwnd, id, y, Layout::of(hwnd, state.dpi), state.dpi);
            }
            0
        }
        // 框里滚动了（滚轮、键盘、打字换行）：重画滚动条
        SCROLLED => {
            InvalidateRect(hwnd, ptr::null(), 0);
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint(hwnd, state);
            0
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            let dc = wparam as HDC;
            // 原文、译文都和卡片同色
            let result = lparam as HWND == GetDlgItem(hwnd, RESULT as i32);
            let p = palette();
            SetBkColor(dc, colorref(p.card));
            // 译文稍深（深色模式下是稍亮），和原文区分
            SetTextColor(dc, colorref(if result { p.ink } else { p.ink_soft }));
            state.card
        }
        // 系统切换了深浅色：换底色画刷和边框，连输入框一起重画
        WM_SETTINGCHANGE if crate::view::is_color_change(lparam) => {
            if crate::view::follow_system() {
                windows_sys::Win32::Graphics::Gdi::DeleteObject(state.card as _);
                state.card = CreateSolidBrush(colorref(palette().card)) as isize;
                dark_frame(hwnd);
                windows_sys::Win32::Graphics::Gdi::RedrawWindow(
                    hwnd,
                    ptr::null(),
                    ptr::null_mut(),
                    windows_sys::Win32::Graphics::Gdi::RDW_INVALIDATE
                        | windows_sys::Win32::Graphics::Gdi::RDW_ERASE
                        | windows_sys::Win32::Graphics::Gdi::RDW_FRAME
                        | windows_sys::Win32::Graphics::Gdi::RDW_ALLCHILDREN,
                );
            }
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            note_close("窗口关闭");
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

/// 系统画的那一圈边框跟着深浅色走
unsafe fn dark_frame(window: HWND) {
    use windows_sys::Win32::Graphics::Dwm::*;
    let dark = u32::from(crate::view::dark());
    DwmSetWindowAttribute(
        window,
        DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
        (&dark as *const u32).cast(),
        4,
    );
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
    // 屏幕上要 WS_CLIPCHILDREN：不然卡片每次重画都盖过输入框，输入、等翻译时整窗狂闪（主人 2026-11-19）。
    // 离屏拍照不加：PrintWindow 截图里子控件会是黑块
    let style = WS_POPUP
        | WS_SYSMENU
        | WS_MINIMIZEBOX
        | WS_MAXIMIZEBOX
        | WS_THICKFRAME
        | if at.is_some() { 0 } else { WS_CLIPCHILDREN };
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
        card: CreateSolidBrush(colorref(palette().card)) as isize,
        asked: 0,
        in_flight: false,
        queued: false,
        replies,
        sender,
        last_source: String::new(),
        busy: false,
        phase: 0.,
        note: String::new(),
        copied: false,
        target: Target::English,
        manual: false,
        offscreen: at.is_some(),
        dragging: None,
        pressed: None,
        pace: Default::default(),
        last_auto: None,
        waiting: false,
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
    dark_frame(window);
    // 长文可以用滚轮看；右边的细滚动条是自己画的（参照 Husic，系统滚动条太粗，主人 2026-11-19）
    let edit = WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_MULTILINE | ES_AUTOVSCROLL;
    for (id, style) in [(SOURCE, edit | ES_WANTRETURN), (RESULT, edit | ES_READONLY)] {
        let control = CreateWindowExW(
            0,
            wide("EDIT").as_ptr(),
            wide("").as_ptr(),
            style,
            0,
            0,
            10,
            10,
            window,
            id as HMENU,
            module,
            ptr::null(),
        );
        let size = if id == RESULT { 1 } else { 0 };
        SendMessageW(
            control,
            WM_SETFONT,
            [font(17, 400, dpi, family), font(17, 500, dpi, family)][size] as usize,
            0,
        );
    }
    place_edits(window, &*state);
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
        wide("输入文字").as_ptr() as isize,
    );
    window
}

/// `xueer-ear.exe --translate`：开翻译小窗，关掉就退出进程
pub fn run() -> Result<(), String> {
    // 万一崩了，把原因记下来再退（release 是 panic=abort，不记就什么都看不到）
    std::panic::set_hook(Box::new(|info| note_close(&format!("崩溃：{info}"))));
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
        let _renderer = crate::view::Renderer::new()?;
        crate::view::follow_system();
        let window = open(None);
        if window.is_null() {
            return Err("打不开翻译小窗".into());
        }
        ShowWindow(window, SW_SHOW);
        SetForegroundWindow(window);
        SetFocus(GetDlgItem(window, SOURCE as i32));
        let mut message: MSG = mem::zeroed();
        while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {
            // 滚轮：没有系统滚动条的多行框不认滚轮，自己按行滚（主人 2026-11-19：滚轮滑不动）
            if message.message == WM_MOUSEWHEEL {
                let mut point = POINT {
                    x: (message.lParam as i16) as i32,
                    y: ((message.lParam >> 16) as i16) as i32,
                };
                ScreenToClient(window, &mut point);
                let layout = Layout::of(window, GetDpiForWindow(window).max(96));
                let dpi = GetDpiForWindow(window).max(96);
                for id in [SOURCE, RESULT] {
                    let (x, y, w, h) = if id == SOURCE {
                        layout.source()
                    } else {
                        layout.result()
                    };
                    if inside(point, (x, y, w + BAR_GAP + BAR_W + 4, h), dpi) {
                        let delta = ((message.wParam >> 16) as i16) as i32;
                        let lines = -delta * 3 / 120;
                        SendMessageW(
                            GetDlgItem(window, id as i32),
                            EM_LINESCROLL,
                            0,
                            lines as isize,
                        );
                        InvalidateRect(window, ptr::null(), 0);
                    }
                }
                continue;
            }
            if message.message == WM_KEYDOWN {
                if message.wParam == 0x1B {
                    note_close("按了 Esc");
                    DestroyWindow(window);
                    continue;
                }
                // Ctrl+A 全选当前框（系统多行输入框默认不支持；只读的译文框也要能全选复制，主人 2026-11-19）
                if message.wParam == 0x41 && GetKeyState(VK_CONTROL as i32) < 0 {
                    SendMessageW(message.hwnd, EM_SETSEL, 0, -1);
                    continue;
                }
                // 回车立刻翻译；Shift+回车照常换行
                if message.wParam == 0x0D && GetKeyState(VK_SHIFT as i32) >= 0 {
                    let state = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut State;
                    if !state.is_null() {
                        translate_now(window, &mut *state);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clearing_the_source_discards_an_in_flight_reply() {
        unsafe {
            let window = open(Some((-32000, -32000)));
            assert!(!window.is_null());
            set_text(window, SOURCE, "旧原文");
            let state = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut State;
            let old_id = (*state).asked;
            (*state).in_flight = true;
            // A plain EDIT does not send EN_CHANGE for every WM_SETTEXT path;
            // use the notification generated by an actual user edit.
            set_text(window, SOURCE, "");
            SendMessageW(window, WM_COMMAND, SOURCE | ((EN_CHANGE as usize) << 16), 0);
            (*state)
                .sender
                .send((old_id, Ok("stale translation".into())))
                .unwrap();
            SendMessageW(window, DONE, old_id, 0);
            let result = text_of(window, RESULT);
            DestroyWindow(window);
            assert!(result.is_empty(), "{result}");
        }
    }

    // 点了没反应的老毛病：按钮不在 buttons() 里，命中测试就当标题栏拖窗口，松手消息根本收不到
    #[test]
    fn every_clickable_part_is_a_button_for_hit_testing() {
        let layout = Layout {
            w: 420,
            h: 300,
            divider: 144,
        };
        let buttons = layout.buttons();
        for part in [
            layout.swap(),
            layout.close(),
            layout.maximize(),
            layout.minimize(),
            layout.copy(),
            layout.go(),
        ] {
            assert!(buttons.contains(&part), "{part:?} 不在 buttons() 里");
        }
    }
}
