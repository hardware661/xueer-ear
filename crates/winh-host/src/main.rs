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
    // 只画面板不听写：截图核对外观用（`--panel-preview listening|idle`）
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("--panel-preview") {
        let look = match args.next().as_deref() {
            Some("idle") => panel::Look::Idle,
            _ => panel::Look::Listening,
        };
        let (tx, _rx) = std::sync::mpsc::channel();
        let handle = panel::spawn(tx);
        std::thread::sleep(std::time::Duration::from_millis(300));
        handle.set(look);
        for i in 0..80 {
            handle.level(((i as f32 * 0.4).sin().abs() * 9000.) as u32);
            std::thread::sleep(std::time::Duration::from_millis(50));
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
