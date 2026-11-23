use crate::install::{self, Adapter, Result};
use crate::status::Status;
use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
use xueer_ear_backend::engine::{self, Event, Source};
use xueer_ear_backend::{postprocess::Postprocess, settings::Settings};
use xueer_ear_bridge::ipc::{BridgeError, BridgeEvent, BridgeRequest};

static EXIT: AtomicBool = AtomicBool::new(false);
unsafe extern "system" fn console(_: u32) -> i32 {
    EXIT.store(true, Ordering::Release);
    1
}

type Output = Arc<Mutex<File>>;

#[derive(Clone, Copy)]
enum Backend {
    WeType,
    WeChat,
}

impl Backend {
    fn parse(value: &std::ffi::OsStr) -> Result<Self> {
        match value.to_str() {
            Some("wetype") => Ok(Self::WeType),
            Some("wechat") => Ok(Self::WeChat),
            _ => Err("识别后端须为 wetype 或 wechat".into()),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::WeType => "wetype",
            Self::WeChat => "wechat",
        }
    }

    fn preflight(self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        match self {
            Self::WeType => xueer_ear_backend::wetype::preflight_cancellable(cancelled),
            Self::WeChat => {
                let pid = install::wechat_pid()?;
                xueer_ear_backend::runtime_account::read(pid)?;
                Ok(())
            }
        }
    }

    fn run(
        self,
        source: Source,
        stop: Arc<AtomicBool>,
        mut output: impl FnMut(Event) -> Result<()>,
    ) -> Result<String> {
        // 定稿文字先过文字整理并记历史，再交给原来的上屏流程
        let post = postprocess();
        let output = move |event: Event| match event {
            Event::Speech(speech) => output(Event::Speech(post.apply(speech, now_ms()))),
            other => output(other),
        };
        match self {
            Self::WeType => engine::run_cloud(source, settings().0.engine, stop, output),
            Self::WeChat => engine::run(install::wechat_pid()?, source, stop, output),
        }
    }
}

/// 读设置。设置文件坏了（手改错、写到一半）不悄悄用默认值顶上（审查 R14）：
/// 记一行日志，返回错误让调用方告诉主人；这次先按默认值跑。
pub(crate) fn settings() -> (Settings, Option<String>) {
    match Settings::load_default() {
        Ok(settings) => (settings, None),
        Err(error) => {
            let reason = format!("设置文件读不了，这次先用默认设置：{error}");
            eprintln!("{reason}");
            (Settings::default(), Some(reason))
        }
    }
}

/// 每次唤起 Win+H 重新读设置和热词，改完下次生效（和麦克风、静音时间一致）。
pub(crate) fn postprocess() -> Postprocess {
    let settings = settings().0;
    let dir = std::env::var_os("LOCALAPPDATA")
        .map(|local| PathBuf::from(local).join("XueerEar"))
        .unwrap_or_default();
    let post = Postprocess::new(&settings, &dir);
    post.prune_history(now_ms());
    post
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn send(output: &Output, event: &BridgeEvent) -> Result<()> {
    let mut bytes = serde_json::to_vec(event)?;
    bytes.push(b'\n');
    output
        .lock()
        .map_err(|_| "结果通道锁不可用")?
        .write_all(&bytes)?;
    Ok(())
}

struct Active {
    since: Instant,
    reco: u64,
    generation: u64,
    stop: Arc<AtomicBool>,
    completed: Arc<AtomicBool>,
    worker: JoinHandle<()>,
}

fn verification_target() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    unsafe {
        let window = GetForegroundWindow();
        let mut class = [0_u16; 256];
        let count = GetClassNameW(window, class.as_mut_ptr(), class.len() as i32);
        let class = String::from_utf16_lossy(&class[..count.max(0) as usize]);
        if class == "XueerEarNativeVerification" {
            return true;
        }
        if class != "CASCADIA_HOSTING_WINDOW_CLASS" && !class.contains("wpf_native_smoke") {
            return false;
        }
        let mut title = [0_u16; 256];
        let count = GetWindowTextW(window, title.as_mut_ptr(), title.len() as i32);
        let title = String::from_utf16_lossy(&title[..count.max(0) as usize]);
        title.starts_with("WinH-Terminal-Test-") || title == "Win+H WPF 独立验证文本框"
    }
}

fn start(
    reco: u64,
    generation: u64,
    device: String,
    file: Option<PathBuf>,
    hold_wave: bool,
    output: Output,
    status: Status,
    panel_pid: u32,
    backend: Backend,
) -> Active {
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let completed = Arc::new(AtomicBool::new(false));
    let worker_completed = completed.clone();
    let worker = thread::spawn(move || {
        let _round = status.round();
        let verification = file.is_some();
        let source = file.map_or_else(
            || Source::Microphone((!device.is_empty()).then_some(device)),
            |file| {
                if hold_wave {
                    Source::WaveHold(file)
                } else {
                    Source::Wave(file)
                }
            },
        );
        let mut started = Instant::now();
        let mut utterance_offset = 0;
        let mut last_log = Instant::now() - Duration::from_secs(15);
        let result = (|| {
            let source = match source {
                Source::Microphone(native) => Source::Microphone(
                    xueer_ear_backend::settings::Settings::load_default()?
                        .microphone
                        .or(native),
                ),
                source => source,
            };
            if verification && !verification_target() {
                return Err("测试仅允许自有验证窗口".into());
            }
            let mut terminal = crate::terminal::Terminal::foreground(panel_pid)?;
            if terminal.is_some() {
                println!("终端[{generation}] 已启用流式修订");
            }
            let stopping = worker_stop.clone();
            backend.run(source, worker_stop, |event| {
                let event = match event {
                    Event::Input { name, muted, volume } => {
                        println!("采音[{generation}] 设备={name} 静音={muted:?} 音量={volume:?}");
                        status.microphone(name);
                        return Ok(());
                    }
                    Event::Activity(phase) => {
                        status.phase(phase);
                        println!("活动[{generation}] {phase}");
                        if matches!(phase, "connecting" | "paused") {
                            send(&output, &BridgeEvent::SpeechActivity {
                                reco,
                                generation,
                                speaking: phase == "connecting",
                                offset: (started.elapsed().as_nanos() / 100).min(u64::MAX as u128) as u64,
                            })?;
                        }
                        return Ok(());
                    }
                    Event::Progress { samples, peak, rms, pieces, responses, partials, finals, .. } => {
                        if !stopping.load(Ordering::Acquire) { if let Some(target)=terminal.as_mut() { target.poll(&stopping)?; } }
                        status.progress(peak, partials, finals);
                        if last_log.elapsed() >= Duration::from_secs(if started.elapsed().as_secs() < 10 { 1 } else { 15 }) {
                            println!("链路[{generation}] PCM={samples} peak={peak} rms={rms:.1} 上传={pieces} 回包={responses} partial={partials} final={finals}");
                            last_log = Instant::now();
                        }
                        return Ok(());
                    }
                    Event::Channel { name, fallback } => {
                        status.channel(name);
                        if fallback {
                            println!("通道[{generation}] 豆包不行，这句改走微信");
                        }
                        return Ok(());
                    }
                    Event::Started => {
                        println!("就绪[{generation}]");
                        started = Instant::now();
                        BridgeEvent::Started {
                            reco,
                            generation,
                            session: format!("{reco:x}-{generation:x}"),
                        }
                    }
                    Event::Speech(speech) => {
                        if let Some(target)=terminal.as_mut() {
                            if !stopping.load(Ordering::Acquire) { target.update(&speech.text,speech.final_result,&stopping)?; }
                            return Ok(());
                        }
                        let now = (started.elapsed().as_nanos() / 100).min(u64::MAX as u128) as u64;
                        let offset = utterance_offset;
                        let duration = now.saturating_sub(offset).max(1);
                        if speech.final_result {
                            utterance_offset = now;
                            BridgeEvent::Final {
                                reco,
                                generation,
                                text: speech.text,
                                offset,
                                duration,
                            }
                        } else {
                            BridgeEvent::Partial {
                                reco,
                                generation,
                                text: speech.text,
                                offset,
                                duration,
                            }
                        }
                    }
                };
                send(&output, &event)
            })
        })();
        // Error is terminal in the bridge too: it dispatches Canceled/Stopped
        // and Carbon can immediately start again from those callbacks. Publish
        // completion before either terminal event, not just before Stopped.
        worker_completed.store(true, Ordering::Release);
        if let Some(error) = result
            .as_ref()
            .err()
            .filter(|error| !crate::input_guard::expected_end(error.as_ref()))
        {
            eprintln!("识别失败：{error}");
            status.error(error.to_string());
            let _ = send(
                &output,
                &BridgeEvent::Error {
                    reco,
                    generation,
                    error: BridgeError {
                        message: error.to_string(),
                        code: 1,
                    },
                },
            );
        } else {
            if let Err(error) = result {
                // 带上触发来源和已听了几秒，方便对照「说着说着就停了」
                println!(
                    "正常结束[{generation}] {error}（来源 {}，{:.1} 秒）",
                    crate::terminal::last_interrupt(),
                    started.elapsed().as_secs_f32()
                );
            }
            status.phase("ready");
        }
        println!("结束[{generation}]");
        let _ = send(&output, &BridgeEvent::Stopped { reco, generation });
    });
    Active {
        since: Instant::now(),
        reco,
        generation,
        stop,
        completed,
        worker,
    }
}

/// 不接管 Win+H，只开快捷键听写，直到被要求退出。
/// `fallback` 有值表示本想接管但系统组件对不上（提示一次）；None 表示设置里本来就不接管。
fn hotkey_only(
    control: &crate::control::Control,
    status: &Status,
    fallback: Option<&str>,
) -> Result<()> {
    let (mut settings, broken) = settings();
    if let Some(reason) = &broken {
        status.error(reason.clone());
    }
    settings.hotkey_enabled = true;
    let hotkey = crate::hotkey::Hotkey::start(&settings, status.clone());
    match fallback {
        Some(reason) => {
            eprintln!("{reason}");
            crate::sound::play(crate::sound::Cue::Fallback, settings.sounds_enabled);
            status.error(reason.to_string());
        }
        None if broken.is_none() => status.phase("ready"),
        None => {}
    }
    control.ready();
    let mut heartbeat = Instant::now();
    while !EXIT.load(Ordering::Acquire) && !control.should_stop() {
        thread::sleep(Duration::from_millis(200));
        if heartbeat.elapsed() >= Duration::from_secs(5) {
            status.touch();
            heartbeat = Instant::now();
        }
    }
    drop(hotkey);
    control.stopping();
    status.restored();
    control.completed();
    Ok(())
}

pub fn run() -> Result<()> {
    let control = crate::control::Control::new()?;
    let mut arguments = std::env::args_os().skip(1);
    let command = arguments.next();
    if command.as_deref().is_some_and(|s| s != "run") {
        return Err(
            "用法：xueer-ear-host.exe run [--backend wetype|wechat] [--verify-wave FILE]".into(),
        );
    }
    let mut verification_file = None;
    let mut bridge_file = None;
    let mut once = false;
    let mut hold_wave = false;
    let mut backend = Backend::WeType;
    while let Some(flag) = arguments.next() {
        if flag == "--verify-wave" {
            verification_file = Some(PathBuf::from(arguments.next().ok_or("缺少WAV路径")?));
        } else if flag == "--bridge" {
            bridge_file = Some(PathBuf::from(arguments.next().ok_or("缺少适配器路径")?));
        } else if flag == "--once" {
            once = true;
        } else if flag == "--hold" {
            hold_wave = true;
        } else if flag == "--backend" {
            backend = Backend::parse(&arguments.next().ok_or("缺少识别后端")?)?;
        } else {
            return Err(
                "用法：xueer-ear-host.exe run [--backend wetype|wechat] [--verify-wave FILE] [--once]"
                    .into(),
            );
        }
    }
    let status = Status::new(backend.name())?;
    if let Some(path) = &verification_file {
        xueer_ear_backend::engine::validate_wave(path)?;
    }
    // Provider initialization must succeed before touching the Windows adapter.
    unsafe {
        SetConsoleCtrlHandler(Some(console), 1);
    }
    let preflight = backend.preflight(&|| control.should_stop() || EXIT.load(Ordering::Acquire));
    if control.should_stop() || EXIT.load(Ordering::Acquire) {
        status.restored();
        control.completed();
        return Ok(());
    }
    if let Err(error) = preflight {
        // No adapter has been installed: completion can be confirmed immediately.
        control.completed();
        return Err(error);
    }
    let dll = bridge_file.unwrap_or(std::env::current_exe()?.with_file_name(concat!(
        "xueer_ear_bridge-",
        env!("CARGO_PKG_VERSION"),
        ".dll"
    )));
    if !settings().0.winh_enabled && verification_file.is_none() {
        println!("不接管 Win+H（设置里没开），只开快捷键听写。");
        return hotkey_only(&control, &status, None);
    }
    let mut adapter = match Adapter::install(&dll) {
        Ok(adapter) => adapter,
        Err(error) if error.is::<crate::install::Unsupported>() => {
            return hotkey_only(&control, &status, Some(&error.to_string()));
        }
        Err(error) => return Err(error),
    };
    let output = Arc::new(Mutex::new(adapter.events.take().ok_or("结果通道不存在")?));
    let (sender, requests) = mpsc::sync_channel(32);
    let read = adapter.requests.try_clone()?;
    let reader = thread::spawn(move || {
        let mut reader = BufReader::new(read);
        loop {
            let mut line = String::new();
            // Take limits allocation even when the adapter sends an incomplete line.
            let count = std::io::Read::take(&mut reader, 65_536).read_line(&mut line);
            match count {
                Ok(0) | Err(_) => break,
                Ok(_) if !line.ends_with('\n') => break,
                Ok(_) => match serde_json::from_str::<BridgeRequest>(&line) {
                    Ok(request) => {
                        if sender.send(request).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                },
            }
        }
    });
    println!("已接入原生 Win+H；按 Ctrl+C 退出并恢复。");
    // 快捷键模式和 Win+H 并行可用；broker 退出时随 drop 停掉
    let _hotkey = crate::hotkey::Hotkey::start(&settings().0, status.clone());
    status.phase("ready");
    control.ready();
    let mut active: Option<Active> = None;
    let once_deadline = once.then(|| Instant::now() + Duration::from_secs(45));
    let mut heartbeat = Instant::now();
    let outcome: Result<()> = (|| {
        while !EXIT.load(Ordering::Acquire) && !control.should_stop() {
            if heartbeat.elapsed() >= Duration::from_secs(5) {
                status.touch();
                heartbeat = Instant::now();
            }
            if once_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                break;
            }
            if active.as_ref().is_some_and(|a| a.worker.is_finished()) {
                let _ = active.take().unwrap().worker.join();
                if once {
                    break;
                }
            }
            let request = match requests.recv_timeout(Duration::from_millis(100)) {
                Ok(request) => request,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            };
            match request {
                BridgeRequest::Start {
                    reco,
                    generation,
                    language,
                    device,
                } => {
                    // Completion may arrive while recv_timeout is waiting for a new Start.
                    if active.as_ref().is_some_and(|a| {
                        a.completed.load(Ordering::Acquire) || a.worker.is_finished()
                    }) {
                        let _ = active.take().unwrap().worker.join();
                    }
                    if active.is_some() {
                        println!("拒绝[{generation}] 上一轮尚未结束");
                        send(
                            &output,
                            &BridgeEvent::Error {
                                reco,
                                generation,
                                error: BridgeError {
                                    message: "上一轮听写尚未结束".into(),
                                    code: 1,
                                },
                            },
                        )?;
                        send(&output, &BridgeEvent::Stopped { reco, generation })?;
                    } else if !language.is_empty()
                        && !language.eq_ignore_ascii_case("zh-CN")
                        && !language.eq_ignore_ascii_case("zh")
                    {
                        send(
                            &output,
                            &BridgeEvent::Error {
                                reco,
                                generation,
                                error: BridgeError {
                                    message: "当前仅验证中文识别".into(),
                                    code: 1,
                                },
                            },
                        )?;
                        send(&output, &BridgeEvent::Stopped { reco, generation })?;
                    } else {
                        println!("接受[{generation}] 语言={language}");
                        active = Some(start(
                            reco,
                            generation,
                            device,
                            verification_file.clone(),
                            hold_wave,
                            output.clone(),
                            status.clone(),
                            adapter.pid,
                            backend,
                        ));
                    }
                }
                BridgeRequest::Stop {
                    reco, generation, ..
                } => {
                    let heard = active
                        .as_ref()
                        .map_or(0.0, |a| a.since.elapsed().as_secs_f32());
                    println!("停止[{generation}] 面板要求停止（本轮 {heard:.1} 秒）");
                    if let Some(active) = active
                        .as_ref()
                        .filter(|a| a.reco == reco && a.generation == generation)
                    {
                        active.stop.store(true, Ordering::Release);
                    }
                }
            }
        }
        Ok(())
    })();
    control.stopping();
    if let Some(active) = active.take() {
        active.stop.store(true, Ordering::Release);
        let _ = active.worker.join();
    }
    // EOF wakes the bridge event reader; no process or thread is terminated.
    drop(output);
    if once {
        println!("bridge_diagnostics={:#x}", adapter.diagnostics()?);
    }
    adapter.disable()?;
    status.restored();
    drop(adapter.requests);
    let _ = reader.join();
    control.completed();
    println!("已恢复原生识别。");
    outcome
}
