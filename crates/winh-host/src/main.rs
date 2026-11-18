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
    // 挑耳朵：`--ear-sheet 目录`，按几组参数各画一张「在听」的面板
    if first.as_deref() == Some("--ear-sheet") {
        let dir = std::path::PathBuf::from(args.next().unwrap_or_else(|| ".".into()));
        for (name, ear) in panel::EAR_CHOICES {
            let _ = panel::render_png(
                panel::Look::Listening,
                0.,
                3.,
                "",
                &dir.join(format!("ear-{name}.png")),
                ear,
            );
        }
        return;
    }
    if first.as_deref() == Some("--panel-render") {
        let dir = std::path::PathBuf::from(args.next().unwrap_or_else(|| ".".into()));
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
                &dir.join(format!("panel-{name}.png")),
                panel::EAR,
            ) {
                eprintln!("{e}");
                std::process::exit(1);
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
