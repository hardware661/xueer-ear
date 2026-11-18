//! Antialiased Fluent decoration; native child controls retain input and focus.
#![allow(unsafe_op_in_unsafe_fn)]
use std::{mem, ptr};
use windows_sys::Win32::{
    Foundation::RECT,
    Graphics::{Gdi::HDC, GdiPlus as gp},
};

pub const BACKGROUND: u32 = 0x00fffcfa;
pub const WIDTH: i32 = 400;
pub const HEADER: i32 = 42;
pub const HEIGHT: i32 = 646;
pub fn body_y(y: i32) -> i32 {
    y
}
// 主色：深灰（雪耳黑白灰，原来是绿色）
const GREEN: u32 = 0xff2c2c2e;
const LINE: u32 = 0xffe5eaf0;
const MUTED: u32 = 0xff657388;

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

pub unsafe fn background(dc: HDC, dpi: u32, enabled: bool, peak: u32, frame: u64) {
    let Some(c) = Canvas::new(dc, dpi) else {
        return;
    };
    let accent = if enabled { GREEN } else { 0xff8190a2 };
    c.ellipse(
        150.,
        58.,
        100.,
        100.,
        if enabled { 0xffe8e8ed } else { 0xfff0f3f7 },
    );
    c.ellipse(162., 70., 76., 76., accent);
    c.rounded(193., 86., 14., 28., 7., 0xffffffff, false);
    c.arc(186., 96., 28., 28., 0., 180., 2.5, 0xffffffff);
    c.line(200., 124., 200., 134., 2.5, 0xffffffff);
    c.line(192., 134., 208., 134., 2.5, 0xffffffff);
    for i in 0..28 {
        let activity = (peak as f32 / 8000.).clamp(0., 1.);
        let shape = ((i as f32 * 1.13 - frame as f32 * 0.32).sin().abs() * 0.75 + 0.25)
            * (1. - (i as f32 - 14.).abs() / 22.);
        let h = if peak > 0 {
            5. + 16. * activity * shape
        } else {
            4. + 2. * ((i as f32 * 1.13).sin().abs())
        };
        c.rounded(
            90.5 + i as f32 * 8.,
            174. - h / 2.,
            3.,
            h,
            1.5,
            if peak > 0 && enabled {
                accent
            } else {
                0xffcbd4df
            },
            false,
        );
    }
    for y in [230., 288., 346., 404., 462., 520., 578.] {
        c.line(20., y, 380., y, 1., LINE);
    }
    // 每行左边的小图标：服务、快捷键、麦克风、静音收句、热词、历史、更多设置
    let gear = (0..48)
        .map(|i| {
            let a = i as f32 * std::f32::consts::TAU / 48.;
            let r = if matches!(i % 6, 1..=3) { 9. } else { 7. };
            (30. + a.cos() * r, 259. + a.sin() * r)
        })
        .collect::<Vec<_>>();
    c.polygon(&gear, MUTED, true);
    c.arc(27., 256., 6., 6., 0., 360., 1.5, MUTED);
    // 快捷键：一块小键盘
    c.rounded(20., 310., 20., 14., 2.5, MUTED, true);
    for row in 0..2 {
        for col in 0..5 {
            c.ellipse(
                23. + col as f32 * 3.5,
                313. + row as f32 * 3.5,
                1.3,
                1.3,
                MUTED,
            );
        }
    }
    c.line(25., 321., 35., 321., 1.3, MUTED);
    // 麦克风
    c.rounded(27., 365., 6., 11., 3., MUTED, true);
    c.arc(24., 369., 12., 12., 0., 180., 1.5, MUTED);
    c.line(30., 381., 30., 385., 1.5, MUTED);
    c.line(27., 385., 33., 385., 1.5, MUTED);
    // 静音收句：钟
    c.arc(21., 424., 18., 18., 0., 360., 1.5, MUTED);
    c.line(30., 427., 30., 433., 1.5, MUTED);
    c.line(30., 433., 34., 435., 1.5, MUTED);
    // 热词：书签
    c.polygon(
        &[
            (24., 482.),
            (36., 482.),
            (36., 500.),
            (30., 495.),
            (24., 500.),
        ],
        MUTED,
        true,
    );
    // 历史：三行字
    for (i, w) in [18., 12., 15.].iter().enumerate() {
        c.line(
            21.,
            544. + i as f32 * 5.5,
            21. + w,
            544. + i as f32 * 5.5,
            1.5,
            MUTED,
        );
    }
    // 更多设置：两条滑杆
    for (y, knob) in [(603., 26.), (611., 34.)] {
        c.line(21., y, 39., y, 1.5, MUTED);
        c.ellipse(knob - 2.5, y - 2.5, 5., 5., MUTED);
    }
}

pub unsafe fn value_button(dc: HDC, dpi: u32, width: i32, height: i32, pressed: bool) {
    let Some(c) = Canvas::new(dc, dpi) else {
        return;
    };
    let w = width as f32 * 96. / dpi as f32;
    let h = height as f32 * 96. / dpi as f32;
    c.rounded(
        0.5,
        0.5,
        w - 1.,
        h - 1.,
        6.,
        if pressed { 0xffedf3f8 } else { 0xfffcfdff },
        false,
    );
    c.rounded(0.5, 0.5, w - 1., h - 1., 6., 0xffdbe3ec, true);
    c.line(w - 23., h / 2. - 2., w - 19., h / 2. + 2., 1.5, MUTED);
    c.line(w - 19., h / 2. + 2., w - 15., h / 2. - 2., 1.5, MUTED);
}

pub unsafe fn menu_row(dc: HDC, dpi: u32, rect: &RECT, hover: bool, checked: bool) {
    let Some(c) = Canvas::new(dc, dpi) else {
        return;
    };
    let ratio = dpi as f32 / 96.;
    let x = rect.left as f32 / ratio;
    let y = rect.top as f32 / ratio;
    let w = (rect.right - rect.left) as f32 / ratio;
    let h = (rect.bottom - rect.top) as f32 / ratio;
    if hover || checked {
        c.rounded(
            x + 3.,
            y + 2.,
            w - 6.,
            h - 4.,
            5.,
            if hover { 0xffedf3f6 } else { 0xfff2f2f7 },
            false,
        );
    }
    if checked {
        c.line(x + 13., y + h / 2., x + 16., y + h / 2. + 3., 1.5, GREEN);
        c.line(
            x + 16.,
            y + h / 2. + 3.,
            x + 22.,
            y + h / 2. - 4.,
            1.5,
            GREEN,
        );
    }
}

pub unsafe fn update_bubble(dc: HDC, dpi: u32, available: bool) {
    let Some(c) = Canvas::new(dc, dpi) else {
        return;
    };
    let h = if available { 126. } else { 78. };
    c.rounded(14., 95., 268., h, 10., 0x180f172a, false);
    c.rounded(12., 92., 268., h, 10., 0xfffafcff, false);
    c.rounded(12., 92., 268., h, 10., 0xffdbe3ec, true);
    c.polygon(&[(24., 93.), (30., 86.), (36., 93.)], 0xfffafcff, false);
}

pub unsafe fn update_badge(dc: HDC, dpi: u32) {
    let Some(c) = Canvas::new(dc, dpi) else {
        return;
    };
    c.ellipse(23., 3., 8., 8., 0xfff2f7fc);
    c.ellipse(24., 4., 6., 6., GREEN);
}

pub unsafe fn close_glyph(dc: HDC, dpi: u32, width: i32, height: i32) {
    let Some(c) = Canvas::new(dc, dpi) else {
        return;
    };
    let x = width as f32 * 48. / dpi as f32;
    let y = height as f32 * 48. / dpi as f32;
    c.line(x - 3., y - 3., x + 3., y + 3., 1.3, MUTED);
    c.line(x - 3., y + 3., x + 3., y - 3., 1.3, MUTED);
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
    if toggle {
        let fill = if disabled {
            0xffd5dce5
        } else if selected {
            GREEN
        } else {
            0xffbbc5d2
        };
        c.rounded(1., (h - 26.) / 2., w - 2., 26., 13., fill, false);
        c.ellipse(
            if selected { w - 24. } else { 4. },
            (h - 20.) / 2.,
            20.,
            20.,
            0xffffffff,
        );
    } else {
        let fill = if pressed {
            0xffdcdce1
        } else if selected {
            0xffe8e8ed
        } else {
            0xfff0f3f7
        };
        c.rounded(0.5, 0.5, w - 1., h - 1., 6., fill, false);
        c.rounded(
            0.5,
            0.5,
            w - 1.,
            h - 1.,
            6.,
            if selected { 0xffaeaeb2 } else { LINE },
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
    // 整张白底；分隔线（主人 2026-11-19：加回来）上下各留距离，中间空出放「中 ⇄ 英」的小胶囊。
    // 译文那半不再整块铺灰，只靠线和留白分开，胶囊不会骑在两种底色的接缝上
    c.rect(0., 0., w, h, 0xffffffff);
    let gap = 60.;
    c.line(18., split, w / 2. - gap, split, 1., 0xffd1d1d6);
    c.line(w / 2. + gap, split, w - 18., split, 1., 0xffd1d1d6);
    // 右上三个小按钮：最小化、最大化/还原、关闭（主人 2026-11-19：要有最小和全屏窗口化）
    let (x, y) = (w - 24., 22.);
    c.line(x - 4.5, y - 4.5, x + 4.5, y + 4.5, 1.4, 0xff8e8e93);
    c.line(x - 4.5, y + 4.5, x + 4.5, y - 4.5, 1.4, 0xff8e8e93);
    let x = w - 56.;
    if maximized {
        c.rounded(x - 2.5, y - 4.5, 7.5, 7.5, 1.5, 0xff8e8e93, true);
        c.rect(x - 4.5, y - 2.5, 7.5, 7.5, 0xffffffff);
        c.rounded(x - 4.5, y - 2.5, 7.5, 7.5, 1.5, 0xff8e8e93, true);
    } else {
        c.rounded(x - 4.5, y - 4.5, 9., 9., 2., 0xff8e8e93, true);
    }
    let x = w - 88.;
    c.line(x - 4.5, y, x + 4.5, y, 1.4, 0xff8e8e93);
    // 右下复制图标：两张叠着的圆角纸；复制后变成对勾
    let (x, y) = (w - 34., h - 34.);
    if copied {
        c.line(x + 3., y + 10., x + 8., y + 15., 1.8, 0xff2c2c2e);
        c.line(x + 8., y + 15., x + 17., y + 4., 1.8, 0xff2c2c2e);
    } else {
        c.rounded(x + 6., y + 1., 12., 14., 3., 0xff8e8e93, true);
        c.rounded(x + 2., y + 5., 12., 14., 3., 0xffffffff, false);
        c.rounded(x + 2., y + 5., 12., 14., 3., 0xff8e8e93, true);
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
        GREEN,
        false,
    );
}

/// 分隔线中间的方向胶囊：白底细灰边，中间画一对反向小箭头（⇄）。两边的字由调用方写。
/// (x, y, w, h) 是 96 DPI 坐标
pub unsafe fn swap_pill(dc: HDC, dpi: u32, x: f32, y: f32, w: f32, h: f32) {
    let Some(c) = Canvas::new(dc, dpi) else {
        return;
    };
    c.rounded(x, y, w, h, h / 2., 0xffffffff, false);
    c.rounded(x, y, w, h, h / 2., 0xffd1d1d6, true);
    let (cx, cy) = (x + w / 2., y + h / 2.);
    let color = 0xff636366;
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
    c.rounded(x, y, w, h, w / 2., 0xffc7c7cc, false);
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
    c.rect(0., 0., w, h, 0xffffffff);
    for i in 0..3 {
        let lift = ((phase - i as f32 * 0.6).sin().max(0.)) * 3.;
        c.ellipse(4. + i as f32 * 11., h / 2. - 1. - lift, 6., 6., 0xffaeaeb2);
    }
}
