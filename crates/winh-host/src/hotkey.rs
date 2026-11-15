//! 快捷键听写（默认右 Alt）：雪耳自己的面板 + 边说边出字，不经过微软的 Win+H。
//!
//! 交互见 `dictation.rs`：点一下打开面板开始听，停顿收句后麦克风变灰，再点接着听；
//! 按住说、松开收句也行；点 ×、Esc 或灰着一会儿就收起。
//! 只轮询自己关心的几个键（GetAsyncKeyState），不装全局键盘钩子、不记录按键。
//! 键入用 SendInput 的 Unicode 方式，只发我们识别出的文字和语音指令对应的键。
use crate::{
    dictation::{Dictation, Effect, Input, Panel},
    panel::{self, Click, Look, PanelHandle},
    typing::{Key, Live},
};
use std::{
    mem::size_of,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows_sys::Win32::UI::{
    Input::KeyboardAndMouse::{
        GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
        KEYEVENTF_UNICODE, SendInput, VK_BACK, VK_CONTROL, VK_ESCAPE, VK_LCONTROL, VK_LMENU,
        VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RETURN, VK_RMENU, VK_RSHIFT, VK_RWIN,
        VK_SHIFT,
    },
    WindowsAndMessaging::{GetClassNameW, GetForegroundWindow},
};
use xueer_ear_backend::{
    engine::{self, Event, Source},
    settings::Settings,
};

/// 按住超过这么久再松开，算「按住说话」，松开就收句；更短的算「点一下」
const HOLD_THRESHOLD: Duration = Duration::from_millis(400);
/// 麦克风灰着这么久没动静，面板自动收起
const IDLE_HIDE: Duration = Duration::from_secs(10);

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
        println!("快捷键听写已开启：点一下 {vk:#x} 打开面板说话");
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

impl Session {
    fn finish(&self) {
        self.release.store(true, Ordering::Release);
    }
}

fn poll_loop(vk: i32, stop: &AtomicBool) {
    let (clicks_tx, clicks) = mpsc::channel();
    let panel = panel::spawn(clicks_tx);
    let mut state = Dictation::new(HOLD_THRESHOLD, IDLE_HIDE);
    let mut session: Option<Session> = None;
    let mut was_held = false;
    let mut was_escape = false;
    while !stop.load(Ordering::Acquire) {
        let now = Instant::now();
        let mut inputs = Vec::new();
        if session.as_ref().is_some_and(|s| s.worker.is_finished()) {
            let _ = session.take().unwrap().worker.join();
            inputs.push(Input::SessionEnded);
        }
        let held = pressed(vk);
        if held && !was_held {
            inputs.push(if other_modifier(vk) {
                Input::Chord
            } else {
                Input::KeyDown
            });
        } else if !held && was_held {
            inputs.push(Input::KeyUp);
        }
        was_held = held;
        let escape = pressed(i32::from(VK_ESCAPE));
        if escape && !was_escape && state.panel() != Panel::Hidden {
            inputs.push(Input::Close);
        }
        was_escape = escape;
        while let Ok(click) = clicks.try_recv() {
            match click {
                Click::Mic => inputs.push(Input::MicClick),
                Click::Close => inputs.push(Input::Close),
                Click::Settings => open_control_panel(),
            }
        }
        let mut effects = Vec::new();
        for input in inputs {
            effects.extend(state.input(now, input));
        }
        effects.extend(state.tick(now));
        for effect in effects {
            match effect {
                Effect::Show => {}
                Effect::Hide => panel.set(Look::Hidden),
                Effect::Start => {
                    // 上一轮还没收尾就先取消它，避免两轮抢麦克风
                    if let Some(old) = session.take() {
                        old.cancel.store(true, Ordering::Release);
                        let _ = old.worker.join();
                    }
                    panel.set(Look::Connecting);
                    session = Some(begin(panel.clone()));
                }
                Effect::Finish => {
                    if let Some(s) = &session {
                        s.finish();
                    }
                }
            }
        }
        match state.panel() {
            Panel::Idle => panel.set_if_changed(Look::Idle),
            Panel::Hidden => panel.set_if_changed(Look::Hidden),
            Panel::Listening => {}
        }
        thread::sleep(Duration::from_millis(15));
    }
    if let Some(s) = session {
        s.cancel.store(true, Ordering::Release);
        let _ = s.worker.join();
    }
    panel.set(Look::Hidden);
}

fn begin(panel: PanelHandle) -> Session {
    let release = Arc::new(AtomicBool::new(false));
    let cancel = Arc::new(AtomicBool::new(false));
    let (worker_release, worker_cancel) = (release.clone(), cancel.clone());
    let worker = thread::spawn(move || {
        if let Err(error) = record(worker_release, worker_cancel, &panel) {
            eprintln!("快捷键听写失败：{error}");
        }
    });
    Session {
        release,
        cancel,
        worker,
    }
}

/// 录一轮音：中间结果边说边打到光标处，每句定稿换成整理后的文字或执行指令。
/// 停顿只收句不结束；主人再点一下 / 点麦克风 / Esc，或到「最长听写时间」才结束（面板变灰）。
fn record(
    release: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    panel: &PanelHandle,
) -> crate::install::Result<()> {
    let settings = Settings::load_default().unwrap_or_default();
    let post = crate::broker::postprocess();
    let mut live = Live::new(foreground_is_terminal());
    let source = Source::MicrophoneHold {
        device: settings.microphone.clone(),
        release: release.clone(),
    };
    let stopped = cancel.clone();
    let mut typed_any = false;
    let started = Instant::now();
    let max_listen = Duration::from_secs(u64::from(settings.max_listen_minutes) * 60);
    engine::run_wetype(source, cancel, |event| {
        if stopped.load(Ordering::Acquire) {
            return Ok(());
        }
        match event {
            // 停顿只是一句说完（定稿上屏），不结束：一直听到主人再点一下、点麦克风、Esc，
            // 或者超过「最长听写时间」（主人 2026-11-15：说着说着停顿一下就断了，要能一直说）
            Event::Activity("armed")
            | Event::Activity("recognizing")
            | Event::Activity("paused") => panel.set(Look::Listening),
            Event::Activity("connecting") => panel.set(Look::Connecting),
            Event::Progress { peak, .. } => {
                panel.level(peak);
                if started.elapsed() >= max_listen {
                    release.store(true, Ordering::Release);
                }
            }
            Event::Speech(speech) => {
                let keys = if speech.final_result {
                    live.finalize(post.finalize(&speech.text, crate::broker::now_ms()))
                } else {
                    live.partial(&speech.text)
                };
                typed_any |= keys.iter().any(|k| matches!(k, Key::Text(_)));
                send_keys(&keys);
            }
            _ => {}
        }
        Ok(())
    })?;
    if !typed_any && !stopped.load(Ordering::Acquire) {
        crate::sound::play(crate::sound::Cue::Missed, settings.sounds_enabled);
    }
    Ok(())
}

/// 齿轮：打开雪耳控制面板
fn open_control_panel() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe.with_file_name("xueer-ear.exe")).spawn();
    }
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
