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
    // 离屏把面板画成 PNG 核对外观（不在屏幕上弹窗）：`--panel-render 目录`
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
    if first.as_deref() == Some("--panel-render") {
        let dir = std::path::PathBuf::from(args.next().unwrap_or_else(|| ".".into()));
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
                    3.,
                    text,
                    &dir.join(format!("panel-{name}{suffix}.png")),
                    panel::EAR,
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
