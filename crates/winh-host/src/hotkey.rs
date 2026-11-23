//! 快捷键听写（默认右 Alt）：雪耳自己的面板 + 边说边出字，不经过微软的 Win+H。
//!
//! 交互见 `dictation.rs`：点一下打开面板开始听，再点一下、点面板或按 Esc 结束。
//! 只轮询自己关心的几个键（GetAsyncKeyState），不装全局键盘钩子、不记录按键。
//! 键入用 SendInput 的 Unicode 方式，只发我们识别出的文字和语音指令对应的键。
//! 语音指令管整个输入框（`inputbox`）；什么时候算单独说的指令见 `command_gate`。
use crate::{
    command_gate::{CommandGate, Finish, Partial},
    dictation::{Dictation, Effect, Input, Panel},
    focus::{Delivery, Probe, Route},
    inputbox::{self, BoxText, Kind},
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
        atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows_sys::Win32::UI::{
    Input::KeyboardAndMouse::{
        GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
        KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, SendInput, VK_BACK, VK_CAPITAL, VK_CONTROL, VK_END,
        VK_ESCAPE, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RETURN,
        VK_RIGHT, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
    },
    WindowsAndMessaging::{GetClassNameW, GetForegroundWindow},
};
use xueer_ear_backend::{
    engine::{self, Event, Source},
    postprocess::Postprocess,
    settings::{Engine, Settings},
};
use xueer_ear_textflow::{Output, VoiceCommand};

/// 正在往前台发我们自己的按键（里面可能有 Enter），轮询线程这时不把 Enter 算作主人按的
static INJECTING: AtomicBool = AtomicBool::new(false);
/// 现在的快捷键（虚拟键码）：发键前等它松开，免得 Alt 和要打的字凑成组合键
static HOTKEY: AtomicI32 = AtomicI32::new(0);

/// 停了马上又开时，等上一轮把最后一句收完的最长时间（收句最多等 4 秒定稿）
const FINISH_GRACE: Duration = Duration::from_millis(4500);
/// 识别出新字后，波浪保持这么久再变回麦克风
const WAVE_HOLD: Duration = Duration::from_millis(900);

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
    superseded: Arc<AtomicBool>,
    /// 主人自己按 Enter 的次数（轮询线程加，听写线程读），用来去掉已发走的文字
    sent: Arc<AtomicU32>,
    /// 主人动了键盘或鼠标的次数（点别处、打字）：听写线程看到变化就不再往回改字
    touched: Arc<AtomicU32>,
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
    HOTKEY.store(vk, Ordering::Release);
    let (clicks_tx, clicks) = mpsc::channel();
    let panel = panel::spawn(clicks_tx);
    let mut state = Dictation::new();
    let mut session: Option<Session> = None;
    let mut was_held = false;
    let mut escape_key = EscapeKey::default();
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
                HOTKEY.store(vk, Ordering::Release);
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
            // CapsLock 当热键：系统已经因为这一下切换了大小写锁定，再按一下把它切回去（审查 R15）
            if vk == i32::from(VK_CAPITAL) {
                undo_caps_toggle();
            }
        }
        was_held = held;
        // 主人自己动了键盘或鼠标（不是热键、Esc，也不是点雪耳面板）：通知这轮听写别再往回改字
        if let Some(s) = &session
            && owner_input(vk, panel.hwnd())
        {
            s.touched.fetch_add(1, Ordering::AcqRel);
        }
        // 主人自己按 Enter（我们注入 Enter 时会先置 INJECTING，不算）
        let enter = pressed(i32::from(VK_RETURN));
        if enter
            && !was_enter
            && !INJECTING.load(Ordering::Acquire)
            && let Some(s) = &session
        {
            s.sent.fetch_add(1, Ordering::AcqRel);
        }
        was_enter = enter;
        // 面板开着时把 Esc 登记成雪耳自己的热键：这一下 Esc 只给雪耳，不会再发给前台程序
        // （审查 R10：原来只是轮询，Esc 同时打断了 Claude Code 里正在跑的任务）。面板收起就还回去。
        escape_key.hold(state.panel() != Panel::Hidden);
        if escape_key.pressed() && state.panel() != Panel::Hidden {
            inputs.push(Input::Close);
        }
        while let Ok(click) = clicks.try_recv() {
            match click {
                Click::Mic => inputs.push(Input::MicClick),
                // 左边设置：打开控制面板（已经开着就切到前面）；右边翻译：弹翻译小窗
                Click::Settings => open_control_panel(&[]),
                Click::Translate => open_control_panel(&["--translate"]),
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
                    // 上一轮可能还在收尾（已停止采音、在等最后一句定稿）：交给新一轮的线程去等它收完再开录，
                    // 轮询线程不卡（审查 R9：原来直接取消上一轮，最后一句定稿丢了）
                    let previous = session.take();
                    if let Some(old) = &previous {
                        old.superseded.store(true, Ordering::Release);
                        old.finish();
                    }
                    panel.set(Look::Connecting);
                    session = Some(begin(panel.clone(), status.clone(), previous));
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

fn begin(panel: PanelHandle, status: Status, previous: Option<Session>) -> Session {
    let release = Arc::new(AtomicBool::new(false));
    let cancel = Arc::new(AtomicBool::new(false));
    let superseded = Arc::new(AtomicBool::new(false));
    let sent = Arc::new(AtomicU32::new(0));
    let touched = Arc::new(AtomicU32::new(0));
    let worker_touched = touched.clone();
    let worker_superseded = superseded.clone();
    let (worker_release, worker_cancel, worker_sent) =
        (release.clone(), cancel.clone(), sent.clone());
    let worker = thread::spawn(move || {
        if let Some(old) = previous {
            // 等上一轮把最后一句收完，最多 FINISH_GRACE；超时就取消它，免得两轮同时往屏幕上打字
            let deadline = Instant::now() + FINISH_GRACE;
            while !old.worker.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
            old.cancel.store(true, Ordering::Release);
            let _ = old.worker.join();
        }
        // 这一轮开始：雪儿 speaker 先不出声，这轮结束（收起、出错、被新一轮接替）再念
        let _round = status.round();
        let cancelled = worker_cancel.clone();
        let result = record(
            worker_release,
            worker_cancel,
            worker_superseded,
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
            Ok(()) => match crate::broker::settings().1 {
                // 设置文件坏了：面板提示几秒，控制面板显示原因，直到下一轮开始（审查 R14）
                Some(reason) => {
                    panel.notice("设置文件坏了，用的是默认设置");
                    status.error(reason);
                }
                None => status.phase("ready"),
            },
        }
    });
    Session {
        release,
        cancel,
        superseded,
        sent,
        touched,
        worker,
    }
}

/// 这一轮哪家为主：上一轮屋里吵（开着风扇这类）、又选了豆包为主时，这轮微信为主、豆包备用
/// （主人 2026-11-24 真实风扇录音：豆包错字 14～17%，微信 7%）。安静了就回到选的那家
fn channel_for_round(chosen: Engine) -> Engine {
    match (chosen, xueer_ear_audio::room::last_round_noisy()) {
        (Engine::Doubao, Some(true)) => {
            println!("上一轮屋里吵，这轮微信为主、豆包备用");
            Engine::Wetype
        }
        _ => chosen,
    }
}

/// 录一轮音：中间结果边说边打到光标处，每句定稿换成整理后的文字或执行指令。
/// 停顿只收句不结束；主人再点一下 / 点面板 / Esc，或到「最长听写时间」才结束（面板变灰）。
fn record(
    release: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    superseded: Arc<AtomicBool>,
    sent: Arc<AtomicU32>,
    touched: Arc<AtomicU32>,
    panel: &PanelHandle,
    status: &Status,
) -> crate::install::Result<()> {
    // 这一轮开头每一步的耗时（只记毫秒和通道），排查「刚醒来按右 Alt 反应慢」
    let round_began = Instant::now();
    let mut opening = Opening::default();
    let mut seen_sent = 0;
    let mut seen_touched = 0;
    // 设置文件坏了就先用默认值听写，这一轮结束时再提示（开头提示会被「在听」的样子马上盖掉）
    let settings = crate::broker::settings().0;
    let post = crate::broker::postprocess();
    let terminal = foreground_is_terminal();
    let mut live = Live::new(terminal);
    let target = foreground_text_target();
    // 排查用：XUEER_EAR_TEST_WAVE=文件.wav 时用录好的音代替麦克风，走完全一样的上屏和指令流程
    let source = match std::env::var_os("XUEER_EAR_TEST_WAVE") {
        Some(path) => Source::Wave(path.into()),
        None => Source::MicrophoneHold {
            device: settings.microphone.clone(),
            release: release.clone(),
        },
    };
    let wave_probe = matches!(&source, Source::Wave(_));
    // 这一轮听写期间别的程序小声一点，record 结束（含出错返回）时恢复
    let _ducking = (!wave_probe).then(|| crate::broker::duck_others(&settings));
    // 前台有没有能打字的地方（终端一律当能打字）；没有的话这一轮的字攒着、说完复制
    let mut delivery = Delivery::new((!wave_probe && !terminal).then(Probe::start));
    // 「被新一轮取消」要单独记：引擎收完一轮正常结束时也会把传进去的 stop 置位（Capture 的 Drop），
    // 直接拿 cancel 判断的话，每轮正常结束都像被取消，说「翻译」整段替换就永远不执行（2026-11-19 复现）
    let stopped = cancel.clone();
    let engine_stop = Arc::new(AtomicBool::new(false));
    {
        let (from, to) = (cancel.clone(), engine_stop.clone());
        thread::spawn(move || {
            while !to.load(Ordering::Acquire) {
                if from.load(Ordering::Acquire) {
                    to.store(true, Ordering::Release);
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
        });
    }
    let mut typed_any = false;
    // 雪耳往屏幕上发过几次会改字的键：译文回来时这个数变了，说明框里又多了字，译文就过时了
    let mut typed = 0_u32;
    let started = Instant::now();
    // 最近一次识别出新字的时间；开头留出说第一句的时间
    let mut last_text = Instant::now();
    let mut last_heard = String::new();
    let max_listen = Duration::from_secs(u64::from(settings.max_listen_minutes) * 60);
    // 这一句里说话的最大音量和最后的增益，一句说完写一行日志（只记数字，不记内容）
    let (mut loudest, mut last_gain) = (0_u32, 1_f32);
    // 语音指令：前面停够、单独说、说完停一下才算（主人 2026-11-24）
    let mut gate = CommandGate::new(settings.commands.pause(), settings.commands.settle());
    let mut translated_requested = false;
    let mut translation: Option<VoiceTranslation> = None;
    let still_current = |(sent_at_command, touched_at_command)| {
        !stopped.load(Ordering::Acquire)
            && !superseded.load(Ordering::Acquire)
            && sent.load(Ordering::Acquire) == sent_at_command
            && touched.load(Ordering::Acquire) == touched_at_command
            && target.is_some()
            && foreground_text_target() == target
    };
    let engine = channel_for_round(settings.engine);
    let result = engine::run_cloud(source, engine, engine_stop.clone(), |event| {
        if stopped.load(Ordering::Acquire) {
            return Ok(());
        }
        let mut command: Option<(String, &str)> = None;
        let mut keys = Vec::new();
        match event {
            Event::Input { name, .. } => status.microphone(name),
            Event::Started => opening.microphone = Some(round_began.elapsed()),
            // 主通道这句不行、改走了备用：面板上说一声，不打断听写
            Event::Channel {
                name,
                fallback,
                waited_ms,
            } => {
                status.channel(name);
                if fallback {
                    panel.listening_notice(&format!(
                        "{}没连上，这句改用{}",
                        channel_name(other_channel(name)),
                        channel_name(name)
                    ));
                }
                opening.connected(name, waited_ms);
            }
            Event::Voice { pause_ms } => gate.voice(pause_ms),
            // 停顿只是一句说完（定稿上屏），不结束：一直听到主人再点一下、点面板、Esc，
            // 或者超过「最长听写时间」（主人 2026-11-15：说着说着停顿一下就断了，要能一直说）
            // 蓝麦克风=在听；识别出新字的那一阵子显示波浪（只认真的出字，咳嗽、敲桌子不算）
            Event::Activity(phase) => {
                // 一句说完（paused）后其实还在接着听，控制面板上照样显示「聆听中」
                status.phase(if phase == "paused" { "armed" } else { phase });
                if phase == "paused" {
                    last_heard.clear();
                    println!("一句说完：原始峰值 {loudest}，自动放大 {last_gain:.1} 倍");
                    loudest = 0;
                }
                // 主人已经点了停止（面板收起了），这轮还在收最后一句：别再把面板叫出来。
                // 原来收尾时的「一句说完」会把面板重新显示一下、轮询线程又马上收起——看起来就是闪两下（2026-11-19）
                if !release.load(Ordering::Acquire) && translation.is_none() {
                    panel.set(if matches!(phase, "connecting" | "reconnecting") {
                        Look::Connecting
                    } else {
                        Look::Listening
                    });
                }
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
                // 单独说完指令停一小会儿（设置「指令后等」，默认 0.6 秒）没再出新字就执行，不等云端定稿
                if let Some(spoken) = gate.due(Instant::now()) {
                    command = Some((spoken, "停够了执行"));
                }
                status.progress(peak, partials, finals);
                panel.level(peak);
                if last_text.elapsed() >= WAVE_HOLD
                    && !release.load(Ordering::Acquire)
                    && translation.is_none()
                {
                    panel.set_if_changed(Look::Listening);
                }
                // 到「最长听写时间」，或者好一阵（设置「没有新字自动结束」，默认 10 秒）没识别出新字，就自己结束
                if started.elapsed() >= max_listen || last_text.elapsed() >= settings.quiet_stop() {
                    release.store(true, Ordering::Release);
                }
            }
            Event::Speech(speech) => {
                if !speech.text.trim().is_empty() {
                    opening.first_text(round_began.elapsed());
                }
                // 定稿只补标点不算新字，不能给本轮的空闲计时续命。
                if !speech.text.trim().is_empty()
                    && !speech
                        .text
                        .chars()
                        .filter(|c| c.is_alphanumeric())
                        .eq(last_heard.chars().filter(|c| c.is_alphanumeric()))
                {
                    last_heard = speech.text.clone();
                    last_text = Instant::now();
                    if !release.load(Ordering::Acquire) {
                        panel.set_if_changed(Look::Speaking);
                    }
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
                    delivery.touched(Probe::start);
                }
                let text = live.unsent(&speech.text);
                let is_command = post.command_only(&text).is_some();
                if speech.final_result {
                    match gate.finished(&text, is_command) {
                        Finish::Text(text) => {
                            let output = post.finalize_text(&text, crate::broker::now_ms());
                            keys = live.finalize_from(&text, output);
                        }
                        Finish::Command(spoken) => {
                            keys = live.finalize(Output::Text(String::new()));
                            command = Some((spoken, "定稿时执行"));
                        }
                    }
                } else {
                    match gate.partial(
                        &text,
                        post.starts_command(&text),
                        is_command,
                        Instant::now(),
                    ) {
                        Partial::Hold => {}
                        Partial::Type(text) => keys = live.partial(&text),
                    }
                }
            }
        }
        typed_any |= keys.iter().any(|k| matches!(k, Key::Text(_)));
        if deliver(&mut delivery, &keys, panel) {
            typed += 1;
        }
        if let Some((spoken, how)) = command
            && let Some(command) = post.command_only(&spoken)
        {
            println!("指令：{}（{how}）", command.name());
            let input = (seen_sent, seen_touched);
            match run_command(command, terminal, &mut live, &mut delivery, panel) {
                Ran::Done => typed += 1,
                Ran::Nothing => {}
                Ran::Translate(source, target) => {
                    translated_requested = true;
                    translation = VoiceTranslation::start(source, target, input, typed)
                        .map_err(|reason| panel.listening_notice(&format!("翻译失败：{reason}")))
                        .ok();
                    if translation.is_some() {
                        panel.set(Look::Connecting);
                    }
                }
            }
        }
        if translation
            .as_ref()
            .is_some_and(|job| !still_current(job.input) || job.typed != typed)
        {
            println!("翻译作废：输入框在等译文时变了");
            translation = None;
            panel.set(Look::Listening);
        }
        if let Some(job) = translation.as_ref()
            && let Some(result) = job.try_result()
        {
            let job = translation.take().expect("checked above");
            if apply_translation(job, result, &mut live, panel) {
                typed += 1;
            }
        }
        if release.load(Ordering::Acquire) && !wave_probe {
            return Ok(());
        }
        Ok(())
    });
    // 这一轮没有输入框：说的话（出错中断的也算）放进剪贴板
    if let Some(text) = delivery.held() {
        match crate::clipboard::copy(panel.hwnd(), text) {
            Ok(()) => panel.notice("没找到输入框，已复制，Ctrl+V 粘贴"),
            Err(reason) => panel.notice(&format!("没找到输入框，{reason}")),
        }
        println!("没找到输入框，这轮 {} 字放进剪贴板", text.chars().count());
    }
    if let Err(error) = result {
        // 屏幕上已经出的字不动；还没说定的那截原样存进历史，免得白说
        if !stopped.load(Ordering::Acquire) {
            post.keep_unfinished(live.leftover(), crate::broker::now_ms());
        }
        return Err(error);
    }
    println!(
        "这轮结束：说了翻译={translated_requested} 取消={} 已出字={typed_any}",
        stopped.load(Ordering::Acquire)
    );
    // 「翻译成英文」说完马上收起了：译文回来时框里没变就照样换上
    if let Some(job) = translation {
        match job.wait(|| still_current(job.input)) {
            Some(result) if still_current(job.input) => {
                apply_translation(job, result, &mut live, panel);
            }
            _ => {
                println!("翻译作废：这轮已结束，输入框变了");
                panel.notice("翻译回来时输入框已经变了，没替换");
            }
        }
    }
    if !typed_any && !translated_requested && !stopped.load(Ordering::Acquire) {
        crate::sound::play(crate::sound::Cue::Missed, settings.sounds_enabled);
    }
    Ok(())
}

/// 一轮开头的耗时：按下后多久开好麦，第一句开口后多久连上哪家，按下后多久出第一个字。
/// 出第一个字时写一行日志；之后哪句连得慢（超过 0.8 秒）再单独记一行
#[derive(Default)]
struct Opening {
    microphone: Option<Duration>,
    channel: Option<(&'static str, u32)>,
    logged: bool,
}

impl Opening {
    fn connected(&mut self, name: &'static str, waited_ms: u32) {
        if self.channel.is_none() {
            self.channel = Some((name, waited_ms));
        } else if waited_ms > 800 {
            println!("这句开口后 {waited_ms} ms 才连上{}", channel_name(name));
        }
    }

    fn first_text(&mut self, since_press: Duration) {
        if self.logged {
            return;
        }
        self.logged = true;
        let microphone = self.microphone.map_or(0, |d| d.as_millis());
        let (name, waited) = self.channel.unwrap_or(("", 0));
        println!(
            "这轮开头：按下后 {microphone} ms 开麦，开口后 {waited} ms 连上{}，按下后 {} ms 出第一个字",
            channel_name(name),
            since_press.as_millis()
        );
    }
}

fn channel_name(name: &str) -> &'static str {
    match name {
        "doubao" => "豆包",
        "wetype" => "微信",
        _ => "云端",
    }
}

fn other_channel(name: &str) -> &'static str {
    if name == "doubao" { "wetype" } else { "doubao" }
}

/// 拉起同目录的 xueer-ear.exe（控制面板程序）。它自己保证只开一个控制面板。
fn open_control_panel(args: &[&str]) {
    let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
    else {
        return;
    };
    if let Err(error) = std::process::Command::new(dir.join("xueer-ear.exe"))
        .args(args)
        .spawn()
    {
        eprintln!("打不开控制面板：{error}");
    }
}

/// 执行了指令之后
enum Ran {
    /// 按了键，输入框变了
    Done,
    /// 什么都没按（没找到输入框、框是空的……面板上已经说了原因）
    Nothing,
    /// 读到了要翻的整个输入框，去翻译
    Translate(BoxText, xueer_ear_backend::translate::Target),
}

/// 执行一个语音指令（主人 2026-11-24：指令管焦点所在输入框的全部文字）
fn run_command(
    command: VoiceCommand,
    terminal: bool,
    live: &mut Live,
    delivery: &mut Delivery,
    panel: &PanelHandle,
) -> Ran {
    if delivery.holding() {
        panel.listening_notice("没找到输入框");
        return Ran::Nothing;
    }
    if command == VoiceCommand::Send {
        send_keys(&[Key::Enter]);
        live.box_changed("");
        return Ran::Done;
    }
    let read = inputbox::read(terminal, &|keys| send_keys(keys), panel.hwnd());
    let box_text = match read {
        Ok(box_text) => box_text,
        Err(reason) => {
            println!("指令没执行：{reason}");
            panel.listening_notice(&reason);
            return Ran::Nothing;
        }
    };
    if command.is_translate() {
        let target = if command == VoiceCommand::TranslateToEnglish {
            xueer_ear_backend::translate::Target::English
        } else {
            xueer_ear_backend::translate::Target::Chinese
        };
        let count = box_text.text.chars().count();
        if box_text.text.trim().is_empty() {
            panel.listening_notice("输入框是空的");
            return Ran::Nothing;
        }
        if count > inputbox::MAX_CHARS {
            panel.listening_notice(&format!(
                "输入框里超过 {} 字，没翻；可以用翻译小窗",
                inputbox::MAX_CHARS
            ));
            return Ran::Nothing;
        }
        return Ran::Translate(box_text, target);
    }
    match box_text.delete(command) {
        Ok(change) => {
            send_keys(&change.keys);
            live.box_changed(&change.after);
            Ran::Done
        }
        Err(reason) => {
            panel.listening_notice(&reason);
            Ran::Nothing
        }
    }
}

/// 后台在翻的整个输入框
struct VoiceTranslation {
    source: BoxText,
    result: mpsc::Receiver<Result<String, String>>,
    /// 说指令时主人按回车、动键鼠的次数：变了就说明框可能变了
    input: (u32, u32),
    /// 说指令时雪耳发过几次改字的键：之后又打了字，译文就过时了
    typed: u32,
    started: Instant,
}

impl VoiceTranslation {
    fn start(
        source: BoxText,
        target: xueer_ear_backend::translate::Target,
        input: (u32, u32),
        typed: u32,
    ) -> Result<Self, String> {
        let text = source.text.clone();
        let (sender, result) = mpsc::channel();
        thread::Builder::new()
            .spawn(move || {
                let _ = sender.send(xueer_ear_backend::translate::translate_into(&text, target));
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            source,
            result,
            input,
            typed,
            started: Instant::now(),
        })
    }

    fn try_result(&self) -> Option<Result<String, String>> {
        match self.result.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("翻译线程意外退出".into())),
        }
    }

    fn wait(&self, still_current: impl Fn() -> bool) -> Option<Result<String, String>> {
        let deadline = Instant::now() + Duration::from_secs(60);
        while still_current() && Instant::now() < deadline {
            match self.result.recv_timeout(Duration::from_millis(25)) {
                Ok(result) => return Some(result),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Some(Err("翻译线程意外退出".into()));
                }
            }
        }
        None
    }
}

/// 译文回来了：框里还是当时那些字就整框换掉（读得到的框再读一遍核对；全选复制读的，
/// 靠主人没动键鼠、雪耳没再打字来保证）。失败就在面板上说一声。返回按了键没有
fn apply_translation(
    job: VoiceTranslation,
    result: Result<String, String>,
    live: &mut Live,
    panel: &PanelHandle,
) -> bool {
    let waited = job.started.elapsed().as_millis();
    panel.set(Look::Listening);
    let translated = match result {
        Ok(text) => text,
        Err(reason) => {
            println!(
                "翻译失败：{} 字，{waited} ms",
                job.source.text.chars().count()
            );
            eprintln!("翻译失败：{reason}");
            panel.listening_notice(&format!("翻译失败：{reason}"));
            return false;
        }
    };
    if job.source.kind != Kind::Copied {
        let now = inputbox::read(job.source.kind == Kind::Terminal, &|_| {}, panel.hwnd());
        if now.as_ref().map(|b| b.text.as_str()) != Ok(job.source.text.as_str()) {
            println!("翻译回来：输入框已经变了，没替换（{waited} ms）");
            panel.listening_notice("翻译回来时输入框已经变了，没替换");
            return false;
        }
    }
    let change = job.source.replace(&translated);
    send_keys(&change.keys);
    live.box_changed(&change.after);
    println!(
        "翻译回来：整框 {} 字 → {} 字，{waited} ms，已替换",
        job.source.text.chars().count(),
        translated.chars().count()
    );
    true
}

/// 发按键，或者（这一轮前台没有输入框时）攒起来；第一次攒时在面板上说一声。返回真的发到屏幕上了没有
fn deliver(delivery: &mut Delivery, keys: &[Key], panel: &PanelHandle) -> bool {
    if keys.is_empty() {
        return false;
    }
    match delivery.route(keys) {
        Route::Screen => {
            send_keys(keys);
            true
        }
        Route::Held { started: true } => {
            panel.listening_notice("没找到输入框，说完自动复制");
            false
        }
        Route::Held { started: false } => false,
    }
}

/// 比较前台窗口和当前得到焦点的控件，防止迟到的译文被打到别处。
fn foreground_text_target() -> Option<(usize, usize)> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GUITHREADINFO, GetGUIThreadInfo};
    let window = unsafe { GetForegroundWindow() };
    if window.is_null() {
        return None;
    }
    let mut info: GUITHREADINFO = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<GUITHREADINFO>() as u32;
    let focused = unsafe { GetGUIThreadInfo(0, &mut info) } != 0;
    Some((
        window as usize,
        if focused { info.hwndFocus as usize } else { 0 },
    ))
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

/// 面板开着时独占 Esc（RegisterHotKey，只登记这一个键，不装键盘钩子）。
/// 热键消息发到轮询线程自己的消息队列，`pressed` 取走。
#[derive(Default)]
struct EscapeKey {
    held: bool,
    /// 登记失败退回轮询时，上一次看到的 Esc 状态
    fallback_down: bool,
}

const ESCAPE_HOTKEY_ID: i32 = 0x5845; // 'XE'

impl EscapeKey {
    fn hold(&mut self, want: bool) {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey};
        if want == self.held {
            return;
        }
        unsafe {
            if want {
                // 别的程序已经占了 Esc 时登记会失败，那就退回轮询（Esc 会同时发给前台程序）
                self.held = RegisterHotKey(
                    std::ptr::null_mut(),
                    ESCAPE_HOTKEY_ID,
                    0x4000, // MOD_NOREPEAT
                    u32::from(VK_ESCAPE),
                ) != 0;
                if !self.held {
                    self.fallback_down = pressed(i32::from(VK_ESCAPE));
                }
            } else {
                UnregisterHotKey(std::ptr::null_mut(), ESCAPE_HOTKEY_ID);
                self.held = false;
            }
        }
    }

    /// 自从上次问过，Esc 有没有被按下
    fn pressed(&mut self) -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            MSG, PM_REMOVE, PeekMessageW, WM_HOTKEY,
        };
        if !self.held {
            let down = pressed(i32::from(VK_ESCAPE));
            let edge = down && !self.fallback_down;
            self.fallback_down = down;
            return edge;
        }
        let mut hit = false;
        unsafe {
            let mut message: MSG = std::mem::zeroed();
            while PeekMessageW(
                &mut message,
                std::ptr::null_mut(),
                WM_HOTKEY,
                WM_HOTKEY,
                PM_REMOVE,
            ) != 0
            {
                hit |= message.wParam as i32 == ESCAPE_HOTKEY_ID;
            }
        }
        hit
    }
}

impl Drop for EscapeKey {
    fn drop(&mut self) {
        self.hold(false);
    }
}

fn is_alt(vk: i32) -> bool {
    matches!(vk, 0x12 | 0xA4 | 0xA5)
}

/// 补按一下 CapsLock，把热键带来的大小写锁定切换抵消掉。补的这一下不会被当成热键：
/// 轮询线程这一轮已经记下「松开」，补的按下和抬起在同一次 SendInput 里，下一轮看到的还是松开。
fn undo_caps_toggle() {
    let inputs = tap(VK_CAPITAL);
    unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        );
    }
    remember_own_input();
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

/// 终端的窗口类名：Windows Terminal、老式控制台、mintty（Git Bash）、ConEmu、PuTTY。
/// 终端里回车就是发送、Ctrl+C 是中断：句子之间补分隔，读输入框只看屏幕，从不全选复制
const TERMINALS: [&str; 5] = [
    "CASCADIA_HOSTING_WINDOW_CLASS",
    "ConsoleWindowClass",
    "mintty",
    "VirtualConsoleClass",
    "PuTTY",
];

fn foreground_is_terminal() -> bool {
    let mut class = [0_u16; 64];
    let count = unsafe {
        GetClassNameW(
            GetForegroundWindow(),
            class.as_mut_ptr(),
            class.len() as i32,
        )
    };
    count > 0 && TERMINALS.contains(&String::from_utf16_lossy(&class[..count as usize]).as_str())
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

/// 方向键、End 这类在扩展区的键要带扩展标志，不然会被当成小键盘上的
fn extended_tap(vk: u16) -> [INPUT; 2] {
    [
        key(vk, 0, KEYEVENTF_EXTENDEDKEY),
        key(vk, 0, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP),
    ]
}

/// 雪耳自己的翻译小窗（xueer-ear.exe 里的系统输入框）
const TRANSLATE_WINDOW_CLASS: &str = "XueerEar.TranslateWindow";

/// 不走 SendInput、直接发消息给有焦点的输入框的两种窗口：
/// - Qt 程序（微信 4 是 Qt 5.15）：SendInput 的 Unicode 方式在 Qt 里会把每批第一个字
///   认成上一批的最后一个字（Qt 把连续的 VK_PACKET 当成同一个键的自动重复）。
///   文字发 WM_CHAR、退格发 VK_BACK 的按下/抬起（Qt 不认 WM_CHAR 里的 \b，
///   2026-11-15 退格也走 WM_CHAR 时，微信里每句都重复一次）。
/// - 雪耳的翻译小窗：中文输入法开着时，SendInput 打进去的英文字母被输入法当拼音接走，
///   出来一串乱码（主人 2026-11-21）。系统输入框认 WM_CHAR 里的 \b 和 \r，全走 WM_CHAR。
///
/// 其他程序照旧 SendInput。返回 false 表示不是这两种窗口或发不过去，调用方退回 SendInput。
fn send_to_qt(key: &Key) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GUITHREADINFO, GetGUIThreadInfo, GetWindowThreadProcessId, SMTO_ABORTIFHUNG, SMTO_BLOCK,
        SendMessageTimeoutW,
    };
    if !matches!(key, Key::Text(_) | Key::Backspace(_)) {
        return false;
    }
    unsafe {
        let window = GetForegroundWindow();
        let mut class = [0_u16; 64];
        let count = GetClassNameW(window, class.as_mut_ptr(), class.len() as i32);
        let class = String::from_utf16_lossy(&class[..count.max(0) as usize]);
        let edit = class == TRANSLATE_WINDOW_CLASS;
        if !edit && !class.starts_with("Qt") {
            return false;
        }
        let messages = window_messages(key, edit);
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

/// `send_to_qt` 要发的消息。`edit` = 系统输入框（翻译小窗）：退格是 WM_CHAR \b，换行是 WM_CHAR \r；
/// 否则（Qt）退格发 VK_BACK 的按下/抬起
fn window_messages(key: &Key, edit: bool) -> Vec<(u32, usize, isize)> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_CHAR, WM_KEYDOWN, WM_KEYUP};
    match key {
        Key::Text(text) => text
            .encode_utf16()
            .map(|u| match u {
                0x0A if edit => (WM_CHAR, 0x0D, 1),
                _ => (WM_CHAR, usize::from(u), 1),
            })
            .collect(),
        Key::Backspace(count) if edit => (0..*count).map(|_| (WM_CHAR, 0x08, 1)).collect(),
        Key::Backspace(count) => (0..*count)
            .flat_map(|_| {
                [
                    (WM_KEYDOWN, VK_BACK as usize, 0x000e_0001),
                    (WM_KEYUP, VK_BACK as usize, 0xc00e_0001_u32 as isize),
                ]
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// 排查漏字用：等 `delay_ms` 让人把焦点切到目标窗口，然后按听写的打字方式把 `text` 分 `pieces` 次打出去
/// （模拟边说边出字：每次接一截），最后补一个换行。只发这段文字，不读屏幕。
/// 排查用：起一个面板，模拟按一下快捷键，用 XUEER_EAR_TEST_WAVE 里的录音跑一整轮听写（字打进前台窗口）
pub fn wave_probe(delay_ms: u64) {
    thread::sleep(Duration::from_millis(delay_ms));
    let (clicks, _) = mpsc::channel();
    let panel = panel::spawn(clicks);
    let status = Status::new("wetype").expect("status");
    let session = begin(panel.clone(), status, None);
    let _ = session.worker.join();
    thread::sleep(Duration::from_millis(500));
}

pub fn type_probe(text: &str, pieces: usize, delay_ms: u64) {
    thread::sleep(Duration::from_millis(delay_ms));
    // 排查用：XUEER_EAR_PROBE=each 每个字单独一次 SendInput
    if std::env::var("XUEER_EAR_PROBE").as_deref() == Ok("each") {
        for c in text.chars() {
            send_keys(&[Key::Text(c.to_string())]);
        }
        send_keys(&[Key::Enter]);
        return;
    }
    let chars: Vec<char> = text.chars().collect();
    let step = chars.len().div_ceil(pieces.max(1));
    for chunk in chars.chunks(step.max(1)) {
        send_keys(&[Key::Text(chunk.iter().collect())]);
        thread::sleep(Duration::from_millis(120));
    }
    send_keys(&[Key::Enter]);
}

/// 标点（中英文）：它后面的字要另起一批发
fn is_mark(unit: u16) -> bool {
    char::from_u32(u32::from(unit))
        .is_some_and(|c| c.is_ascii_punctuation() || "，。！？；：、…—“”‘’（）《》【】".contains(c))
}

/// 一批最多发这么多个字（或退格），批与批之间等 flush 里的 20 ms
const BATCH_UNITS: usize = 12;

fn send_keys(keys: &[Key]) {
    wait_for_hotkey_release();
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
            // 分小批发，每批之间 flush 会等一下，让目标程序来得及处理（2026-11-19：一次塞一长串，
            // 终端、微信、网页都会漏字）
            Key::Text(text) => {
                let mut sent = 0;
                let mut after_mark = false;
                for unit in text.encode_utf16() {
                    // 标点后面紧跟的字单独起一批：Windows Terminal 收到同一批里「标点＋下一个字」时，
                    // 后面那个字常被吞掉（2026-11-19 复现：82 字丢 6 个，全在「，。？」后面；一字一批 0 丢失）
                    if sent > 0 && (sent % BATCH_UNITS == 0 || after_mark) {
                        flush(&mut inputs);
                    }
                    inputs.push(key(0, unit, KEYEVENTF_UNICODE));
                    inputs.push(key(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
                    after_mark = is_mark(unit);
                    sent += 1;
                }
            }
            Key::Enter => inputs.extend(tap(VK_RETURN)),
            // 全选、复制、挪光标走 SendInput；微信这类 Qt 窗口的字和退格走窗口消息，会插到还没处理的按键前面，
            // 所以这几个发完都等目标程序处理好再往下（全选后再打字、Ctrl+End 后再退格）
            Key::SelectAll | Key::Copy | Key::End => {
                let letter = match k {
                    Key::SelectAll => Some(b'A'),
                    Key::Copy => Some(b'C'),
                    _ => None,
                };
                inputs.push(key(VK_CONTROL, 0, 0));
                match letter {
                    Some(letter) => inputs.extend(tap(u16::from(letter))),
                    None => inputs.extend(extended_tap(VK_END)),
                }
                inputs.push(key(VK_CONTROL, 0, KEYEVENTF_KEYUP));
                flush(&mut inputs);
                thread::sleep(Duration::from_millis(60));
            }
            Key::Right(count) => {
                for i in 0..*count {
                    if i > 0 && i % BATCH_UNITS == 0 {
                        flush(&mut inputs);
                    }
                    inputs.extend(extended_tap(VK_RIGHT));
                }
                flush(&mut inputs);
                thread::sleep(Duration::from_millis(30));
            }
            Key::Backspace(count) => {
                for i in 0..*count {
                    if i > 0 && i % BATCH_UNITS == 0 {
                        flush(&mut inputs);
                    }
                    inputs.extend(tap(VK_BACK));
                }
            }
        }
    }
    flush(&mut inputs);
}

/// 热键是 Alt/Ctrl/Shift 这类修饰键、还按着时，先等它松开再发键（最多 3 秒）：
/// 按着 Alt 打进去的字会变成菜单快捷键（主人 2026-11-24：按住右 Alt 不出字、编辑器的菜单栏来回切）
fn wait_for_hotkey_release() {
    let hotkey = HOTKEY.load(Ordering::Acquire);
    if !matches!(hotkey, 0x10..=0x12 | 0xA0..=0xA5) {
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while pressed(hotkey) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_CHAR, WM_KEYDOWN, WM_KEYUP};

    // 主人 2026-11-21：在翻译小窗里说话，打进去的英文被中文输入法当拼音接走、出来乱码。
    // 小窗里的文字、退格、换行都直接发 WM_CHAR 给输入框，不经过输入法
    #[test]
    fn the_translate_window_gets_plain_characters() {
        assert_eq!(
            window_messages(&Key::Text("Hi\n好".into()), true),
            [
                (WM_CHAR, 'H' as usize, 1),
                (WM_CHAR, 'i' as usize, 1),
                (WM_CHAR, 0x0D, 1),
                (WM_CHAR, '好' as usize, 1)
            ]
        );
        assert_eq!(
            window_messages(&Key::Backspace(2), true),
            [(WM_CHAR, 0x08, 1), (WM_CHAR, 0x08, 1)]
        );
        // Qt（微信）不认 WM_CHAR 里的退格，照旧按键
        assert_eq!(
            window_messages(&Key::Backspace(1), false),
            [
                (WM_KEYDOWN, VK_BACK as usize, 0x000e_0001),
                (WM_KEYUP, VK_BACK as usize, 0xc00e_0001_u32 as isize)
            ]
        );
        assert!(window_messages(&Key::Enter, true).is_empty());
    }
}
