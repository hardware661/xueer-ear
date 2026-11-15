//! 快捷键听写（默认右 Alt）：雪耳自己的面板 + 边说边出字，不经过微软的 Win+H。
//!
//! 交互见 `dictation.rs`：点一下打开面板开始听，停顿收句后麦克风变灰，再点接着听；
//! 按住说、松开收句也行；Esc 或灰着一会儿就收起。
//! 只轮询自己关心的几个键（GetAsyncKeyState），不装全局键盘钩子、不记录按键。
//! 键入用 SendInput 的 Unicode 方式，只发我们识别出的文字和语音指令对应的键。
use crate::{
    dictation::{Dictation, Effect, Input, Panel},
    panel::{self, Click, Look, PanelHandle},
    recovery,
    reload::KeyWatch,
    status::Status,
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
/// 这么久没识别出新字，就自己结束听写并收起面板（主人 2026-11-15：「10秒钟就比较合适」）
const QUIET_STOP: Duration = Duration::from_secs(10);
/// 识别出新字后，波浪保持这么久再变回麦克风
const WAVE_HOLD: Duration = Duration::from_millis(900);
/// 单独说一个指令词（删除、撤回、发送……）后停这么久没再出新字，就马上执行，不等静音收句。
/// 主人 2026-11-16：「撤回和删除的速度…不是特别快，还有发送的速度」——原来要等 3 秒静音
const COMMAND_SETTLE: Duration = Duration::from_millis(600);

pub struct Hotkey {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Hotkey {
    /// 设置里关着就返回 None。
    /// `status` 是给控制面板看的状态文件：听写时的阶段、出错原因都写进去。
    pub fn start(settings: &Settings, status: Status) -> Option<Self> {
        if !settings.hotkey_enabled {
            return None;
        }
        let vk = settings.hotkey_vk;
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker = thread::spawn(move || poll_loop(vk, &worker_stop, &status));
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
    /// 主人动了键盘或鼠标的次数（点别处、打字）：听写线程看到变化就不再往回改字
    touched: Arc<std::sync::atomic::AtomicU32>,
    worker: JoinHandle<()>,
}

impl Session {
    fn finish(&self) {
        self.release.store(true, Ordering::Release);
    }
}

fn poll_loop(vk: u32, stop: &AtomicBool, status: &Status) {
    println!("快捷键听写已开启：点一下 {vk:#x} 打开面板说话");
    // 控制面板里换了快捷键，这里每秒看一眼设置文件，变了就换，不用重启服务
    let mut watch = KeyWatch::new(vk, Settings::modified_default());
    let mut checked = Instant::now();
    let mut vk = vk as i32;
    let (clicks_tx, clicks) = mpsc::channel();
    let panel = panel::spawn(clicks_tx);
    let mut state = Dictation::new(HOLD_THRESHOLD);
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
        // 只在没在听写、旧键也松开时换键
        if session.is_none()
            && !was_held
            && state.panel() == Panel::Hidden
            && checked.elapsed() >= Duration::from_secs(1)
        {
            checked = now;
            let changed = watch.check(Settings::modified_default(), || {
                Settings::load_default().ok().map(|s| s.hotkey_vk)
            });
            if let Some(new) = changed {
                println!("快捷键改成 {new:#x}");
                vk = new as i32;
            }
        }
        let held = pressed(vk);
        if held && !was_held && is_alt(vk) {
            mask_menu();
        }
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
        // 主人自己动了键盘或鼠标（不是热键、Esc，也不是点雪耳面板）：通知这轮听写别再往回改字
        if let Some(s) = &session {
            if owner_input(vk, panel.hwnd()) {
                s.touched.fetch_add(1, Ordering::AcqRel);
            }
        }
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
                    session = Some(begin(panel.clone(), status.clone()));
                }
                Effect::Finish => {
                    if let Some(s) = &session {
                        s.finish();
                    }
                }
            }
        }
        match state.panel() {
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

fn begin(panel: PanelHandle, status: Status) -> Session {
    let release = Arc::new(AtomicBool::new(false));
    let cancel = Arc::new(AtomicBool::new(false));
    let sent = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let touched = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let worker_touched = touched.clone();
    let (worker_release, worker_cancel, worker_sent) =
        (release.clone(), cancel.clone(), sent.clone());
    let worker = thread::spawn(move || {
        let cancelled = worker_cancel.clone();
        let result = record(
            worker_release,
            worker_cancel,
            worker_sent,
            worker_touched,
            &panel,
            &status,
        );
        match result {
            // 被新一轮听写取消的不算出错
            Err(_) if cancelled.load(Ordering::Acquire) => status.phase("ready"),
            Err(error) => {
                // 走到这里的连接错误已经自动重连重发过一次了：告诉主人原因，面板留几秒
                let kind = recovery::classify(&error.to_string());
                eprintln!("快捷键听写失败（{kind:?}）：{error}");
                panel.notice(recovery::message(kind));
                status.error(format!("{}（{error}）", recovery::message(kind)));
            }
            Ok(()) => status.phase("ready"),
        }
    });
    Session {
        release,
        cancel,
        sent,
        touched,
        worker,
    }
}

/// 录一轮音：中间结果边说边打到光标处，每句定稿换成整理后的文字或执行指令。
/// 停顿只收句不结束；主人再点一下 / 点麦克风 / Esc，或到「最长听写时间」才结束（面板变灰）。
fn record(
    release: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    sent: Arc<std::sync::atomic::AtomicU32>,
    touched: Arc<std::sync::atomic::AtomicU32>,
    panel: &PanelHandle,
    status: &Status,
) -> crate::install::Result<()> {
    let mut seen_sent = 0;
    let mut seen_touched = 0;
    let settings = Settings::load_default().unwrap_or_default();
    let post = crate::broker::postprocess();
    let mut live = Live::new(foreground_is_terminal());
    live.set_terminal_newline(settings.text.terminal_newline);
    let source = Source::MicrophoneHold {
        device: settings.microphone.clone(),
        release: release.clone(),
    };
    let stopped = cancel.clone();
    let mut typed_any = false;
    let started = Instant::now();
    // 最近一次识别出新字的时间；开头留出说第一句的时间
    let mut last_text = Instant::now();
    let mut last_heard = String::new();
    let max_listen = Duration::from_secs(u64::from(settings.max_listen_minutes) * 60);
    // 这一句里说话的最大音量和最后的增益，一句说完写一行日志（只记数字，不记内容）
    let (mut loudest, mut last_gain) = (0_u32, 1_f32);
    // 中间结果整句是指令、或者句尾是指令：（此刻的中间结果, 从什么时候开始没变）
    let mut pending_command: Option<(String, Instant)> = None;
    let result = engine::run_wetype(source, cancel, |event| {
        if stopped.load(Ordering::Acquire) {
            return Ok(());
        }
        match event {
            Event::Input { name, .. } => status.microphone(name),
            // 停顿只是一句说完（定稿上屏），不结束：一直听到主人再点一下、点麦克风、Esc，
            // 或者超过「最长听写时间」（主人 2026-11-15：说着说着停顿一下就断了，要能一直说）
            // 蓝麦克风=在听；识别出新字的那一阵子显示波浪（只认真的出字，咳嗽、敲桌子不算）
            Event::Activity(phase) => {
                // 一句说完（paused）后其实还在接着听，控制面板上照样显示「聆听中」
                status.phase(if phase == "paused" { "armed" } else { phase });
                if phase == "paused" {
                    println!("一句说完：原始峰值 {loudest}，自动放大 {last_gain:.1} 倍");
                    loudest = 0;
                }
                panel.set(if matches!(phase, "connecting" | "reconnecting") {
                    Look::Connecting
                } else {
                    Look::Listening
                });
            }
            Event::Progress {
                peak,
                gain,
                partials,
                finals,
                ..
            } => {
                loudest = loudest.max(peak);
                last_gain = gain;
                // 指令说完停了一下、没再出新字：现在就执行，不等静音收句
                if let Some((text, since)) = &pending_command {
                    if since.elapsed() >= COMMAND_SETTLE {
                        let keys = match post.command_only(text) {
                            Some(command) => live.early_command(command),
                            // 句尾指令：前面的话先定稿上屏（会记进历史），再按指令
                            None => live
                                .early_finish(post.finalize(text, crate::broker::now_ms()), text),
                        };
                        pending_command = None;
                        typed_any |= keys.iter().any(|k| matches!(k, Key::Text(_)));
                        send_keys(&keys);
                    }
                }
                status.progress(peak, partials, finals);
                panel.level(peak);
                if last_text.elapsed() >= WAVE_HOLD {
                    panel.set_if_changed(Look::Listening);
                }
                // 到「最长听写时间」，或者 10 秒没识别出新字，就自己结束
                if started.elapsed() >= max_listen || last_text.elapsed() >= QUIET_STOP {
                    release.store(true, Ordering::Release);
                }
            }
            Event::Speech(speech) => {
                if speech.text != last_heard {
                    last_heard = speech.text.clone();
                    last_text = Instant::now();
                    panel.set_if_changed(Look::Speaking);
                }
                // 主人在这句说到一半时按了 Enter：屏幕上的部分已经发走，后面只打剩下的
                let now_sent = sent.load(Ordering::Acquire);
                if now_sent != seen_sent {
                    seen_sent = now_sent;
                    live.user_sent();
                }
                // 主人动了键盘或鼠标：这句不再往回改字，只往后接
                let now_touched = touched.load(Ordering::Acquire);
                if now_touched != seen_touched {
                    seen_touched = now_touched;
                    live.user_touched();
                }
                let text = live.unsent(&speech.text);
                let keys = if speech.final_result {
                    pending_command = None;
                    live.finalize_from(&text, post.finalize(&text, crate::broker::now_ms()))
                } else {
                    // 整句只有指令词，或者句尾单独一小句是指令（「今天吃什么，发送」）：
                    // 记下来，停一小会儿没再说别的就提前执行
                    let is_command = post.command_only(&text).is_some()
                        || post.trailing_command(&text).is_some();
                    pending_command = match pending_command.take() {
                        Some((old, since)) if is_command && old == text => Some((old, since)),
                        _ if is_command => Some((text.clone(), Instant::now())),
                        _ => None,
                    };
                    live.partial(&text)
                };
                typed_any |= keys.iter().any(|k| matches!(k, Key::Text(_)));
                send_keys(&keys);
            }
            _ => {}
        }
        Ok(())
    });
    if let Err(error) = result {
        // 屏幕上已经出的字不动；还没说定的那截原样存进历史，免得白说
        if !stopped.load(Ordering::Acquire) {
            post.keep_unfinished(live.leftover(), crate::broker::now_ms());
        }
        return Err(error);
    }
    if !typed_any && !stopped.load(Ordering::Acquire) {
        crate::sound::play(crate::sound::Cue::Missed, settings.sounds_enabled);
    }
    Ok(())
}

/// 正在往前台发我们自己的按键：这期间看到的按键状态不是主人的
static TYPING: AtomicBool = AtomicBool::new(false);

/// 系统记的「最近一次输入」时间。SendInput 注入的也算，所以每次我们自己发完键就记一下，
/// 之后这个值再变，才可能是主人动的。
static LAST_SEEN_INPUT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn last_input_tick() -> u32 {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut info = LASTINPUTINFO {
        cbSize: size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    unsafe {
        GetLastInputInfo(&mut info);
    }
    info.dwTime
}

fn remember_own_input() {
    LAST_SEEN_INPUT.store(last_input_tick(), Ordering::Release);
}

/// 主人是不是刚动了键盘或鼠标键。条件：系统的输入时间变了，并且此刻真有键按着——
/// 鼠标左/右/中键（点在雪耳面板上的不算），或者热键、Esc、修饰键以外的任意键盘键。
/// 只看「此刻有没有键按着」，不记录按的是什么；光移动鼠标不算。
fn owner_input(hotkey: i32, panel: usize) -> bool {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetCursorPos, WindowFromPoint};
    if TYPING.load(Ordering::Acquire) || INJECTING.load(Ordering::Acquire) {
        return false;
    }
    let tick = last_input_tick();
    if LAST_SEEN_INPUT.swap(tick, Ordering::AcqRel) == tick {
        return false;
    }
    let mouse = [0x01, 0x02, 0x04].into_iter().any(pressed);
    if mouse {
        let mut point = POINT { x: 0, y: 0 };
        let on_panel = unsafe {
            GetCursorPos(&mut point);
            WindowFromPoint(point) as usize == panel && panel != 0
        };
        return !on_panel;
    }
    // 0x08 退格起到 0xFE；跳过热键、Esc、Enter（回车另有处理）、各种 Shift/Ctrl/Alt/Win、
    // 以及我们自己注入用的 0xE7（Unicode 包）和 0xE8（菜单遮罩）
    (0x08..=0xFE).any(|key: i32| {
        key != hotkey
            && !matches!(
                key,
                0x0D | 0x10 | 0x11 | 0x12 | 0x1B | 0x5B | 0x5C | 0xA0..=0xA5 | 0xE7 | 0xE8
            )
            && pressed(key)
    })
}

fn is_alt(vk: i32) -> bool {
    matches!(vk, 0x12 | 0xA4 | 0xA5)
}

/// 热键是 Alt 时，趁 Alt 还按着补发一个没人用的键（vkE8，和 AutoHotkey 的 MenuMaskKey 一样），
/// 这样松开 Alt 时前台窗口不会进入菜单模式把后面打的字吃掉（主人：WSL 里有时不出字）。
fn mask_menu() {
    let inputs = [key(0xE8, 0, 0), key(0xE8, 0, KEYEVENTF_KEYUP)];
    unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        );
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

/// Qt 程序（微信 4 是 Qt 5.15）单独处理：SendInput 的 Unicode 方式在 Qt 里会把每批第一个字
/// 认成上一批的最后一个字（Qt 把连续的 VK_PACKET 当成同一个键的自动重复）。
/// 所以对 Qt 窗口：文字发 WM_CHAR、退格发 VK_BACK 的按下/抬起（Qt 不认 WM_CHAR 里的 \b，
/// 2026-11-15 退格也走 WM_CHAR 时，微信里每句都重复一次）。其他程序照旧 SendInput。
/// 返回 false 表示不是 Qt 窗口或发不过去，调用方退回 SendInput。
fn send_to_qt(key: &Key) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GUITHREADINFO, GetGUIThreadInfo, GetWindowThreadProcessId, SMTO_ABORTIFHUNG, SMTO_BLOCK,
        SendMessageTimeoutW, WM_CHAR, WM_KEYDOWN, WM_KEYUP,
    };
    let messages: Vec<(u32, usize, isize)> = match key {
        Key::Text(text) => text
            .encode_utf16()
            .map(|u| (WM_CHAR, usize::from(u), 1))
            .collect(),
        Key::Backspace(count) => (0..*count)
            .flat_map(|_| {
                [
                    (WM_KEYDOWN, VK_BACK as usize, 0x000e_0001),
                    (WM_KEYUP, VK_BACK as usize, 0xc00e_0001_u32 as isize),
                ]
            })
            .collect(),
        _ => return false,
    };
    unsafe {
        let window = GetForegroundWindow();
        let mut class = [0_u16; 64];
        let count = GetClassNameW(window, class.as_mut_ptr(), class.len() as i32);
        if count <= 0 || !String::from_utf16_lossy(&class[..count as usize]).starts_with("Qt") {
            return false;
        }
        let thread = GetWindowThreadProcessId(window, std::ptr::null_mut());
        let mut info: GUITHREADINFO = std::mem::zeroed();
        info.cbSize = size_of::<GUITHREADINFO>() as u32;
        if GetGUIThreadInfo(thread, &mut info) == 0 || info.hwndFocus.is_null() {
            return false;
        }
        for (message, wparam, lparam) in messages {
            let mut result = 0;
            if SendMessageTimeoutW(
                info.hwndFocus,
                message,
                wparam,
                lparam,
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
        // Qt 窗口（微信）里的文字和退格单独发；前面攒着的按键先发出去，保证顺序
        if matches!(k, Key::Text(_) | Key::Backspace(_)) {
            flush(&mut inputs);
            if send_to_qt(k) {
                remember_own_input();
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
    // 我们自己发键的这一小段时间，轮询线程不把按键算成主人的（退格是真的 VK_BACK，会被看到）
    TYPING.store(true, Ordering::Release);
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    // 等目标程序把这批键处理掉、键都抬起来，再记下输入时间并放开
    thread::sleep(Duration::from_millis(20));
    remember_own_input();
    TYPING.store(false, Ordering::Release);
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
