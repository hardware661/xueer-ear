//! 雪耳自己的听写面板：样子仿 Win+H（左齿轮、中间麦克风、右关闭），屏幕底部居中、置顶、
//! 不抢焦点（WS_EX_NOACTIVATE），所以打字的目标窗口一直是你原来的那个。
//! 面板跑在自己的线程和消息循环里；其他线程通过 `PanelHandle` 改状态、收点击。
use std::{
    mem, ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
        mpsc::Sender,
    },
    thread,
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM},
    Graphics::{
        Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DwmSetWindowAttribute},
        Gdi::{
            AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
            CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC,
            GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromWindow, ReleaseDC,
            ScreenToClient, SelectObject,
        },
        GdiPlus as gp,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::GetDpiForWindow,
        WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DispatchMessageW, GWLP_USERDATA, GetCursorPos,
            GetForegroundWindow, GetMessageW, GetWindowLongPtrW, HTCAPTION, HTCLIENT, IDC_ARROW,
            KillTimer, LoadCursorW, MSG, PostMessageW, RegisterClassW, SW_HIDE, SW_SHOWNOACTIVATE,
            SetTimer, SetWindowLongPtrW, ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WM_APP,
            WM_LBUTTONUP, WM_NCHITTEST, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
            WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
        },
    },
};

/// 面板逻辑尺寸（96 DPI 下的像素）
const WIDTH: f32 = 228.;
const HEIGHT: f32 = 120.;
const REFRESH: u32 = WM_APP + 10;
const ANIMATE_TIMER: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Hidden,
    /// 蓝色麦克风，随音量起伏
    Listening,
    /// 正在连接云端
    Connecting,
    /// 灰色麦克风
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    Mic,
    Close,
    Settings,
}

struct Shared {
    look: Mutex<Look>,
    level: AtomicU32,
    clicks: Sender<Click>,
}

/// 在任意线程上改面板外观。
#[derive(Clone)]
pub struct PanelHandle {
    shared: Arc<Shared>,
    hwnd: Arc<Mutex<usize>>,
}

impl PanelHandle {
    pub fn set(&self, look: Look) {
        *self.shared.look.lock().unwrap_or_else(|e| e.into_inner()) = look;
        self.post();
    }

    /// 只有外观真的变了才刷新（轮询线程每 15 ms 调一次）
    pub fn set_if_changed(&self, look: Look) {
        let changed = {
            let mut current = self.shared.look.lock().unwrap_or_else(|e| e.into_inner());
            let changed = *current != look;
            *current = look;
            changed
        };
        if changed {
            self.post();
        }
    }

    /// 0～32767 的峰值，用来让麦克风周围的光圈跟着声音动
    pub fn level(&self, peak: u32) {
        self.shared.level.store(peak, Ordering::Relaxed);
    }

    fn post(&self) {
        let hwnd = *self.hwnd.lock().unwrap_or_else(|e| e.into_inner());
        if hwnd != 0 {
            unsafe {
                PostMessageW(hwnd as HWND, REFRESH, 0, 0);
            }
        }
    }
}

/// 起一个面板线程，返回句柄；点击通过 `clicks` 送回。
pub fn spawn(clicks: Sender<Click>) -> PanelHandle {
    let shared = Arc::new(Shared {
        look: Mutex::new(Look::Hidden),
        level: AtomicU32::new(0),
        clicks,
    });
    let hwnd = Arc::new(Mutex::new(0_usize));
    let handle = PanelHandle {
        shared: shared.clone(),
        hwnd: hwnd.clone(),
    };
    thread::spawn(move || unsafe { run(shared, hwnd) });
    handle
}

struct Window {
    shared: Arc<Shared>,
    hwnd: HWND,
    phase: f32,
    smooth: f32,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

unsafe fn run(shared: Arc<Shared>, slot: Arc<Mutex<usize>>) {
    unsafe {
        windows_sys::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(
            windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
        let mut input: gp::GdiplusStartupInput = mem::zeroed();
        input.GdiplusVersion = 1;
        let mut token = 0;
        if gp::GdiplusStartup(&mut token, &input, ptr::null_mut()) != 0 {
            eprintln!("面板：无法初始化绘图");
            return;
        }
        let module = GetModuleHandleW(ptr::null());
        let class_name = wide("XueerEar.DictationPanel");
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: module,
            hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
            lpszClassName: class_name.as_ptr(),
            ..mem::zeroed()
        };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            wide("雪耳").as_ptr(),
            WS_POPUP,
            0,
            0,
            1,
            1,
            ptr::null_mut(),
            ptr::null_mut(),
            module,
            ptr::null(),
        );
        if hwnd.is_null() {
            eprintln!("面板：无法创建窗口");
            return;
        }
        let corner = 2_u32;
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            (&corner as *const u32).cast(),
            4,
        );
        let window = Box::into_raw(Box::new(Window {
            shared,
            hwnd,
            phase: 0.,
            smooth: 0.,
        }));
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, window as isize);
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = hwnd as usize;
        let mut message: MSG = mem::zeroed();
        while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {
            DispatchMessageW(&message);
        }
        gp::GdiplusShutdown(token);
    }
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let window = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Window;
        if window.is_null() {
            return DefWindowProcW(hwnd, message, wp, lp);
        }
        let window = &mut *window;
        match message {
            REFRESH => {
                window.refresh();
                0
            }
            WM_TIMER => {
                window.phase += 0.18;
                window.draw();
                0
            }
            // 中间麦克风、左齿轮、右关闭是按钮；其余地方可以拖动面板
            WM_NCHITTEST => {
                let result = DefWindowProcW(hwnd, message, wp, lp);
                if result == HTCLIENT as isize && window.button_at_cursor().is_none() {
                    HTCAPTION as isize
                } else {
                    result
                }
            }
            WM_LBUTTONUP => {
                if let Some(click) = window.button_at_cursor() {
                    let _ = window.shared.clicks.send(click);
                }
                0
            }
            _ => DefWindowProcW(hwnd, message, wp, lp),
        }
    }
}

impl Window {
    fn look(&self) -> Look {
        *self.shared.look.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn scale(&self) -> f32 {
        unsafe { GetDpiForWindow(self.hwnd).max(96) as f32 / 96. }
    }

    unsafe fn refresh(&mut self) {
        unsafe {
            match self.look() {
                Look::Hidden => {
                    KillTimer(self.hwnd, ANIMATE_TIMER);
                    ShowWindow(self.hwnd, SW_HIDE);
                }
                look => {
                    self.place();
                    self.draw();
                    ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                    if matches!(look, Look::Listening | Look::Connecting) {
                        SetTimer(self.hwnd, ANIMATE_TIMER, 33, None);
                    } else {
                        KillTimer(self.hwnd, ANIMATE_TIMER);
                    }
                }
            }
        }
    }

    /// 放到前台窗口所在屏幕的底部居中，和 Win+H 面板的位置差不多
    unsafe fn place(&self) {
        unsafe {
            let monitor = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY);
            let mut info: MONITORINFO = mem::zeroed();
            info.cbSize = mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return;
            }
            let work = info.rcWork;
            let s = self.scale();
            let (w, h) = ((WIDTH * s) as i32, (HEIGHT * s) as i32);
            let x = work.left + (work.right - work.left - w) / 2;
            let y = work.bottom - h - (24. * s) as i32;
            windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPos(
                self.hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::HWND_TOPMOST,
                x,
                y,
                w,
                h,
                windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
            );
        }
    }

    fn button_at_cursor(&self) -> Option<Click> {
        unsafe {
            let mut point = POINT { x: 0, y: 0 };
            GetCursorPos(&mut point);
            ScreenToClient(self.hwnd, &mut point);
            let s = self.scale();
            let (x, y) = (point.x as f32 / s, point.y as f32 / s);
            hit(x, y)
        }
    }

    /// 用每像素透明的位图整体更新窗口（圆角、阴影都由 alpha 决定）
    unsafe fn draw(&mut self) {
        unsafe {
            let s = self.scale();
            let (w, h) = ((WIDTH * s) as i32, (HEIGHT * s) as i32);
            let screen = GetDC(ptr::null_mut());
            let dc = CreateCompatibleDC(screen);
            let mut info: BITMAPINFO = mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..mem::zeroed()
            };
            let mut bits = ptr::null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
            if bitmap.is_null() {
                DeleteDC(dc);
                ReleaseDC(ptr::null_mut(), screen);
                return;
            }
            let old = SelectObject(dc, bitmap);
            let target = self.target_level();
            self.smooth += (target - self.smooth) * 0.35;
            paint(dc, s, self.look(), self.smooth, self.phase);
            let size = SIZE { cx: w, cy: h };
            let source = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            UpdateLayeredWindow(
                self.hwnd,
                screen,
                ptr::null(),
                &size,
                dc,
                &source,
                0,
                &blend,
                ULW_ALPHA,
            );
            SelectObject(dc, old);
            DeleteObject(bitmap);
            DeleteDC(dc);
            ReleaseDC(ptr::null_mut(), screen);
        }
    }

    fn target_level(&self) -> f32 {
        match self.look() {
            Look::Listening => {
                let peak = self.shared.level.swap(0, Ordering::Relaxed) as f32;
                (peak / 6000.).clamp(0., 1.)
            }
            _ => 0.,
        }
    }
}

/// 按钮位置（96 DPI 逻辑坐标）：左齿轮、中麦克风、右关闭
fn hit(x: f32, y: f32) -> Option<Click> {
    let near = |cx: f32, cy: f32, r: f32| (x - cx).powi(2) + (y - cy).powi(2) <= r * r;
    if near(WIDTH / 2., 70., 28.) {
        Some(Click::Mic)
    } else if near(WIDTH - 22., 20., 14.) {
        Some(Click::Close)
    } else if near(36., 70., 18.) {
        Some(Click::Settings)
    } else {
        None
    }
}

const PANEL_FILL: u32 = 0xf5f3f3f6;
const PANEL_EDGE: u32 = 0x33000000;
const INK: u32 = 0xff1c1c1e;
const MUTED: u32 = 0xff8e8e93;
const BLUE: u32 = 0xff0a64d8;
const HALO: u32 = 0x330a64d8;
const GREY_FILL: u32 = 0xffe5e5ea;

unsafe fn paint(
    dc: windows_sys::Win32::Graphics::Gdi::HDC,
    scale: f32,
    look: Look,
    level: f32,
    phase: f32,
) {
    unsafe {
        let mut g = ptr::null_mut();
        if gp::GdipCreateFromHDC(dc, &mut g) != 0 {
            return;
        }
        gp::GdipSetSmoothingMode(g, gp::SmoothingModeAntiAlias8x8);
        gp::GdipScaleWorldTransform(g, scale, scale, gp::MatrixOrderPrepend);
        // 面板底：浅色磨砂感圆角卡片 + 细描边
        rounded(g, 0.5, 0.5, WIDTH - 1., HEIGHT - 1., 12., PANEL_FILL, false);
        rounded(g, 0.5, 0.5, WIDTH - 1., HEIGHT - 1., 12., PANEL_EDGE, true);
        // 顶部拖动条
        rounded(g, WIDTH / 2. - 18., 10., 36., 4., 2., 0xffc7c7cc, false);
        // 右上关闭 ×
        line(g, WIDTH - 27., 15., WIDTH - 17., 25., 1.6, INK);
        line(g, WIDTH - 17., 15., WIDTH - 27., 25., 1.6, INK);
        // 左边齿轮
        gear(g, 36., 70., MUTED);
        // 中间麦克风按钮
        let (cx, cy) = (WIDTH / 2., 70.);
        match look {
            Look::Listening | Look::Connecting => {
                let pulse = if look == Look::Connecting {
                    0.25 + 0.25 * phase.sin().abs()
                } else {
                    level
                };
                let r = 26. + 8. * pulse;
                ellipse(g, cx - r, cy - r, r * 2., r * 2., HALO);
                ellipse(g, cx - 24., cy - 24., 48., 48., BLUE);
                mic(g, cx - 10., cy - 13., 20., 0xffffffff);
            }
            _ => {
                ellipse(g, cx - 24., cy - 24., 48., 48., GREY_FILL);
                mic(g, cx - 10., cy - 13., 20., MUTED);
            }
        }
        // 右边一个小兽耳：雪耳的记号
        ear(
            g,
            WIDTH - 40.,
            60.,
            if look == Look::Idle { MUTED } else { INK },
        );
        gp::GdipDeleteGraphics(g);
    }
}

unsafe fn rounded(
    g: *mut gp::GpGraphics,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r: f32,
    color: u32,
    outline: bool,
) {
    unsafe {
        let mut path = ptr::null_mut();
        if gp::GdipCreatePath(gp::FillModeAlternate, &mut path) != 0 {
            return;
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
        if outline {
            let mut pen = ptr::null_mut();
            if gp::GdipCreatePen1(color, 1., gp::UnitPixel, &mut pen) == 0 {
                gp::GdipDrawPath(g, pen, path);
                gp::GdipDeletePen(pen);
            }
        } else {
            let mut brush = ptr::null_mut();
            if gp::GdipCreateSolidFill(color, &mut brush) == 0 {
                gp::GdipFillPath(g, brush.cast(), path);
                gp::GdipDeleteBrush(brush.cast());
            }
        }
        gp::GdipDeletePath(path);
    }
}

unsafe fn ellipse(g: *mut gp::GpGraphics, x: f32, y: f32, w: f32, h: f32, color: u32) {
    unsafe {
        let mut brush = ptr::null_mut();
        if gp::GdipCreateSolidFill(color, &mut brush) == 0 {
            gp::GdipFillEllipse(g, brush.cast(), x, y, w, h);
            gp::GdipDeleteBrush(brush.cast());
        }
    }
}

unsafe fn line(g: *mut gp::GpGraphics, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, color: u32) {
    unsafe {
        let mut pen = ptr::null_mut();
        if gp::GdipCreatePen1(color, width, gp::UnitPixel, &mut pen) == 0 {
            gp::GdipSetPenStartCap(pen, gp::LineCapRound);
            gp::GdipSetPenEndCap(pen, gp::LineCapRound);
            gp::GdipDrawLine(g, pen, x1, y1, x2, y2);
            gp::GdipDeletePen(pen);
        }
    }
}

unsafe fn arc(
    g: *mut gp::GpGraphics,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    start: f32,
    sweep: f32,
    width: f32,
    color: u32,
) {
    unsafe {
        let mut pen = ptr::null_mut();
        if gp::GdipCreatePen1(color, width, gp::UnitPixel, &mut pen) == 0 {
            gp::GdipSetPenStartCap(pen, gp::LineCapRound);
            gp::GdipSetPenEndCap(pen, gp::LineCapRound);
            gp::GdipDrawArc(g, pen, x, y, w, h, start, sweep);
            gp::GdipDeletePen(pen);
        }
    }
}

unsafe fn mic(g: *mut gp::GpGraphics, x: f32, y: f32, size: f32, color: u32) {
    unsafe {
        rounded(
            g,
            x + size * 0.32,
            y,
            size * 0.36,
            size * 0.62,
            size * 0.18,
            color,
            false,
        );
        arc(
            g,
            x + size * 0.14,
            y + size * 0.3,
            size * 0.72,
            size * 0.56,
            0.,
            180.,
            size * 0.08,
            color,
        );
        line(
            g,
            x + size * 0.5,
            y + size * 0.86,
            x + size * 0.5,
            y + size * 1.08,
            size * 0.08,
            color,
        );
    }
}

unsafe fn gear(g: *mut gp::GpGraphics, cx: f32, cy: f32, color: u32) {
    unsafe {
        for i in 0..8 {
            let a = i as f32 * std::f32::consts::TAU / 8.;
            line(
                g,
                cx + a.cos() * 6.5,
                cy + a.sin() * 6.5,
                cx + a.cos() * 9.5,
                cy + a.sin() * 9.5,
                2.6,
                color,
            );
        }
        arc(g, cx - 6.5, cy - 6.5, 13., 13., 0., 360., 1.8, color);
        arc(g, cx - 2.5, cy - 2.5, 5., 5., 0., 360., 1.6, color);
    }
}

/// 一对小兽耳（和图标一样），雪耳的记号
unsafe fn ear(g: *mut gp::GpGraphics, x: f32, y: f32, color: u32) {
    unsafe {
        for dx in [0., 11.] {
            let points = [
                gp::PointF {
                    X: x + dx,
                    Y: y + 18.,
                },
                gp::PointF {
                    X: x + dx + 3.5,
                    Y: y + 2.,
                },
                gp::PointF {
                    X: x + dx + 8.,
                    Y: y + 18.,
                },
            ];
            let mut brush = ptr::null_mut();
            if gp::GdipCreateSolidFill(color, &mut brush) == 0 {
                gp::GdipFillPolygon(g, brush.cast(), points.as_ptr(), 3, gp::FillModeAlternate);
                gp::GdipDeleteBrush(brush.cast());
            }
        }
    }
}
