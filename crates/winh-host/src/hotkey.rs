//! 快捷键模式（C11）：不经过 Win+H。按住热键录音，松开后把定稿文字键入到光标处。
//!
//! 只轮询自己关心的那几个键的状态（GetAsyncKeyState），不装全局键盘钩子，也不记录按键。
//! 键入用 SendInput 的 Unicode 方式，只发我们自己识别出的文字和语音指令对应的键。
use crate::{
    hotkey_state::{Action, HotkeyState},
    typing::{Key, Typist},
};
use std::{
    mem::size_of,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows_sys::Win32::UI::{
    Input::KeyboardAndMouse::{
        GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
        KEYEVENTF_UNICODE, SendInput, VK_BACK, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT,
        VK_LWIN, VK_MENU, VK_RCONTROL, VK_RETURN, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
    },
    WindowsAndMessaging::{GetClassNameW, GetForegroundWindow},
};
use xueer_ear_backend::{
    engine::{self, Event, Source},
    settings::Settings,
};
use xueer_ear_textflow::Output;

/// 按得比这短算误触
const MIN_HOLD: Duration = Duration::from_millis(300);

pub struct Hotkey {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Hotkey {
    /// 设置里关着就返回 None。
    pub fn start(settings: &Settings) -> Option<Self> {
        if !settings.hotkey_enabled {
            return None;
        }
        let vk = settings.hotkey_vk as i32;
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker = thread::spawn(move || poll_loop(vk, &worker_stop));
        println!("快捷键模式已开启：按住 {vk:#x} 说话，松开上屏");
        Some(Self {
            stop,
            worker: Some(worker),
        })
    }
}

impl Drop for Hotkey {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Session {
    release: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    worker: JoinHandle<()>,
}

fn poll_loop(vk: i32, stop: &AtomicBool) {
    let mut state = HotkeyState::new(MIN_HOLD);
    let mut session: Option<Session> = None;
    while !stop.load(Ordering::Acquire) {
        if session.as_ref().is_some_and(|s| s.worker.is_finished()) {
            let _ = session.take().unwrap().worker.join();
        }
        let held = pressed(vk);
        let other = other_modifier(vk);
        match state.poll(Instant::now(), held, other, session.is_some()) {
            Some(Action::Start) => session = Some(begin()),
            Some(Action::Finish) => {
                if let Some(s) = &session {
                    s.release.store(true, Ordering::Release);
                }
            }
            Some(Action::Cancel) => {
                if let Some(s) = &session {
                    s.cancel.store(true, Ordering::Release);
                }
            }
            None => {}
        }
        thread::sleep(Duration::from_millis(15));
    }
    if let Some(s) = session {
        s.cancel.store(true, Ordering::Release);
        let _ = s.worker.join();
    }
}

fn begin() -> Session {
    let release = Arc::new(AtomicBool::new(false));
    let cancel = Arc::new(AtomicBool::new(false));
    let (worker_release, worker_cancel) = (release.clone(), cancel.clone());
    let worker = thread::spawn(move || {
        if let Err(error) = record(worker_release, worker_cancel) {
            eprintln!("快捷键听写失败：{error}");
        }
    });
    Session {
        release,
        cancel,
        worker,
    }
}

/// 录一次音、识别、把定稿文字和指令键入前台窗口。
fn record(release: Arc<AtomicBool>, cancel: Arc<AtomicBool>) -> crate::install::Result<()> {
    let settings = Settings::load_default().unwrap_or_default();
    let post = crate::broker::postprocess();
    let terminal = foreground_is_terminal();
    let mut typist = Typist::default();
    let source = Source::MicrophoneHold {
        device: settings.microphone.clone(),
        release,
    };
    let stopped = cancel.clone();
    let mut typed_any = false;
    engine::run_wetype(source, cancel, |event| {
        if let Event::Speech(speech) = event {
            if speech.final_result && !stopped.load(Ordering::Acquire) {
                let keys = match post.finalize(&speech.text, crate::broker::now_ms()) {
                    Output::Text(text) => typist.text(&text, terminal),
                    Output::Command(command) => typist.command(command, terminal),
                };
                typed_any |= !keys.is_empty();
                send_keys(&keys);
            }
        }
        Ok(())
    })?;
    if !typed_any && !stopped.load(Ordering::Acquire) {
        crate::sound::play(crate::sound::Cue::Missed, settings.sounds_enabled);
    }
    Ok(())
}

fn pressed(vk: i32) -> bool {
    unsafe { GetAsyncKeyState(vk) < 0 }
}

/// 热键之外是否还按着修饰键（Ctrl/Alt/Shift/Win）。按着就当组合键，不算说话。
fn other_modifier(vk: i32) -> bool {
    [
        VK_LCONTROL,
        VK_RCONTROL,
        VK_LMENU,
        VK_RMENU,
        VK_LSHIFT,
        VK_RSHIFT,
        VK_LWIN,
        VK_RWIN,
    ]
    .into_iter()
    .map(i32::from)
    .filter(|&key| key != vk)
    .any(pressed)
}

fn foreground_is_terminal() -> bool {
    let mut class = [0_u16; 64];
    let count = unsafe {
        GetClassNameW(
            GetForegroundWindow(),
            class.as_mut_ptr(),
            class.len() as i32,
        )
    };
    count > 0
        && String::from_utf16_lossy(&class[..count as usize]) == "CASCADIA_HOSTING_WINDOW_CLASS"
}

fn key(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn tap(vk: u16) -> [INPUT; 2] {
    [key(vk, 0, 0), key(vk, 0, KEYEVENTF_KEYUP)]
}

fn send_keys(keys: &[Key]) {
    let mut inputs: Vec<INPUT> = Vec::new();
    // 热键可能是 Alt/Ctrl/Shift：先把它们抬起，免得和要键入的字凑成组合键
    for modifier in [VK_MENU, VK_CONTROL, VK_SHIFT] {
        if pressed(i32::from(modifier)) {
            inputs.push(key(modifier, 0, KEYEVENTF_KEYUP));
        }
    }
    for k in keys {
        match k {
            Key::Text(text) => {
                for unit in text.encode_utf16() {
                    inputs.push(key(0, unit, KEYEVENTF_UNICODE));
                    inputs.push(key(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
                }
            }
            Key::Enter => inputs.extend(tap(VK_RETURN)),
            Key::ShiftEnter => {
                inputs.push(key(VK_SHIFT, 0, 0));
                inputs.extend(tap(VK_RETURN));
                inputs.push(key(VK_SHIFT, 0, KEYEVENTF_KEYUP));
            }
            Key::CtrlZ => {
                inputs.push(key(VK_CONTROL, 0, 0));
                inputs.extend(tap(u16::from(b'Z')));
                inputs.push(key(VK_CONTROL, 0, KEYEVENTF_KEYUP));
            }
            Key::Backspace(count) => {
                for _ in 0..*count {
                    inputs.extend(tap(VK_BACK));
                }
            }
        }
    }
    if inputs.is_empty() {
        return;
    }
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    if sent as usize != inputs.len() {
        eprintln!(
            "键入未完成：{sent}/{}（目标窗口可能是管理员权限）",
            inputs.len()
        );
    }
}
