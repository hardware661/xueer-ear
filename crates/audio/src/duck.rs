//! 听写时把别的程序的声音调小（音乐、视频、游戏），这一轮说完恢复。
//!
//! - 只动音量合成器里各程序自己的那一条，不动系统总音量，也不发暂停 / 播放键；
//! - 只调此刻正在出声的（会话状态是「活动」），系统提示音和雪耳自己不调；
//! - 调之前先把每条原来的音量写进记录文件：雪耳中途崩了、断电了，下次启动按它恢复（`restore_leftover`）；
//! - 恢复时主人这期间自己改过音量的那条不动。
#[cfg(windows)]
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// 调到原来音量的几分之几
pub const DUCK_TO: f32 = 0.2;
/// 音量读回来和写进去的差在这以内算没人动过
const SAME: f32 = 0.01;

/// 调小过的一条：会话的实例标识、原来的音量、调小后的音量
#[derive(Debug, Clone, PartialEq)]
pub struct Saved {
    pub id: String,
    pub original: f32,
    pub ducked: f32,
}

/// 现在读到 `current`：还是我们调小后的样子就恢复成原来的音量；主人自己改过了就不动
pub fn restore_level(saved: &Saved, current: f32) -> Option<f32> {
    ((current - saved.ducked).abs() <= SAME).then_some(saved.original)
}

/// 记录文件：一行一条「原音量<TAB>调小后<TAB>会话标识」，`#` 开头是说明
pub fn format_saved(saved: &[Saved]) -> String {
    let mut out =
        String::from("# 雪耳听写时调小的声音（原音量、调小后、会话），下次启动按它恢复\n");
    for s in saved {
        out.push_str(&format!("{}\t{}\t{}\n", s.original, s.ducked, s.id));
    }
    out
}

pub fn parse_saved(file: &str) -> Vec<Saved> {
    file.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let original: f32 = parts.next()?.parse().ok()?;
            let ducked: f32 = parts.next()?.parse().ok()?;
            let id = parts.next()?.to_string();
            let valid = |v: f32| (0.0..=1.0).contains(&v);
            (valid(original) && valid(ducked) && !id.is_empty()).then_some(Saved {
                id,
                original,
                ducked,
            })
        })
        .collect()
}

/// 一轮听写里调小的那些；丢掉时恢复并删记录文件
#[cfg(windows)]
pub struct Ducking {
    path: PathBuf,
    saved: Arc<Mutex<Vec<Saved>>>,
    stop: Arc<AtomicBool>,
    watcher: Option<std::thread::JoinHandle<()>>,
}

/// 听写中途新出声的程序，多久查一次
#[cfg(windows)]
const WATCH: Duration = Duration::from_secs(1);

#[cfg(windows)]
impl Ducking {
    /// 调了几条
    pub fn count(&self) -> usize {
        lock(&self.saved).len()
    }

    /// 把正在出声的别的程序调小，之后中途新出声的也调小。先写记录文件再动音量。
    pub fn start(path: &Path) -> Result<Self, String> {
        let own = std::process::id();
        Self::start_where(path, WATCH, move |session| {
            session.active && !session.system && session.pid != own
        })
    }

    fn start_where(
        path: &Path,
        every: Duration,
        wanted: impl Fn(&sessions::Info) -> bool + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let wanted = Arc::new(wanted);
        let saved = Arc::new(Mutex::new(Vec::new()));
        {
            let (file, saved, wanted) = (path.to_path_buf(), saved.clone(), wanted.clone());
            sessions::on_com_thread(move || duck_new(&file, &saved, &*wanted))?;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let watcher = {
            let (file, saved, stop) = (path.to_path_buf(), saved.clone(), stop.clone());
            std::thread::spawn(move || {
                let watched = sessions::with_com(|| {
                    while !stopped_within(&stop, every) {
                        if let Err(error) = duck_new(&file, &saved, &*wanted) {
                            eprintln!("调小中途出声的程序失败：{error}");
                        }
                    }
                    Ok(())
                });
                if let Err(error) = watched {
                    eprintln!("看不了中途出声的程序：{error}");
                }
            })
        };
        Ok(Self {
            path: path.to_path_buf(),
            saved,
            stop,
            watcher: Some(watcher),
        })
    }
}

#[cfg(windows)]
impl Drop for Ducking {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(watcher) = self.watcher.take() {
            let _ = watcher.join();
        }
        let saved = std::mem::take(&mut *lock(&self.saved));
        if saved.is_empty() {
            return;
        }
        match restore(saved) {
            Ok(()) => {
                let _ = std::fs::remove_file(&self.path);
            }
            Err(error) => eprintln!("恢复别的程序的音量失败（下次启动再试）：{error}"),
        }
    }
}

/// 还没调过、又该调的会话调小：先把它们加进记录文件，再动音量
#[cfg(windows)]
fn duck_new(
    file: &Path,
    saved: &Mutex<Vec<Saved>>,
    wanted: &dyn Fn(&sessions::Info) -> bool,
) -> Result<(), String> {
    let mut list = lock(saved);
    let mut found = Vec::new();
    sessions::each(|info, volume| {
        if wanted(&info) && info.volume > 0. && !list.iter().any(|s| s.id == info.id) {
            found.push((
                Saved {
                    id: info.id.clone(),
                    original: info.volume,
                    ducked: info.volume * DUCK_TO,
                },
                volume.clone(),
            ));
        }
    })?;
    if found.is_empty() {
        return Ok(());
    }
    let mut next = list.clone();
    next.extend(found.iter().map(|(s, _)| s.clone()));
    write_atomically(file, &format_saved(&next))?;
    *list = next;
    for (s, volume) in &found {
        sessions::set(volume, s.ducked);
    }
    Ok(())
}

/// 睡 `every`，中途让停就提前醒；返回是不是让停了
#[cfg(windows)]
fn stopped_within(stop: &AtomicBool, every: Duration) -> bool {
    let until = std::time::Instant::now() + every;
    while std::time::Instant::now() < until {
        if stop.load(Ordering::Acquire) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    stop.load(Ordering::Acquire)
}

#[cfg(windows)]
fn lock(saved: &Mutex<Vec<Saved>>) -> std::sync::MutexGuard<'_, Vec<Saved>> {
    saved
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// 上次调小后没来得及恢复（崩溃、断电）：按记录文件恢复，然后删掉它
#[cfg(windows)]
pub fn restore_leftover(path: &Path) {
    let Ok(file) = std::fs::read_to_string(path) else {
        return;
    };
    match restore(parse_saved(&file)) {
        Ok(()) => {
            let _ = std::fs::remove_file(path);
        }
        Err(error) => eprintln!("恢复上次调小的音量失败：{error}"),
    }
}

#[cfg(windows)]
fn restore(saved: Vec<Saved>) -> Result<(), String> {
    if saved.is_empty() {
        return Ok(());
    }
    sessions::on_com_thread(move || {
        sessions::each(|info, volume| {
            if let Some(s) = saved.iter().find(|s| s.id == info.id)
                && let Some(level) = restore_level(s, info.volume)
            {
                sessions::set(volume, level);
            }
        })
    })
}

#[cfg(windows)]
fn write_atomically(path: &Path, content: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, content)
        .and_then(|()| std::fs::rename(&temporary, path))
        .map_err(|e| format!("记录文件写不了：{e}"))
}

/// Windows 音频会话：所有在用的播放设备上的每个会话
#[cfg(windows)]
mod sessions {
    use windows::{
        Win32::{
            Foundation::S_OK,
            Media::Audio::{
                AudioSessionStateActive, AudioSessionStateExpired, DEVICE_STATE_ACTIVE,
                IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator,
                ISimpleAudioVolume, MMDeviceEnumerator, eRender,
            },
            System::Com::{
                CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
                CoUninitialize,
            },
        },
        core::Interface,
    };

    pub struct Info {
        pub id: String,
        pub pid: u32,
        pub active: bool,
        pub system: bool,
        pub volume: f32,
    }

    /// COM 放在自己的线程上做（MTA），不管调用方线程是什么套间
    pub fn on_com_thread<T: Send + 'static>(
        work: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        std::thread::spawn(move || with_com(work))
            .join()
            .map_err(|_| "音量线程出错".to_string())?
    }

    /// 在当前线程进 MTA 做完 `work` 再出来（当前线程不能已经是 STA）
    pub fn with_com<T>(work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .map_err(|e| e.to_string())?;
        let result = work();
        unsafe { CoUninitialize() };
        result
    }

    /// 对每个没过期的会话调用 `f`。某个设备、某个会话读不了就跳过
    pub fn each(mut f: impl FnMut(Info, &ISimpleAudioVolume)) -> Result<(), String> {
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
                .map_err(|e| e.to_string())?;
        let devices = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }
            .map_err(|e| e.to_string())?;
        for d in 0..unsafe { devices.GetCount() }.unwrap_or(0) {
            let Ok(device) = (unsafe { devices.Item(d) }) else {
                continue;
            };
            let Ok(manager) =
                (unsafe { device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) })
            else {
                continue;
            };
            let Ok(list) = (unsafe { manager.GetSessionEnumerator() }) else {
                continue;
            };
            for i in 0..unsafe { list.GetCount() }.unwrap_or(0) {
                let Ok(control) = (unsafe { list.GetSession(i) }) else {
                    continue;
                };
                let Ok(state) = (unsafe { control.GetState() }) else {
                    continue;
                };
                if state == AudioSessionStateExpired {
                    continue;
                }
                let (Ok(details), Ok(volume)) = (
                    control.cast::<IAudioSessionControl2>(),
                    control.cast::<ISimpleAudioVolume>(),
                ) else {
                    continue;
                };
                let Some(id) = instance_id(&details) else {
                    continue;
                };
                let Ok(level) = (unsafe { volume.GetMasterVolume() }) else {
                    continue;
                };
                let info = Info {
                    id,
                    pid: unsafe { details.GetProcessId() }.unwrap_or(0),
                    active: state == AudioSessionStateActive,
                    system: unsafe { details.IsSystemSoundsSession() } == S_OK,
                    volume: level,
                };
                f(info, &volume);
            }
        }
        Ok(())
    }

    fn instance_id(details: &IAudioSessionControl2) -> Option<String> {
        let raw = unsafe { details.GetSessionInstanceIdentifier() }.ok()?;
        let id = unsafe { raw.to_string() }.ok();
        unsafe { CoTaskMemFree(Some(raw.0.cast())) };
        id.filter(|id| !id.is_empty() && !id.contains(['\t', '\n', '\r']))
    }

    pub fn set(volume: &ISimpleAudioVolume, level: f32) {
        if let Err(error) = unsafe { volume.SetMasterVolume(level.clamp(0., 1.), std::ptr::null()) }
        {
            eprintln!("改音量失败：{error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(id: &str, original: f32) -> Saved {
        Saved {
            id: id.into(),
            original,
            ducked: original * DUCK_TO,
        }
    }

    #[test]
    fn the_record_file_reads_back_what_was_written() {
        let list = vec![
            saved("{0.0.0.00000000}.{abc}|\\Device\\chrome.exe%b{1}", 0.8),
            saved("{0.0.0.00000000}.{def}|game.exe", 1.0),
        ];
        assert_eq!(parse_saved(&format_saved(&list)), list);
    }

    #[test]
    fn broken_lines_in_the_record_file_are_skipped() {
        let file = "# 说明\n0.5\t0.1\tok\nnot a line\n2\t0.1\ttoo loud\n0.5\t0.1\t\n0.4\tx\tbad\n";
        assert_eq!(
            parse_saved(file),
            [Saved {
                id: "ok".into(),
                original: 0.5,
                ducked: 0.1
            }]
        );
    }

    #[test]
    fn a_volume_the_owner_changed_meanwhile_is_left_alone() {
        let s = saved("x", 0.6);
        assert_eq!(restore_level(&s, 0.12), Some(0.6));
        assert_eq!(restore_level(&s, 0.125), Some(0.6));
        // 主人这期间自己调过
        assert_eq!(restore_level(&s, 0.5), None);
        assert_eq!(restore_level(&s, 0.), None);
    }

    /// 两个真机测试都用本进程自己的音频会话，排队跑
    #[cfg(windows)]
    static LIVE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// 只调本测试进程自己放的一路声音（静音），不碰别的程序
    #[cfg(windows)]
    #[test]
    fn ducking_lowers_our_own_test_stream_and_puts_it_back() {
        let _live = LIVE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(_stream) = silent_stream::Stream::start() else {
            eprintln!("没有播放设备（比如 CI 的虚拟机），跳过");
            return;
        };
        let dir = std::env::temp_dir().join(format!("xueer-duck-{}", std::process::id()));
        let path = dir.join("ducking.txt");
        let own = std::process::id();
        let ducking = Ducking::start_where(&path, Duration::from_secs(60), move |s| {
            s.active && s.pid == own
        })
        .unwrap();
        assert_eq!(ducking.count(), 1, "测试进程放的那一路应该被调小");
        assert!(path.exists(), "调之前先写记录文件");
        let level = own_volume().unwrap();
        assert!((level - DUCK_TO).abs() < 0.01, "调小后 {level}");
        drop(ducking);
        assert!((own_volume().unwrap() - 1.).abs() < 0.01, "恢复原音量");
        assert!(!path.exists(), "恢复后删掉记录文件");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[test]
    fn a_sound_that_starts_during_dictation_is_lowered_too() {
        let _live = LIVE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = std::env::temp_dir().join(format!("xueer-duck-late-{}", std::process::id()));
        let path = dir.join("ducking.txt");
        let own = std::process::id();
        if !silent_stream::available() {
            eprintln!("没有播放设备（比如 CI 的虚拟机），跳过");
            return;
        }
        let ducking = Ducking::start_where(&path, Duration::from_millis(100), move |s| {
            s.active && s.pid == own
        })
        .unwrap();
        assert_eq!(ducking.count(), 0, "开始时测试进程还没出声");
        let _stream = silent_stream::Stream::start().expect("刚才还有播放设备");
        let until = std::time::Instant::now() + Duration::from_secs(3);
        while ducking.count() == 0 && std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(ducking.count(), 1, "中途出声的也要调小");
        assert!((own_volume().unwrap() - DUCK_TO).abs() < 0.01);
        drop(ducking);
        assert!((own_volume().unwrap() - 1.).abs() < 0.01, "恢复原音量");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    fn own_volume() -> Option<f32> {
        let own = std::process::id();
        sessions::on_com_thread(move || {
            let mut found = None;
            sessions::each(|info, _| {
                if info.pid == own {
                    found = Some(info.volume);
                }
            })?;
            Ok(found)
        })
        .ok()
        .flatten()
    }

    /// 在默认播放设备上放一路静音，让测试进程有一个「活动」的音频会话
    #[cfg(windows)]
    mod silent_stream {
        use windows::Win32::{
            Media::Audio::{
                AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, IAudioClient,
                IAudioRenderClient, IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender,
            },
            System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx},
        };

        pub struct Stream {
            client: IAudioClient,
        }

        /// 有没有默认播放设备（GitHub 的 Windows 虚拟机上没有）
        pub fn available() -> bool {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .and_then(|enumerator| enumerator.GetDefaultAudioEndpoint(eRender, eConsole))
                    .is_ok()
            }
        }

        impl Stream {
            /// 没有默认播放设备时是 None
            pub fn start() -> Option<Self> {
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                    let enumerator: IMMDeviceEnumerator =
                        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
                    let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
                    let client: IAudioClient = device.Activate(CLSCTX_ALL, None).unwrap();
                    let format = client.GetMixFormat().unwrap();
                    client
                        .Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 10_000_000, 0, format, None)
                        .unwrap();
                    let render: IAudioRenderClient = client.GetService().unwrap();
                    let frames = client.GetBufferSize().unwrap();
                    render.GetBuffer(frames).unwrap();
                    render
                        .ReleaseBuffer(frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)
                        .unwrap();
                    client.Start().unwrap();
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    Some(Self { client })
                }
            }
        }

        impl Drop for Stream {
            fn drop(&mut self) {
                unsafe {
                    let _ = self.client.Stop();
                }
            }
        }
    }
}
