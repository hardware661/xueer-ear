#[cfg(windows)]
mod broker;
#[cfg(windows)]
mod control;
#[cfg(any(windows, test))]
mod dictation;
#[cfg(windows)]
mod hotkey;
#[cfg(any(windows, test))]
mod input_guard;
#[cfg(windows)]
mod install;
#[cfg(any(windows, test))]
mod logfile;
#[cfg(any(windows, test))]
mod look;
#[cfg(windows)]
mod panel;
#[cfg(any(windows, test))]
mod recovery;
#[cfg(any(windows, test))]
mod reload;
#[cfg(any(windows, test))]
mod revision;
#[cfg(windows)]
mod sound;
#[cfg(windows)]
mod status;
#[cfg(windows)]
mod terminal;
#[cfg(any(windows, test))]
mod typing;

#[cfg(windows)]
fn main() {
    // 离屏把面板画成 PNG 核对外观（不在屏幕上弹窗）：`--panel-render 目录 [clear]`，clear 是透明底
    let mut args = std::env::args().skip(1);
    let first = args.next();
    // 排查漏字：`--type-probe 文字 [分几次] [等几毫秒]`，把文字按听写的方式打进前台窗口
    if first.as_deref() == Some("--type-probe") {
        let text = args.next().unwrap_or_default();
        let pieces = args.next().and_then(|v| v.parse().ok()).unwrap_or(8);
        let delay = args.next().and_then(|v| v.parse().ok()).unwrap_or(1500);
        hotkey::type_probe(&text, pieces, delay);
        return;
    }
    // 排查：`--wave-probe [等几毫秒]`，配合环境变量 XUEER_EAR_TEST_WAVE，用录音跑一整轮快捷键听写
    if first.as_deref() == Some("--wave-probe") {
        let delay = args.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        hotkey::wave_probe(delay);
        return;
    }
    // README 动图：`--panel-frames 目录`，「说话中」的面板按真实节奏（33 毫秒一帧，和面板的定时器一样）
    // 画一圈能首尾接上的帧，浅色、深色各一套，透明底
    if first.as_deref() == Some("--panel-frames") {
        let dir = std::path::PathBuf::from(args.next().unwrap_or_else(|| ".".into()));
        // 波浪的起伏周期是 2π/1.7：画 82 帧正好转 4 圈，每帧约 0.18（面板每帧也是 0.18）
        const FRAMES: usize = 82;
        let step = std::f32::consts::TAU * 4. / 1.7 / FRAMES as f32;
        for (dark, suffix) in [(false, ""), (true, "-dark")] {
            panel::set_dark(dark);
            let mut smooth = 0.0_f32;
            // 先空转一圈，让平滑后的音量也首尾接上
            for pass in 0..2 {
                for i in 0..FRAMES {
                    let t = i as f32 / FRAMES as f32;
                    // 像说话的音量：一圈里三个起伏的字句，中间各有一点停顿
                    let speech = (t * std::f32::consts::PI * 3.).sin().abs();
                    let target = 0.15
                        + 0.7 * speech * (0.75 + 0.25 * (t * std::f32::consts::TAU * 2.).sin());
                    smooth += (target - smooth) * 0.35;
                    if pass == 0 {
                        continue;
                    }
                    let out = dir.join(format!("frame{suffix}-{i:03}.png"));
                    let result = panel::render_png(
                        panel::Look::Speaking,
                        smooth,
                        i as f32 * step,
                        2.,
                        "",
                        &out,
                        panel::EAR,
                        true,
                    );
                    if let Err(e) = result {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }
                }
            }
        }
        return;
    }
    if first.as_deref() == Some("--panel-render") {
        let dir = std::path::PathBuf::from(args.next().unwrap_or_else(|| ".".into()));
        let clear = args.next().as_deref() == Some("clear");
        // 浅色、深色各一套：panel-listening.png、panel-listening-dark.png
        for (dark, suffix) in [(false, ""), (true, "-dark")] {
            panel::set_dark(dark);
            for (name, look, level) in [
                ("listening", panel::Look::Listening, 0.),
                ("speaking", panel::Look::Speaking, 0.7),
                ("idle", panel::Look::Idle, 0.),
                ("notice", panel::Look::Notice, 0.),
            ] {
                let text = recovery::message(recovery::Kind::Network);
                if let Err(e) = panel::render_png(
                    look,
                    level,
                    0.,
                    3.,
                    text,
                    &dir.join(format!("panel-{name}{suffix}.png")),
                    panel::EAR,
                    clear,
                ) {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        return;
    }
    let result =
        control::command().and_then(|handled| if handled { Ok(()) } else { broker::run() });
    if let Err(error) = result {
        status::startup_error(&error.to_string());
        eprintln!("错误：{error}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("请在 Windows 运行。");
}
