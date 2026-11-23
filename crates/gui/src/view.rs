//! Antialiased Fluent decoration; native child controls retain input and focus.
#![allow(unsafe_op_in_unsafe_fn)]
use std::{
    mem, ptr,
    sync::atomic::{AtomicBool, Ordering},
};
use windows_sys::Win32::Graphics::{Gdi::HDC, GdiPlus as gp};

pub const WIDTH: i32 = 400;
pub const HEADER: i32 = 42;
pub const HEIGHT: i32 = 588;

/// 一套界面颜色（0xRRGGBB）。浅色是雪耳原来的黑白灰；深色参照 macOS 深色模式。
/// GDI+ 画图用 `argb`，GDI 的字和画刷用 `colorref`
pub struct Palette {
    /// 页面底：标题栏、分组小标题所在的灰底
    pub page: u32,
    /// 卡片、更新气泡、菜单、翻译小窗、对话框的底
    pub card: u32,
    /// 卡片里的分隔线；标题栏按钮按下时的底；关着的麦克风圆底
    pub line: u32,
    /// 气泡、胶囊的细边，翻译小窗的分隔线，待机波形的点
    pub edge: u32,
    /// 正文
    pub ink: u32,
    /// 比正文浅一点：翻译小窗的原文、方向胶囊里的字和箭头
    pub ink_soft: u32,
    /// 次要文字、小图标
    pub muted: u32,
    /// 更淡的记号：值右边的 ›、滚动条
    pub faint: u32,
    /// 细线小图标：方向胶囊的箭头、更新气泡的 ×
    pub glyph: u32,
    /// 翻译中跳动的三个小点
    pub dots: u32,
    /// 主色：开着的麦克风和开关、「翻译」按钮、说话时的波形（浅色是深灰，深色是浅灰）
    pub accent: u32,
    /// 主色上面的图标和字
    pub on_accent: u32,
    /// 出错的红字
    pub danger: u32,
    /// 卡片上的东西按下时的底
    pub pressed: u32,
    /// 普通按钮：底、选中的底、按下的底、边、选中的边
    pub button: u32,
    pub button_selected: u32,
    pub button_pressed: u32,
    pub button_edge: u32,
    pub button_edge_selected: u32,
    /// 开关：关着的轨道、不能点的轨道、关着的圆钮
    pub toggle_off: u32,
    pub toggle_disabled: u32,
    pub knob_off: u32,
    /// 菜单里鼠标停住的那一行，和上面的字
    pub hover: u32,
    pub on_hover: u32,
    /// 卡片下面的影子（0xAARRGGBB，带透明度）
    pub shadow: u32,
    /// 菜单外面一圈细边（0xAARRGGBB）：浅色是淡黑，深色是淡白
    pub hairline: u32,
}

pub const LIGHT: Palette = Palette {
    page: 0xf2f2f7,
    card: 0xffffff,
    line: 0xe5e5ea,
    edge: 0xd1d1d6,
    ink: 0x1d1d1f,
    ink_soft: 0x3a3a3c,
    muted: 0x8e8e93,
    faint: 0xc7c7cc,
    glyph: 0x636366,
    dots: 0xaeaeb2,
    accent: 0x2c2c2e,
    on_accent: 0xffffff,
    danger: 0xbd393f,
    pressed: 0xf2f2f7,
    button: 0xf0f3f7,
    button_selected: 0xe8e8ed,
    button_pressed: 0xdcdce1,
    button_edge: 0xe5eaf0,
    button_edge_selected: 0xaeaeb2,
    toggle_off: 0xbbc5d2,
    toggle_disabled: 0xd5dce5,
    knob_off: 0xffffff,
    hover: 0x3a3a3c,
    on_hover: 0xffffff,
    shadow: 0x1a000000,
    hairline: 0x1a000000,
};

pub const DARK: Palette = Palette {
    page: 0x1c1c1e,
    card: 0x2c2c2e,
    line: 0x3a3a3c,
    edge: 0x48484a,
    ink: 0xf2f2f7,
    ink_soft: 0xd1d1d6,
    muted: 0x98989d,
    faint: 0x636366,
    glyph: 0xaeaeb2,
    dots: 0x8e8e93,
    accent: 0xe5e5ea,
    on_accent: 0x1c1c1e,
    danger: 0xff6961,
    pressed: 0x3a3a3c,
    button: 0x3a3a3c,
    button_selected: 0x48484a,
    button_pressed: 0x545458,
    button_edge: 0x48484a,
    button_edge_selected: 0x8e8e93,
    toggle_off: 0x48484a,
    toggle_disabled: 0x3a3a3c,
    knob_off: 0x98989d,
    hover: 0x5a5a5e,
    on_hover: 0xffffff,
    shadow: 0x59000000,
    hairline: 0x24ffffff,
};

/// 深色模式：跟随系统（设置 → 个性化 → 颜色 → 应用模式）。启动时和收到 WM_SETTINGCHANGE 时读一次
static DARK_MODE: AtomicBool = AtomicBool::new(false);

pub fn dark() -> bool {
    DARK_MODE.load(Ordering::Relaxed)
}

/// 离屏拍照时指定深浅
pub fn set_dark(dark: bool) {
    DARK_MODE.store(dark, Ordering::Relaxed);
}

/// 按系统现在的设置定深浅，返回变没变
pub fn follow_system() -> bool {
    let dark = system_dark();
    DARK_MODE.swap(dark, Ordering::Relaxed) != dark
}

/// 系统「应用模式」是不是深色：AppsUseLightTheme 为 0；读不到（老系统、被策略删掉）算浅色
pub fn system_dark() -> bool {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    let key: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let name: Vec<u16> = "AppsUseLightTheme".encode_utf16().chain(Some(0)).collect();
    let mut value = 1_u32;
    let mut size = mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            ptr::null_mut(),
            (&mut value as *mut u32).cast(),
            &mut size,
        )
    };
    status == 0 && value == 0
}

/// WM_SETTINGCHANGE 带的是不是「颜色设置变了」（lParam 指向 "ImmersiveColorSet"）
pub unsafe fn is_color_change(lparam: isize) -> bool {
    if lparam == 0 {
        return false;
    }
    let text = lparam as *const u16;
    let expected: Vec<u16> = "ImmersiveColorSet".encode_utf16().chain(Some(0)).collect();
    expected
        .iter()
        .enumerate()
        .all(|(i, &unit)| *text.add(i) == unit)
}

pub fn palette() -> &'static Palette {
    if dark() { &DARK } else { &LIGHT }
}

/// GDI+ 用的不透明颜色 0xAARRGGBB
pub const fn argb(rgb: u32) -> u32 {
    0xff00_0000 | rgb
}

/// GDI 用的 COLORREF（0x00BBGGRR）
pub const fn colorref(rgb: u32) -> u32 {
    ((rgb & 0xff) << 16) | (rgb & 0xff00) | ((rgb >> 16) & 0xff)
}

pub struct Renderer(usize);
impl Renderer {
    pub fn new() -> Result<Self, String> {
        unsafe {
            let mut input: gp::GdiplusStartupInput = mem::zeroed();
            input.GdiplusVersion = 1;
            let mut token = 0;
            if gp::GdiplusStartup(&mut token, &input, ptr::null_mut()) != 0 {
                return Err("无法初始化界面绘图".into());
            }
            Ok(Self(token))
        }
    }
}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe { gp::GdiplusShutdown(self.0) };
    }
}

struct Canvas(*mut gp::GpGraphics);
impl Canvas {
    unsafe fn polygon(&self, points: &[(f32, f32)], color: u32, outline: bool) {
        let points = points
            .iter()
            .map(|&(x, y)| gp::PointF { X: x, Y: y })
            .collect::<Vec<_>>();
        let mut path = ptr::null_mut();
        if gp::GdipCreatePath(gp::FillModeAlternate, &mut path) != 0 {
            return;
        }
        gp::GdipAddPathPolygon(path, points.as_ptr(), points.len() as i32);
        if outline {
            let mut pen = ptr::null_mut();
            if gp::GdipCreatePen1(color, 1.5, gp::UnitPixel, &mut pen) == 0 {
                gp::GdipSetPenLineJoin(pen, gp::LineJoinRound);
                gp::GdipDrawPath(self.0, pen, path);
                gp::GdipDeletePen(pen);
            }
        } else {
            let mut brush = ptr::null_mut();
            if gp::GdipCreateSolidFill(color, &mut brush) == 0 {
                gp::GdipFillPath(self.0, brush.cast(), path);
                gp::GdipDeleteBrush(brush.cast());
            }
        }
        gp::GdipDeletePath(path);
    }
    unsafe fn new(dc: HDC, dpi: u32) -> Option<Self> {
        let mut graphics = ptr::null_mut();
        if gp::GdipCreateFromHDC(dc, &mut graphics) != 0 {
            return None;
        }
        gp::GdipSetSmoothingMode(graphics, gp::SmoothingModeAntiAlias8x8);
        gp::GdipScaleWorldTransform(
            graphics,
            dpi as f32 / 96.,
            dpi as f32 / 96.,
            gp::MatrixOrderPrepend,
        );
        Some(Self(graphics))
    }
    /// 直角矩形（rounded 半径为 0 时路径画不出来）
    unsafe fn rect(&self, x: f32, y: f32, w: f32, h: f32, color: u32) {
        let mut brush = ptr::null_mut();
        if gp::GdipCreateSolidFill(color, &mut brush) == 0 {
            gp::GdipFillRectangle(self.0, brush.cast(), x, y, w, h);
            gp::GdipDeleteBrush(brush.cast());
        }
    }
    unsafe fn ellipse(&self, x: f32, y: f32, w: f32, h: f32, color: u32) {
        let mut brush = ptr::null_mut();
        if gp::GdipCreateSolidFill(color, &mut brush) == 0 {
            gp::GdipFillEllipse(self.0, brush.cast(), x, y, w, h);
            gp::GdipDeleteBrush(brush.cast());
        }
    }
    unsafe fn rounded(
        &self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        color: u32,
        outline: bool,
    ) {
        let mut path = ptr::null_mut();
        if gp::GdipCreatePath(gp::FillModeAlternate, &mut path) != 0 {
            return;
        }
        let d = radius * 2.;
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
                gp::GdipDrawPath(self.0, pen, path);
                gp::GdipDeletePen(pen);
            }
        } else {
            let mut brush = ptr::null_mut();
            if gp::GdipCreateSolidFill(color, &mut brush) == 0 {
                gp::GdipFillPath(self.0, brush.cast(), path);
                gp::GdipDeleteBrush(brush.cast());
            }
        }
        gp::GdipDeletePath(path);
    }
    unsafe fn line(&self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, color: u32) {
        let mut pen = ptr::null_mut();
        if gp::GdipCreatePen1(color, width, gp::UnitPixel, &mut pen) == 0 {
            gp::GdipSetPenStartCap(pen, gp::LineCapRound);
            gp::GdipSetPenEndCap(pen, gp::LineCapRound);
            gp::GdipDrawLine(self.0, pen, x1, y1, x2, y2);
            gp::GdipDeletePen(pen);
        }
    }
    unsafe fn arc(
        &self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        start: f32,
        sweep: f32,
        width: f32,
        color: u32,
    ) {
        let mut pen = ptr::null_mut();
        if gp::GdipCreatePen1(color, width, gp::UnitPixel, &mut pen) == 0 {
            gp::GdipSetPenStartCap(pen, gp::LineCapRound);
            gp::GdipSetPenEndCap(pen, gp::LineCapRound);
            gp::GdipDrawArc(self.0, pen, x, y, w, h, start, sweep);
            gp::GdipDeletePen(pen);
        }
    }
}
impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            gp::GdipDeleteGraphics(self.0);
        }
    }
}

/// 卡片的位置（96 DPI）：状态卡、「听写」组、「文字」组、更多设置
pub const CARDS: [(f32, f32); 4] = [(52., 72.), (170., 132.), (336., 176.), (528., 44.)];
/// 一行多高
pub const ROW: f32 = 44.;
/// 系统设置式：浅灰底上几张白色圆角卡片（深色是深灰底上的灰卡片）；
/// 状态卡里是小麦克风、波形（文字和开关是子控件）
pub unsafe fn background(dc: HDC, dpi: u32, enabled: bool, peak: u32, frame: u64) {
    let Some(c) = Canvas::new(dc, dpi) else {
        return;
    };
    let p = palette();
    for (i, (y, h)) in CARDS.into_iter().enumerate() {
        c.rounded(16., y, 368., h, 10., argb(p.card), false);
        if i == 0 {
            continue;
        }
        // 卡片里的行之间一条细线，从字的左边开始（像 macOS 设置）
        let mut line = y + ROW;
        while line < y + h - 1. {
            c.line(32., line, 384., line, 1., argb(p.line));
            line += ROW;
        }
    }
    // 小麦克风：开着是主色底的麦（浅色深灰底白麦），关着是浅一档的底、灰麦
    let (fill, ink) = if enabled {
        (argb(p.accent), argb(p.on_accent))
    } else {
        (argb(p.line), argb(p.muted))
    };
    c.ellipse(32., 66., 44., 44., fill);
    c.rounded(49., 76., 10., 17., 5., ink, false);
    c.arc(45., 82., 18., 18., 0., 180., 1.8, ink);
    c.line(54., 100., 54., 104., 1.8, ink);
    // 波形：说话时随音量跳，平时是一排小点
    for i in 0..22 {
        let activity = (peak as f32 / 8000.).clamp(0., 1.);
        let shape = ((i as f32 * 1.13 - frame as f32 * 0.32).sin().abs() * 0.75 + 0.25)
            * (1. - (i as f32 - 11.).abs() / 16.);
        let h = if peak > 0 {
            3. + 12. * activity * shape
        } else {
            3.
        };
        c.rounded(
            92. + i as f32 * 6.,
            102. - h / 2.,
            3.,
            h,
            1.5,
            if peak > 0 && enabled {
                argb(p.accent)
            } else {
                argb(p.edge)
            },
            false,
        );
    }
}

/// 行右边的值：灰字（由调用方写）+ 一个小 ›；按下时整块浅灰
pub unsafe fn value_button(dc: HDC, dpi: u32, width: i32, height: i32, pressed: bool) {
    let Some(c) = Canvas::new(dc, dpi) else {
        return;
    };
    let w = width as f32 * 96. / dpi as f32;
    let h = height as f32 * 96. / dpi as f32;
    let p = palette();
    if pressed {
        c.rounded(0., 2., w, h - 4., 6., argb(p.pressed), false);
    }
    c.line(w - 12., h / 2. - 4., w - 8., h / 2., 1.6, argb(p.faint));
    c.line(w - 8., h / 2., w - 12., h / 2. + 4., 1.6, argb(p.faint));
}

pub unsafe fn update_bubble(dc: HDC, dpi: u32, available: bool) {
    let Some(c) = Canvas::new(dc, dpi) else {
        return;
    };
    let p = palette();
    let h = if available { 126. } else { 78. };
    c.rounded(122., 49., 264., h, 10., p.shadow, false);
    c.rounded(120., 46., 264., h, 10., argb(p.card), false);
    c.rounded(120., 46., 264., h, 10., argb(p.edge), true);
    c.polygon(
        &[(278., 47.), (284., 40.), (290., 47.)],
        argb(p.card),
        false,
    );
}

pub unsafe fn update_badge(dc: HDC, dpi: u32) {
    let Some(c) = Canvas::new(dc, dpi) else {
        return;
    };
    let p = palette();
    c.ellipse(23., 3., 8., 8., argb(p.page));
    c.ellipse(24., 4., 6., 6., argb(p.accent));
}

pub unsafe fn close_glyph(dc: HDC, dpi: u32, width: i32, height: i32) {
    let Some(c) = Canvas::new(dc, dpi) else {
        return;
    };
    let x = width as f32 * 48. / dpi as f32;
    let y = height as f32 * 48. / dpi as f32;
    let glyph = argb(palette().glyph);
    c.line(x - 3., y - 3., x + 3., y + 3., 1.3, glyph);
    c.line(x - 3., y + 3., x + 3., y - 3., 1.3, glyph);
}

pub unsafe fn button(
    dc: HDC,
    dpi: u32,
    width: i32,
    height: i32,
    toggle: bool,
    selected: bool,
    disabled: bool,
    pressed: bool,
) {
    let Some(c) = Canvas::new(dc, dpi) else {
        return;
    };
    let w = width as f32 * 96. / dpi as f32;
    let h = height as f32 * 96. / dpi as f32;
    let p = palette();
    if toggle {
        let fill = if disabled {
            p.toggle_disabled
        } else if selected {
            p.accent
        } else {
            p.toggle_off
        };
        c.rounded(1., (h - 26.) / 2., w - 2., 26., 13., argb(fill), false);
        c.ellipse(
            if selected { w - 24. } else { 4. },
            (h - 20.) / 2.,
            20.,
            20.,
            argb(if selected { p.on_accent } else { p.knob_off }),
        );
    } else {
        let fill = if pressed {
            p.button_pressed
        } else if selected {
            p.button_selected
        } else {
            p.button
        };
        c.rounded(0.5, 0.5, w - 1., h - 1., 6., argb(fill), false);
        c.rounded(
            0.5,
            0.5,
            w - 1.,
            h - 1.,
            6.,
            argb(if selected {
                p.button_edge_selected
            } else {
                p.button_edge
            }),
            true,
        );
    }
}

/// 翻译小窗的底（主人 2026-11-19 第三版，参照 macOS「翻译」）：上半白底写原文，下半浅灰底出译文，
/// 两块的颜色分界就是分隔线；右上一个小 ×，下半右下一个复制图标（复制后变成对勾），翻译中三个小点。
pub unsafe fn translate_card(
    dc: HDC,
    dpi: u32,
    width: i32,
    height: i32,
    split: f32,
    busy_phase: Option<f32>,
    copied: bool,
    maximized: bool,
) {
    let Some(c) = Canvas::new(dc, dpi) else {
        return;
    };
    let w = width as f32 * 96. / dpi as f32;
    let h = height as f32 * 96. / dpi as f32;
    let p = palette();
    let (card, muted) = (argb(p.card), argb(p.muted));
    // 整张白底（深色是灰底）；分隔线（主人 2026-11-19：加回来）上下各留距离，中间空出放「中 ⇄ 英」的小胶囊。
    // 译文那半不再整块铺灰，只靠线和留白分开，胶囊不会骑在两种底色的接缝上
    c.rect(0., 0., w, h, card);
    let gap = 60.;
    c.line(18., split, w / 2. - gap, split, 1., argb(p.edge));
    c.line(w / 2. + gap, split, w - 18., split, 1., argb(p.edge));
    // 右上三个小按钮：最小化、最大化/还原、关闭（主人 2026-11-19：要有最小和全屏窗口化）
    let (x, y) = (w - 24., 22.);
    c.line(x - 4.5, y - 4.5, x + 4.5, y + 4.5, 1.4, muted);
    c.line(x - 4.5, y + 4.5, x + 4.5, y - 4.5, 1.4, muted);
    let x = w - 56.;
    if maximized {
        c.rounded(x - 2.5, y - 4.5, 7.5, 7.5, 1.5, muted, true);
        c.rect(x - 4.5, y - 2.5, 7.5, 7.5, card);
        c.rounded(x - 4.5, y - 2.5, 7.5, 7.5, 1.5, muted, true);
    } else {
        c.rounded(x - 4.5, y - 4.5, 9., 9., 2., muted, true);
    }
    let x = w - 88.;
    c.line(x - 4.5, y, x + 4.5, y, 1.4, muted);
    // 右下复制图标：两张叠着的圆角纸；复制后变成对勾
    let (x, y) = (w - 34., h - 34.);
    if copied {
        c.line(x + 3., y + 10., x + 8., y + 15., 1.8, argb(p.ink));
        c.line(x + 8., y + 15., x + 17., y + 4., 1.8, argb(p.ink));
    } else {
        c.rounded(x + 6., y + 1., 12., 14., 3., muted, true);
        c.rounded(x + 2., y + 5., 12., 14., 3., card, false);
        c.rounded(x + 2., y + 5., 12., 14., 3., muted, true);
    }
    let _ = busy_phase;
}

/// 深灰胶囊按钮的底（翻译小窗的「翻译」），坐标是 96 DPI 的 x, y, 宽, 高
pub unsafe fn pill(dc: HDC, dpi: u32, x: i32, y: i32, w: i32, h: i32) {
    let Some(c) = Canvas::new(dc, dpi) else {
        return;
    };
    c.rounded(
        x as f32,
        y as f32,
        w as f32,
        h as f32,
        h as f32 / 2.,
        argb(palette().accent),
        false,
    );
}

/// 分隔线中间的方向胶囊：白底细灰边，中间画一对反向小箭头（⇄）。两边的字由调用方写。
/// (x, y, w, h) 是 96 DPI 坐标
pub unsafe fn swap_pill(dc: HDC, dpi: u32, x: f32, y: f32, w: f32, h: f32) {
    let Some(c) = Canvas::new(dc, dpi) else {
        return;
    };
    let p = palette();
    c.rounded(x, y, w, h, h / 2., argb(p.card), false);
    c.rounded(x, y, w, h, h / 2., argb(p.edge), true);
    let (cx, cy) = (x + w / 2., y + h / 2.);
    let color = argb(p.glyph);
    // 上面一支朝右、下面一支朝左
    c.line(cx - 6., cy - 3., cx + 6., cy - 3., 1.4, color);
    c.line(cx + 6., cy - 3., cx + 3., cy - 6., 1.4, color);
    c.line(cx - 6., cy + 3., cx + 6., cy + 3., 1.4, color);
    c.line(cx - 6., cy + 3., cx - 3., cy + 6., 1.4, color);
}

/// 细滚动条的滑块（参照 Husic：细、圆头、浅灰），坐标是 96 DPI
pub unsafe fn thumb(dc: HDC, dpi: u32, x: f32, y: f32, w: f32, h: f32) {
    let Some(c) = Canvas::new(dc, dpi) else {
        return;
    };
    c.rounded(x, y, w, h, w / 2., argb(palette().faint), false);
}

/// 翻译中三个跳动的小点，画在 (0,0) 起的一小条里（浅灰底）
pub unsafe fn busy_dots(dc: HDC, dpi: u32, width: i32, height: i32, phase: f32) {
    busy_dots_at(dc, dpi, 0, 0, width, height, phase);
}

/// 同上，画在 (x, y)（像素）起的那一条
pub unsafe fn busy_dots_at(dc: HDC, dpi: u32, x: i32, y: i32, width: i32, height: i32, phase: f32) {
    let Some(c) = Canvas::new(dc, dpi) else {
        return;
    };
    let ratio = dpi as f32 / 96.;
    gp::GdipTranslateWorldTransform(
        c.0,
        x as f32 / ratio,
        y as f32 / ratio,
        gp::MatrixOrderPrepend,
    );
    let w = width as f32 / ratio;
    let h = height as f32 / ratio;
    let p = palette();
    c.rect(0., 0., w, h, argb(p.card));
    for i in 0..3 {
        let lift = ((phase - i as f32 * 0.6).sin().max(0.)) * 3.;
        c.ellipse(
            4. + i as f32 * 11.,
            h / 2. - 1. - lift,
            6.,
            6.,
            argb(p.dots),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_color_setting_counts_as_a_theme_change() {
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        unsafe {
            assert!(is_color_change(wide("ImmersiveColorSet").as_ptr() as isize));
            assert!(!is_color_change(wide("Policy").as_ptr() as isize));
            assert!(!is_color_change(wide("Immersive").as_ptr() as isize));
            assert!(!is_color_change(
                wide("ImmersiveColorSetX").as_ptr() as isize
            ));
            assert!(!is_color_change(0));
        }
    }

    #[test]
    fn gdi_colors_swap_red_and_blue() {
        assert_eq!(colorref(0x112233), 0x00332211);
        assert_eq!(argb(0x112233), 0xff112233);
        // 浅色的页面灰、卡片白和原来写死的一样
        assert_eq!(colorref(LIGHT.page), 0x00f7f2f2);
        assert_eq!(colorref(LIGHT.card), 0x00ffffff);
    }
}
