//! 雪耳自己的听写面板：一张带一对猫耳的小卡片，中间一个麦克风；固定在屏幕顶部居中、置顶、
//! 不抢焦点（WS_EX_NOACTIVATE），所以打字的目标窗口一直是你原来的那个。点卡片任意处 = 点麦克风。
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
        Gdi::{
            AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
            CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC,
            GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromWindow, ReleaseDC,
            SelectObject,
        },
        GdiPlus as gp,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::GetDpiForWindow,
        WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DispatchMessageW, GWLP_USERDATA, GetForegroundWindow,
            GetMessageW, GetWindowLongPtrW, IDC_ARROW, KillTimer, LoadCursorW, MSG, PostMessageW,
            RegisterClassW, SW_HIDE, SW_SHOWNOACTIVATE, SetTimer, SetWindowLongPtrW, ShowWindow,
            ULW_ALPHA, UpdateLayeredWindow, WM_APP, WM_LBUTTONUP, WM_TIMER, WNDCLASSW,
            WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
        },
    },
};

/// 面板逻辑尺寸（96 DPI 下的像素）；上面留出猫耳的高度
const WIDTH: f32 = 120.;
const HEIGHT: f32 = 100.;
/// 卡片本体从这里开始，上面是耳朵
const BODY_TOP: f32 = 24.;
const REFRESH: u32 = WM_APP + 10;
const ANIMATE_TIMER: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Hidden,
    /// 蓝色麦克风：在听、还没听到说话
    Listening,
    /// 听到在说话：中间变成随音量起伏的波浪
    Speaking,
    /// 正在连接云端
    Connecting,
    /// 灰色麦克风（只在离屏预览里用）
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    Mic,
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
        // 不要 DWM 圆角：Windows 11 会顺带给窗口加一圈灰边和阴影（主人不要边框），形状全靠位图透明度
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
            WM_LBUTTONUP => {
                let _ = window.shared.clicks.send(Click::Mic);
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
                    if matches!(look, Look::Listening | Look::Speaking | Look::Connecting) {
                        SetTimer(self.hwnd, ANIMATE_TIMER, 33, None);
                    } else {
                        KillTimer(self.hwnd, ANIMATE_TIMER);
                    }
                }
            }
        }
    }

    /// 放到前台窗口所在屏幕的顶部居中
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
            // 顶部居中（主人 2026-11-15：放底部会挡住输入框）
            let x = work.left + (work.right - work.left - w) / 2;
            let y = work.top + (12. * s) as i32;
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
            Look::Listening | Look::Speaking => {
                let peak = self.shared.level.swap(0, Ordering::Relaxed) as f32;
                (peak / 6000.).clamp(0., 1.)
            }
            _ => 0.,
        }
    }
}

const PANEL_FILL: u32 = 0xfff2f2f5;
const INNER_EAR: u32 = 0xfff1d9df;
const MUTED: u32 = 0xff8e8e93;
/// 在听、在说话：深灰（主人不喜欢蓝色）
const BLUE: u32 = 0xff3a3a3c;
const HALO: u32 = 0x263a3a3c;
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
        paint_graphics(g, scale, look, level, phase);
        gp::GdipDeleteGraphics(g);
    }
}

unsafe fn paint_graphics(g: *mut gp::GpGraphics, scale: f32, look: Look, level: f32, phase: f32) {
    unsafe {
        gp::GdipSetSmoothingMode(g, gp::SmoothingModeAntiAlias8x8);
        gp::GdipScaleWorldTransform(g, scale, scale, gp::MatrixOrderPrepend);
        let body_h = HEIGHT - BODY_TOP - 6.;
        // 一对大一点的猫耳（左上、右上），和卡片连成一体
        cat_ear(
            g,
            8.,
            BODY_TOP + 16.,
            44.,
            BODY_TOP + 16.,
            16.,
            1.,
            PANEL_FILL,
            0,
        );
        cat_ear(
            g,
            WIDTH - 44.,
            BODY_TOP + 16.,
            WIDTH - 8.,
            BODY_TOP + 16.,
            WIDTH - 16.,
            1.,
            PANEL_FILL,
            0,
        );
        rounded(g, 3., BODY_TOP, WIDTH - 6., body_h, 22., PANEL_FILL, false);
        // 内耳一点粉
        cat_ear(
            g,
            17.,
            BODY_TOP + 10.,
            35.,
            BODY_TOP + 10.,
            20.,
            9.,
            INNER_EAR,
            0,
        );
        cat_ear(
            g,
            WIDTH - 35.,
            BODY_TOP + 10.,
            WIDTH - 17.,
            BODY_TOP + 10.,
            WIDTH - 20.,
            9.,
            INNER_EAR,
            0,
        );
        // 中间：浅灰麦克风=在听、还没听到说话；深灰底白波浪=正在出字（主人：没说话时麦克风应该是淡灰）
        let (cx, cy) = (WIDTH / 2., BODY_TOP + body_h / 2.);
        match look {
            Look::Speaking => {
                let r = 24. + 6. * level;
                ellipse(g, cx - r, cy - r, r * 2., r * 2., HALO);
                ellipse(g, cx - 22., cy - 22., 44., 44., BLUE);
                wave(g, cx, cy, level, phase);
            }
            Look::Connecting => {
                let r = 24. + 6. * (0.25 + 0.25 * phase.sin().abs());
                ellipse(g, cx - r, cy - r, r * 2., r * 2., HALO);
                ellipse(g, cx - 22., cy - 22., 44., 44., GREY_FILL);
                mic(g, cx - 9., cy - 12., 18., MUTED);
            }
            _ => {
                ellipse(g, cx - 22., cy - 22., 44., 44., GREY_FILL);
                mic(g, cx - 9., cy - 12., 18., MUTED);
            }
        }
    }
}

/// 说话时的波浪：五根圆头竖条，高度随音量和时间起伏
unsafe fn wave(g: *mut gp::GpGraphics, cx: f32, cy: f32, level: f32, phase: f32) {
    unsafe {
        let shape = [0.45, 0.75, 1.0, 0.75, 0.45];
        for (i, k) in shape.iter().enumerate() {
            let wobble = ((phase * 1.7 + i as f32 * 1.1).sin() * 0.5 + 0.5) * 0.6 + 0.4;
            let h = 5. + 20. * k * (0.25 + 0.75 * level) * wobble;
            let x = cx + (i as f32 - 2.) * 6.5;
            line(g, x, cy - h / 2., x, cy + h / 2., 4.4, 0xffffffff);
        }
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

/// 一只猫耳：三角形 (x1,y1)-(x2,y2) 是耳根，(tx,ty) 是耳尖；edge 为 0 就不描边
unsafe fn cat_ear(
    g: *mut gp::GpGraphics,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    tx: f32,
    ty: f32,
    fill: u32,
    edge: u32,
) {
    unsafe {
        let mut path = ptr::null_mut();
        if gp::GdipCreatePath(gp::FillModeAlternate, &mut path) != 0 {
            return;
        }
        // 耳尖用一小段圆弧，别太尖
        gp::GdipAddPathLine(path, x1, y1, tx - 2., ty + 3.);
        gp::GdipAddPathBezier(path, tx - 2., ty + 3., tx, ty, tx, ty, tx + 2., ty + 3.);
        gp::GdipAddPathLine(path, tx + 2., ty + 3., x2, y2);
        gp::GdipClosePathFigure(path);
        let mut brush = ptr::null_mut();
        if gp::GdipCreateSolidFill(fill, &mut brush) == 0 {
            gp::GdipFillPath(g, brush.cast(), path);
            gp::GdipDeleteBrush(brush.cast());
        }
        if edge != 0 {
            let mut pen = ptr::null_mut();
            if gp::GdipCreatePen1(edge, 1., gp::UnitPixel, &mut pen) == 0 {
                gp::GdipSetPenLineJoin(pen, gp::LineJoinRound);
                gp::GdipDrawPath(g, pen, path);
                gp::GdipDeletePen(pen);
            }
        }
        gp::GdipDeletePath(path);
    }
}

/// 离屏把面板画成 PNG（核对外观用，不在屏幕上弹窗）
pub fn render_png(look: Look, level: f32, scale: f32, out: &std::path::Path) -> Result<(), String> {
    unsafe {
        let mut input: gp::GdiplusStartupInput = mem::zeroed();
        input.GdiplusVersion = 1;
        let mut token = 0;
        if gp::GdiplusStartup(&mut token, &input, ptr::null_mut()) != 0 {
            return Err("无法初始化绘图".into());
        }
        let (w, h) = ((WIDTH * scale) as i32, (HEIGHT * scale) as i32);
        let mut bitmap = ptr::null_mut();
        let result = if gp::GdipCreateBitmapFromScan0(
            w,
            h,
            0,
            0x0026200A, // PixelFormat32bppARGB
            ptr::null(),
            &mut bitmap,
        ) != 0
        {
            Err("无法创建位图".to_string())
        } else {
            let mut g = ptr::null_mut();
            gp::GdipGetImageGraphicsContext(bitmap.cast(), &mut g);
            // 白底方便看
            gp::GdipGraphicsClear(g, 0xffffffff);
            paint_graphics(g, scale, look, level, 0.);
            gp::GdipDeleteGraphics(g);
            let clsid = windows_sys::core::GUID::from_u128(0x557cf406_1a04_11d3_9a73_0000f81ef32e);
            let path: Vec<u16> = out
                .as_os_str()
                .to_string_lossy()
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let status = gp::GdipSaveImageToFile(bitmap.cast(), path.as_ptr(), &clsid, ptr::null());
            gp::GdipDisposeImage(bitmap.cast());
            if status == 0 {
                Ok(())
            } else {
                Err(format!("保存失败 {status}"))
            }
        };
        gp::GdiplusShutdown(token);
        result
    }
}
