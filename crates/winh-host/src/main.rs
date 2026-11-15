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
#[cfg(windows)]
mod panel;
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
    if args.next().as_deref() == Some("--panel-render") {
        let dir = std::path::PathBuf::from(args.next().unwrap_or_else(|| ".".into()));
        for (name, look, level) in [
            ("listening", panel::Look::Listening, 0.),
            ("speaking", panel::Look::Speaking, 0.7),
            ("idle", panel::Look::Idle, 0.),
        ] {
            if let Err(e) =
                panel::render_png(look, level, 3., &dir.join(format!("panel-{name}.png")))
            {
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
