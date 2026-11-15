//! 快捷键听写（默认右 Alt）：雪耳自己的面板 + 边说边出字，不经过微软的 Win+H。
//!
//! 交互见 `dictation.rs`：点一下打开面板开始听，停顿收句后麦克风变灰，再点接着听；
//! 按住说、松开收句也行；Esc 或灰着一会儿就收起。
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

/// 正在往前台发我们自己的按键（里面可能有 Enter），轮询线程这时不把 Enter 算作主人按的
static INJECTING: AtomicBool = AtomicBool::new(false);

/// 按住超过这么久再松开，算「按住说话」，松开就收句；更短的算「点一下」
const HOLD_THRESHOLD: Duration = Duration::from_millis(400);
/// 一直没人说话这么久，就自己停止听写（麦克风变灰）
const QUIET_STOP: Duration = Duration::from_secs(60);
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
    /// 主人自己按 Enter 的次数（轮询线程加，听写线程读），用来去掉已发走的文字
    sent: Arc<std::sync::atomic::AtomicU32>,
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
    let mut was_enter = false;
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
        // 主人自己按 Enter（我们注入 Enter 时会先置 INJECTING，不算）
        let enter = pressed(i32::from(VK_RETURN));
        if enter && !was_enter && !INJECTING.load(Ordering::Acquire) {
            if let Some(s) = &session {
                s.sent.fetch_add(1, Ordering::AcqRel);
            }
        }
        was_enter = enter;
        let escape = pressed(i32::from(VK_ESCAPE));
        if escape && !was_escape && state.panel() != Panel::Hidden {
            inputs.push(Input::Close);
        }
        was_escape = escape;
        while let Ok(click) = clicks.try_recv() {
            match click {
                Click::Mic => inputs.push(Input::MicClick),
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
    let sent = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let (worker_release, worker_cancel, worker_sent) =
        (release.clone(), cancel.clone(), sent.clone());
    let worker = thread::spawn(move || {
        if let Err(error) = record(worker_release, worker_cancel, worker_sent, &panel) {
            eprintln!("快捷键听写失败：{error}");
        }
    });
    Session {
        release,
        cancel,
        sent,
        worker,
    }
}

/// 录一轮音：中间结果边说边打到光标处，每句定稿换成整理后的文字或执行指令。
/// 停顿只收句不结束；主人再点一下 / 点麦克风 / Esc，或到「最长听写时间」才结束（面板变灰）。
fn record(
    release: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    sent: Arc<std::sync::atomic::AtomicU32>,
    panel: &PanelHandle,
) -> crate::install::Result<()> {
    let mut seen_sent = 0;
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
    let mut last_voice = Instant::now();
    let max_listen = Duration::from_secs(u64::from(settings.max_listen_minutes) * 60);
    engine::run_wetype(source, cancel, |event| {
        if stopped.load(Ordering::Acquire) {
            return Ok(());
        }
        match event {
            // 停顿只是一句说完（定稿上屏），不结束：一直听到主人再点一下、点麦克风、Esc，
            // 或者超过「最长听写时间」（主人 2026-11-15：说着说着停顿一下就断了，要能一直说）
            // 蓝麦克风=在听；听到说话（recognizing）就换成波浪；一句说完（paused）回到蓝麦克风
            Event::Activity("armed") | Event::Activity("paused") => {
                last_voice = Instant::now();
                panel.set(Look::Listening)
            }
            Event::Activity("recognizing") => {
                last_voice = Instant::now();
                panel.set(Look::Speaking)
            }
            Event::Activity("connecting") => panel.set(Look::Connecting),
            Event::Progress { peak, .. } => {
                panel.level(peak);
                // 到「最长听写时间」，或者一分钟没人说话（主人："本次对话已经结束…怎么样才能让这个弹窗消失"），
                // 就自己停：麦克风变灰，再过 10 秒面板收起
                if started.elapsed() >= max_listen || last_voice.elapsed() >= QUIET_STOP {
                    release.store(true, Ordering::Release);
                }
            }
            Event::Speech(speech) => {
                // 主人在这句说到一半时按了 Enter：屏幕上的部分已经发走，后面只打剩下的
                let now_sent = sent.load(Ordering::Acquire);
                if now_sent != seen_sent {
                    seen_sent = now_sent;
                    live.user_sent();
                }
                let text = live.unsent(&speech.text);
                let keys = if speech.final_result {
                    live.finalize(post.finalize(&text, crate::broker::now_ms()))
                } else {
                    live.partial(&text)
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

/// 文字和退格直接给前台窗口里的焦点控件发 WM_CHAR（和终端路径一样）。
/// SendInput 的 Unicode 方式在 Qt 程序里（微信就是 Qt 5.15）会把每批第一个字认成上一批的
/// 最后一个字（Qt 把连续的 VK_PACKET 当成同一个键的自动重复，沿用旧字）——2026-11-15 主人在
/// 微信里看到「。天天气不错」。发不过去（比如管理员窗口）就返回 false，退回 SendInput。
fn send_chars(units: &[u16]) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GUITHREADINFO, GetGUIThreadInfo, GetWindowThreadProcessId, SMTO_ABORTIFHUNG, SMTO_BLOCK,
        SendMessageTimeoutW, WM_CHAR,
    };
    unsafe {
        let window = GetForegroundWindow();
        if window.is_null() {
            return false;
        }
        let thread = GetWindowThreadProcessId(window, std::ptr::null_mut());
        let mut info: GUITHREADINFO = std::mem::zeroed();
        info.cbSize = size_of::<GUITHREADINFO>() as u32;
        if GetGUIThreadInfo(thread, &mut info) == 0 || info.hwndFocus.is_null() {
            return false;
        }
        for &unit in units {
            let mut result = 0;
            if SendMessageTimeoutW(
                info.hwndFocus,
                WM_CHAR,
                usize::from(unit),
                1,
                SMTO_BLOCK | SMTO_ABORTIFHUNG,
                250,
                &mut result,
            ) == 0
            {
                return false;
            }
        }
        true
    }
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
        // 文字和退格优先走 WM_CHAR；前面攒着的按键先发出去，保证顺序
        let chars: Option<Vec<u16>> = match k {
            Key::Text(text) => Some(text.encode_utf16().collect()),
            Key::Backspace(count) => Some(vec![8; *count]),
            _ => None,
        };
        if let Some(units) = chars {
            flush(&mut inputs);
            if send_chars(&units) {
                continue;
            }
        }
        match k {
            Key::Text(text) => {
                for unit in text.encode_utf16() {
                    inputs.push(key(0, unit, KEYEVENTF_UNICODE));
                    inputs.push(key(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
                }
            }
            Key::Enter => inputs.extend(tap(VK_RETURN)),
            Key::CtrlJ => {
                inputs.push(key(VK_CONTROL, 0, 0));
                inputs.extend(tap(u16::from(b'J')));
                inputs.push(key(VK_CONTROL, 0, KEYEVENTF_KEYUP));
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
    flush(&mut inputs);
}

/// 把攒着的 SendInput 按键一次发出去并清空。里面有 Enter 时先立 INJECTING，免得当成主人按的。
fn flush(inputs: &mut Vec<INPUT>) {
    if inputs.is_empty() {
        return;
    }
    let has_enter = inputs
        .iter()
        .any(|i| unsafe { i.Anonymous.ki.wVk } == VK_RETURN);
    if has_enter {
        INJECTING.store(true, Ordering::Release);
    }
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    if has_enter {
        // 轮询每 15 ms 看一次键，等它看过我们自己发的 Enter 再清标记
        thread::sleep(Duration::from_millis(60));
        INJECTING.store(false, Ordering::Release);
    }
    if sent as usize != inputs.len() {
        eprintln!(
            "键入未完成：{sent}/{}（目标窗口可能是管理员权限）",
            inputs.len()
        );
    }
    inputs.clear();
}
