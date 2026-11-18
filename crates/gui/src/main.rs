#![cfg_attr(windows, windows_subsystem = "windows")]
#[cfg(windows)]
mod app;
#[cfg(windows)]
mod hotword_dialog;
#[cfg(windows)]
mod menu;
mod model;
#[cfg(any(windows, test))]
mod prefs;
#[cfg(windows)]
mod translate_window;
mod update;
#[cfg(windows)]
mod view;

fn main() {
    #[cfg(windows)]
    {
        let mut args = std::env::args_os().skip(1);
        let outcome = match args.next() {
            Some(arg) if arg == "--quit" => app::quit(),
            // 听写面板右边的「译」按钮：开翻译小窗
            Some(arg) if arg == "--translate" => translate_window::run(),
            // 离屏拍照核对外观：`--snapshot 目录`
            Some(arg) if arg == "--snapshot" => app::snapshot(std::path::Path::new(
                &args.next().unwrap_or_else(|| ".".into()),
            )),
            _ => {
                app::run();
                Ok(())
            }
        };
        if let Err(error) = outcome {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
    #[cfg(not(windows))]
    eprintln!("请在 Windows 运行。");
}
