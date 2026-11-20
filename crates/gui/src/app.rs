//! Native Win32 control panel. Only our broker executable changes backend state.
#![allow(unsafe_op_in_unsafe_fn)]
use crate::{
    model::{Status, now_ms},
    view,
};
use std::{
    fs::File,
    io::Read,
    mem,
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
    ptr,
    sync::mpsc,
    thread,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{
        LibraryLoader::GetModuleHandleW,
        SystemServices::{SS_ENDELLIPSIS, SS_LEFT},
        Threading::*,
    },
    UI::{
        Controls::{DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_NOFOCUSRECT, ODS_SELECTED},
        HiDpi::*,
        Input::KeyboardAndMouse::{EnableWindow, IsWindowEnabled},
        Shell::*,
        WindowsAndMessaging::*,
    },
};

const CLASS: &str = "XueerEar.ControlPanel";
const APP_NAME: &str = "雪耳";
const TRAY: u32 = WM_APP + 1;
const DONE: u32 = WM_APP + 2;
const UPDATE_READY: u32 = WM_APP + 3;
const UPDATE_DOWNLOADED: u32 = WM_APP + 4;
const OPEN: usize = 101;
const START: usize = 102;
const STOP: usize = 103;
const EXIT: usize = 104;
const TOGGLE: usize = 105;
const MINIMIZE: usize = 106;
const CLOSE: usize = 107;
const GITHUB: usize = 108;
const UPDATE_DOWNLOAD: usize = 109;
const UPDATE_SKIP: usize = 110;
const UPDATE_CLOSE: usize = 111;
/// 一键更新装好后，安装包用 `--updated` 重新打开雪耳
const UPDATED: usize = 112;
const UPDATE_TITLE: usize = 222;
const UPDATE_BODY: usize = 223;
const APP_TITLE: usize = 221;
const LABEL_STATUS: usize = 203;
const LABEL_ERROR: usize = 206;
const LABEL_MIC_TITLE: usize = 210;
const LABEL_SILENCE: usize = 211;
const LABEL_HOTKEY: usize = 212;
const LABEL_MIC_VALUE: usize = 216;
const LABEL_SILENCE_VALUE: usize = 217;
const LABEL_TEXT: usize = 224;
const LABEL_TEXT_VALUE: usize = 225;
const LABEL_HOTKEY_VALUE: usize = 226;
const LABEL_HOTWORDS: usize = 227;
const LABEL_HOTWORDS_VALUE: usize = 228;
const LABEL_HISTORY: usize = 229;
const LABEL_HISTORY_VALUE: usize = 230;
const LABEL_TRANSLATE: usize = 231;
const LABEL_TRANSLATE_VALUE: usize = 232;
/// 分组小标题：「听写」「文字」
const GROUP_DICTATION: usize = 233;
const GROUP_TEXT: usize = 234;
/// 右边带小箭头、点了出菜单的那几个按钮
const VALUE_BUTTONS: [usize; 7] = [
    LABEL_HOTKEY_VALUE,
    LABEL_MIC_VALUE,
    LABEL_SILENCE_VALUE,
    LABEL_HOTWORDS_VALUE,
    LABEL_HISTORY_VALUE,
    LABEL_TRANSLATE_VALUE,
    LABEL_TEXT_VALUE,
];
/// 热词表最大多少字节（和后台读它时的上限一致）
const HOTWORDS_LIMIT: u64 = 256 * 1024;
const HOTWORDS_HEADER: &str = "# 雪耳热词表：一行一个词，读音像的识别结果会改成这个词。\r\n# 固定的错法写成「错=对」，例如 Cloud Code=Claude Code。# 开头是注释。\r\n";
// COLORREF 是 BGR；深灰，和 view 的主色一致
const GREEN: u32 = 0x002e2c2c;
const INK: u32 = 0x00251e18;
const MUTED: u32 = 0x00938e8e;

fn data_dir() -> Result<PathBuf, String> {
    std::env::var_os("LOCALAPPDATA")
        .map(|root| PathBuf::from(root).join("XueerEar"))
        .ok_or_else(|| "LOCALAPPDATA 不可用".to_owned())
}

/// 用记事本打开数据目录里的文件；热词表不存在时先建一个带说明的空表，历史不存在就提示。
unsafe fn open_data_file(name: &str) -> Result<(), String> {
    let dir = data_dir()?;
    let path = dir.join(name);
    if !path.exists() {
        let header = match name {
            "hotwords.txt" => HOTWORDS_HEADER,
            "translate.json" => {
                "{\r\n  \"wetype_enabled\": true,\r\n  \"azure_key\": \"\",\r\n  \"azure_region\": \"eastasia\",\r\n  \"google_key\": \"\",\r\n  \"zhipu_key\": \"\"\r\n}\r\n"
            }
            _ => return Err("还没有历史记录".to_owned()),
        };
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(&path, header).map_err(|e| e.to_string())?;
    }
    let result = ShellExecuteW(
        ptr::null_mut(),
        wide("open").as_ptr(),
        wide("notepad.exe").as_ptr(),
        wide(&format!("\"{}\"", path.display())).as_ptr(),
        ptr::null(),
        SW_SHOWNORMAL,
    );
    if result as isize <= 32 {
        return Err(format!("无法打开 {name}"));
    }
    Ok(())
}

/// 文件最后 `bytes` 字节（日志可能很大），切在完整的一行开头
fn tail(path: &std::path::Path, bytes: u64) -> Option<String> {
    use std::io::{Seek, SeekFrom};
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(bytes))).ok()?;
    let mut data = Vec::new();
    file.read_to_end(&mut data).ok()?;
    let text = String::from_utf8_lossy(&data).into_owned();
    Some(if len > bytes {
        text.split_once('\n')
            .map_or(text.clone(), |(_, rest)| rest.to_owned())
    } else {
        text
    })
}

/// 热词表的内容；不存在返回空，太大或读不了返回错误。
fn read_hotwords() -> Result<String, String> {
    let path = data_dir()?.join("hotwords.txt");
    match std::fs::metadata(&path) {
        Err(_) => Ok(String::new()),
        Ok(meta) if meta.len() > HOTWORDS_LIMIT => Err("热词表太大了，请先删掉一些".to_owned()),
        Ok(_) => std::fs::read_to_string(&path).map_err(|e| format!("无法读取热词表：{e}")),
    }
}

/// 往热词表加一行。返回 false 表示表里已经有了。
fn save_hotword(line: &str) -> Result<bool, String> {
    let dir = data_dir()?;
    let mut file = read_hotwords()?;
    if file.is_empty() {
        file = HOTWORDS_HEADER.to_owned();
    }
    let Some(updated) = xueer_ear_textflow::add_hotword_line(&file, line) else {
        return Ok(false);
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let temp = dir.join(format!("hotwords.{}.tmp", std::process::id()));
    std::fs::write(&temp, updated)
        .and_then(|()| std::fs::rename(&temp, dir.join("hotwords.txt")))
        .map_err(|e| format!("无法保存热词表：{e}"))?;
    Ok(true)
}

/// 最近说过的几句（新的在前）。读不了就当没有。
fn recent_history(count: usize, settings: &xueer_ear_backend::settings::Settings) -> Vec<String> {
    let Ok(dir) = data_dir() else {
        return Vec::new();
    };
    let cutoff =
        crate::model::now_ms().saturating_sub(u64::from(settings.history_keep_days) * 86_400_000);
    xueer_ear_history::History::new(
        dir.join("history.jsonl"),
        xueer_ear_history::Settings {
            enabled: settings.history_enabled,
            keep_days: settings.history_keep_days,
        },
    )
    .recent(count)
    .map(|entries| {
        entries
            .into_iter()
            .filter(|entry| entry.at_ms >= cutoff)
            .map(|entry| entry.text)
            .collect()
    })
    .unwrap_or_default()
}

/// 把一段文字放进剪贴板。
pub(crate) unsafe fn copy_text(owner: HWND, text: &str) -> Result<(), String> {
    use windows_sys::Win32::System::{
        DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
    };
    const CF_UNICODETEXT: u32 = 13;
    let units = wide(text);
    // 剪贴板常被剪贴板工具（截图、历史类软件）短暂占着：隔一下多试几次再放弃
    let mut opened = false;
    for _ in 0..10 {
        if OpenClipboard(owner) != 0 {
            opened = true;
            break;
        }
        thread::sleep(std::time::Duration::from_millis(30));
    }
    if !opened {
        return Err("剪贴板正被别的程序占用".to_owned());
    }
    let memory = GlobalAlloc(GMEM_MOVEABLE, units.len() * 2);
    let mut copied = false;
    if !memory.is_null() {
        let target = GlobalLock(memory) as *mut u16;
        if !target.is_null() {
            ptr::copy_nonoverlapping(units.as_ptr(), target, units.len());
            GlobalUnlock(memory);
            EmptyClipboard();
            // 成功后内存归剪贴板所有
            copied = !SetClipboardData(CF_UNICODETEXT, memory as HANDLE).is_null();
        }
        if !copied {
            GlobalFree(memory);
        }
    }
    CloseClipboard();
    if copied {
        Ok(())
    } else {
        Err("无法复制".to_owned())
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn wide_os(value: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().chain(Some(0)).collect()
}
fn scale(value: i32, dpi: u32) -> i32 {
    (value * dpi as i32 + 48) / 96
}

unsafe fn work_area(window: HWND) -> RECT {
    let monitor = if window.is_null() {
        let mut point = mem::zeroed();
        GetCursorPos(&mut point);
        MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST)
    } else {
        MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST)
    };
    let mut info: MONITORINFO = mem::zeroed();
    info.cbSize = mem::size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(monitor, &mut info);
    info.rcWork
}
fn fitted_dpi(actual: u32, work: RECT) -> u32 {
    actual
        .min(((work.bottom - work.top - 32).max(1) * 96 / view::HEIGHT) as u32)
        .min(((work.right - work.left - 32).max(1) * 96 / view::WIDTH) as u32)
        .max(72)
}

pub(crate) unsafe fn ui_font(window: HWND) -> &'static str {
    unsafe extern "system" fn found(
        _: *const LOGFONTW,
        _: *const TEXTMETRICW,
        _: u32,
        state: LPARAM,
    ) -> i32 {
        unsafe {
            *(state as *mut bool) = true;
        }
        0
    }
    let dc = GetDC(window);
    for name in ["HarmonyOS Sans SC", "MiSans", "Microsoft YaHei UI"] {
        let mut request: LOGFONTW = mem::zeroed();
        request.lfCharSet = DEFAULT_CHARSET;
        for (to, from) in request.lfFaceName.iter_mut().zip(name.encode_utf16()) {
            *to = from;
        }
        let mut exists = false;
        EnumFontFamiliesExW(
            dc,
            &request,
            Some(found),
            (&mut exists as *mut bool) as isize,
            0,
        );
        if exists {
            ReleaseDC(window, dc);
            return name;
        }
    }
    ReleaseDC(window, dc);
    "Segoe UI"
}

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                CloseHandle(self.0);
            }
        }
    }
}

fn event(name: &str, signaled: bool) -> bool {
    unsafe {
        let handle = Handle(OpenEventW(
            SYNCHRONIZATION_SYNCHRONIZE,
            0,
            wide(name).as_ptr(),
        ));
        !handle.0.is_null() && (!signaled || WaitForSingleObject(handle.0, 0) == WAIT_OBJECT_0)
    }
}

fn alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe {
        let process = Handle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid));
        if process.0.is_null() {
            return false;
        }
        let mut code = 0;
        GetExitCodeProcess(process.0, &mut code) != 0 && code == STILL_ACTIVE as u32
    }
}

#[derive(Clone, Copy)]
enum Action {
    Start,
    Stop,
    Exit,
}
impl Action {
    fn argument(self) -> &'static str {
        if matches!(self, Self::Start) {
            "start"
        } else {
            "stop"
        }
    }
}
struct Completed {
    action: Action,
    result: Result<(), String>,
}

fn backend_command(action: Action) -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("无法定位程序目录")?
        .join("xueer-ear-host.exe");
    // An uninstalled GUI may still exit when no broker owns the restoration event.
    if matches!(action, Action::Exit)
        && !executable.is_file()
        && !event("Local\\XueerEar.Stop", false)
    {
        return Ok(());
    }
    // A detached descendant can retain a redirected pipe on Windows. Wait for
    // the command process itself and read a bounded file after it exits.
    let folder = std::env::var_os("LOCALAPPDATA")
        .map(|root| PathBuf::from(root).join("XueerEar"))
        .ok_or("无法定位应用数据目录")?;
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let path = folder.join(format!("gui-command-{}.log", std::process::id()));
    let log = File::create(&path).map_err(|e| e.to_string())?;
    let exit = Command::new(&executable)
        .arg(action.argument())
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log)
        .status()
        .map_err(|e| format!("无法运行识别服务：{e}"))?;
    let mut bytes = Vec::new();
    if let Ok(file) = File::open(&path) {
        let _ = file.take(4096).read_to_end(&mut bytes);
    }
    let _ = std::fs::remove_file(path);
    if exit.success() {
        return Ok(());
    }
    let message = String::from_utf8_lossy(&bytes);
    let message = message.trim();
    Err(if message.is_empty() {
        format!("操作失败（{exit}）")
    } else {
        message.chars().take(300).collect()
    })
}

struct App {
    hwnd: HWND,
    dpi: u32,
    font_family: &'static str,
    controls: Vec<(usize, HWND)>,
    fonts: Vec<HFONT>,
    /// 页面浅灰底
    page: HBRUSH,
    /// 卡片白
    card: HBRUSH,
    icon: HICON,
    github_icon: HICON,
    taskbar_created: u32,
    path: Option<PathBuf>,
    status: Option<Status>,
    ready: bool,
    installed: bool,
    healthy: bool,
    busy: Option<Action>,
    pending_exit: bool,
    /// 改了要重启服务才生效的设置：先停，停完再开
    pending_start: bool,
    command_error: String,
    /// 「已复制」这类一句话提示，过几秒自己消失
    notice: Option<(String, std::time::Instant)>,
    /// 热词表的修改时间和「N 个」摘要，文件变了才重读
    hotwords_seen: Option<(Option<std::time::SystemTime>, String)>,
    /// 离屏拍照核对外观：不建托盘图标、不查更新
    snapshot: bool,
    sender: mpsc::Sender<Completed>,
    receiver: mpsc::Receiver<Completed>,
    tray_ok: bool,
    tray_tip: String,
    _renderer: Option<view::Renderer>,
    animate: bool,
    settings: xueer_ear_backend::settings::Settings,
    microphones: Vec<(String, String)>,
    update_sender: mpsc::Sender<(bool, Result<Option<crate::update::Release>, String>)>,
    update_receiver: mpsc::Receiver<(bool, Result<Option<crate::update::Release>, String>)>,
    update_preferences: crate::update::Preferences,
    update_release: Option<crate::update::Release>,
    update_checking: bool,
    update_visible: bool,
    update_dismissed: Option<String>,
    update_error: String,
    download_sender: mpsc::Sender<Result<PathBuf, String>>,
    download_receiver: mpsc::Receiver<Result<PathBuf, String>>,
    update_downloading: bool,
    /// 第一次打开：后台服务自动开好，状态那里写「点一下右 Alt 说话试试」，说过一次话就不再写
    first_run: bool,
}

impl App {
    unsafe fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        let (update_sender, update_receiver) = mpsc::channel();
        let (download_sender, download_receiver) = mpsc::channel();
        let (update_preferences, update_error) = match crate::update::Preferences::load() {
            Ok(value) => (value, String::new()),
            Err(error) => (Default::default(), error),
        };
        let (settings, settings_error) = match xueer_ear_backend::settings::Settings::load_default()
        {
            Ok(value) => (value, String::new()),
            Err(error) => (Default::default(), format!("无法读取设置：{error}")),
        };
        let module = GetModuleHandleW(ptr::null());
        let mut icon = LoadIconW(module, 100_usize as *const u16);
        if icon.is_null() {
            icon = LoadIconW(ptr::null_mut(), IDI_APPLICATION);
        }
        Self {
            hwnd: ptr::null_mut(),
            dpi: 96,
            font_family: "Microsoft YaHei UI",
            controls: Vec::new(),
            fonts: Vec::new(),
            page: CreateSolidBrush(view::BACKGROUND),
            card: CreateSolidBrush(view::CARD),
            icon,
            github_icon: LoadIconW(module, 101_usize as *const u16),
            taskbar_created: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
            path: std::env::var_os("LOCALAPPDATA")
                .map(|p| PathBuf::from(p).join("XueerEar/status.json")),
            status: None,
            ready: false,
            installed: false,
            healthy: false,
            busy: None,
            pending_exit: false,
            pending_start: false,
            command_error: settings_error,
            notice: None,
            hotwords_seen: None,
            snapshot: false,
            sender,
            receiver,
            tray_ok: false,
            tray_tip: String::new(),
            _renderer: view::Renderer::new().ok(),
            animate: false,
            settings,
            microphones: Vec::new(),
            update_sender,
            update_receiver,
            update_preferences,
            update_release: None,
            update_checking: false,
            update_visible: false,
            update_dismissed: None,
            update_error,
            download_sender,
            download_receiver,
            update_downloading: false,
            first_run: false,
        }
    }

    fn control(&self, id: usize) -> HWND {
        self.controls
            .iter()
            .find(|(key, _)| *key == id)
            .map_or(ptr::null_mut(), |(_, hwnd)| *hwnd)
    }

    unsafe fn label(&mut self, id: usize, text: &str) {
        let control = CreateWindowExW(
            0,
            wide("STATIC").as_ptr(),
            wide(text).as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_LEFT | SS_ENDELLIPSIS,
            0,
            0,
            0,
            0,
            self.hwnd,
            id as HMENU,
            GetModuleHandleW(ptr::null()),
            ptr::null(),
        );
        self.controls.push((id, control));
    }

    unsafe fn create(&mut self) -> Result<(), String> {
        for (id, text) in [
            (APP_TITLE, APP_NAME),
            (LABEL_STATUS, "正在读取状态"),
            (LABEL_ERROR, ""),
            (GROUP_DICTATION, "听写"),
            (GROUP_TEXT, "文字"),
            (LABEL_HOTKEY, "快捷键"),
            (LABEL_MIC_TITLE, "麦克风"),
            (LABEL_SILENCE, "静音收句"),
            (LABEL_HOTWORDS, "热词"),
            (LABEL_HISTORY, "历史"),
            (LABEL_TRANSLATE, "翻译"),
            (LABEL_TEXT, "更多设置"),
            (UPDATE_TITLE, ""),
            (UPDATE_BODY, ""),
        ] {
            self.label(id, text);
        }
        for (id, text, style) in [
            (TOGGLE, "开启后台服务", BS_OWNERDRAW),
            (MINIMIZE, "最小化", BS_OWNERDRAW),
            (CLOSE, "关闭窗口", BS_OWNERDRAW),
            (GITHUB, "GitHub · hardware661", BS_OWNERDRAW),
            (LABEL_MIC_VALUE, "系统默认", BS_OWNERDRAW),
            (LABEL_SILENCE_VALUE, "3 秒", BS_OWNERDRAW),
            (LABEL_HOTKEY_VALUE, "右 Alt", BS_OWNERDRAW),
            (LABEL_HOTWORDS_VALUE, "添加", BS_OWNERDRAW),
            (LABEL_HISTORY_VALUE, "最近 7 天", BS_OWNERDRAW),
            (LABEL_TRANSLATE_VALUE, "打开", BS_OWNERDRAW),
            (LABEL_TEXT_VALUE, "", BS_OWNERDRAW),
            (UPDATE_DOWNLOAD, "下载更新", BS_OWNERDRAW),
            (UPDATE_SKIP, "跳过此版本", BS_OWNERDRAW),
            (UPDATE_CLOSE, "关闭更新提示", BS_OWNERDRAW),
        ] {
            let control = CreateWindowExW(
                0,
                wide("BUTTON").as_ptr(),
                wide(text).as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | style as u32,
                0,
                0,
                0,
                0,
                self.hwnd,
                id as HMENU,
                GetModuleHandleW(ptr::null()),
                ptr::null(),
            );
            self.controls.push((id, control));
        }
        if self.controls.iter().any(|(_, h)| h.is_null()) {
            return Err("无法创建控制面板".into());
        }
        self.dpi = fitted_dpi(GetDpiForWindow(self.hwnd).max(96), work_area(self.hwnd));
        self.font_family = ui_font(self.hwnd);
        self.layout();
        self.update_bubble(false);
        let _ = self.read_microphones();
        SendMessageW(self.hwnd, WM_CHANGEUISTATE, 1 | (3 << 16), 0);
        self.tray(NIM_ADD);
        SetTimer(self.hwnd, 1, 750, None);
        if !self.snapshot {
            SetTimer(self.hwnd, 3, 6 * 60 * 60 * 1000, None);
            SetTimer(self.hwnd, 4, 3000, None);
        }
        self.refresh();
        Ok(())
    }

    unsafe fn set_text(&self, id: usize, text: &str) {
        let control = self.control(id);
        let mut previous = vec![0_u16; GetWindowTextLengthW(control).max(0) as usize + 1];
        GetWindowTextW(control, previous.as_mut_ptr(), previous.len() as i32);
        if previous != wide(text) {
            SetWindowTextW(control, wide(text).as_ptr());
        }
    }

    unsafe fn layout(&mut self) {
        let old = mem::take(&mut self.fonts);
        for (size, weight) in [(14, 400), (12, 400), (18, 500), (13, 400), (15, 500)] {
            let font = CreateFontW(
                -scale(size, self.dpi),
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                DEFAULT_PITCH as u32,
                wide(self.font_family).as_ptr(),
            );
            self.fonts.push(font);
        }
        // 一行 44 高；卡片位置和 view::CARDS 对应
        let row = |card: usize, i: i32| view::CARDS[card].0 as i32 + i * view::ROW as i32;
        let mut positions = vec![
            (LABEL_STATUS, 92, 70, 220, 22, 4),
            (LABEL_ERROR, 92, 95, 220, 16, 1),
            (TOGGLE, 324, 74, 44, 28, 3),
            (GROUP_DICTATION, 32, 150, 120, 16, 1),
            (GROUP_TEXT, 32, 316, 120, 16, 1),
            (UPDATE_TITLE, 136, 61, 216, 22, 4),
            (UPDATE_BODY, 136, 88, 216, 18, 1),
            (UPDATE_DOWNLOAD, 136, 118, 85, 30, 3),
            (UPDATE_SKIP, 227, 118, 105, 30, 3),
            (UPDATE_CLOSE, 354, 53, 24, 24, 3),
        ];
        for (card, i, label, value) in [
            (1, 0, LABEL_HOTKEY, LABEL_HOTKEY_VALUE),
            (1, 1, LABEL_MIC_TITLE, LABEL_MIC_VALUE),
            (1, 2, LABEL_SILENCE, LABEL_SILENCE_VALUE),
            (2, 0, LABEL_HOTWORDS, LABEL_HOTWORDS_VALUE),
            (2, 1, LABEL_HISTORY, LABEL_HISTORY_VALUE),
            (2, 2, LABEL_TRANSLATE, LABEL_TRANSLATE_VALUE),
            (3, 0, LABEL_TEXT, LABEL_TEXT_VALUE),
        ] {
            positions.push((label, 32, row(card, i) + 11, 110, 22, 0));
            positions.push((value, 148, row(card, i) + 4, 228, 36, 0));
        }
        for (id, x, y, w, h, font) in positions {
            let control = self.control(id);
            SetWindowPos(
                control,
                ptr::null_mut(),
                scale(x, self.dpi),
                scale(y, self.dpi),
                scale(w, self.dpi),
                scale(h, self.dpi),
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            SendMessageW(control, WM_SETFONT, self.fonts[font] as usize, 0);
        }
        for (id, x, y, w, h) in [
            (APP_TITLE, 50, 10, 80, 24),
            (MINIMIZE, 312, 0, 44, 42),
            (CLOSE, 356, 0, 44, 42),
            (GITHUB, 272, 3, 36, 36),
        ] {
            SetWindowPos(
                self.control(id),
                ptr::null_mut(),
                scale(x, self.dpi),
                scale(y, self.dpi),
                scale(w, self.dpi),
                scale(h, self.dpi),
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            SendMessageW(self.control(id), WM_SETFONT, self.fonts[4] as usize, 0);
        }
        for font in old {
            if !font.is_null() {
                DeleteObject(font);
            }
        }
        InvalidateRect(self.hwnd, ptr::null(), 1);
    }

    unsafe fn tray(&mut self, action: u32) {
        if self.snapshot {
            return;
        }
        let mut data: NOTIFYICONDATAW = mem::zeroed();
        data.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = self.hwnd;
        data.uID = 1;
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = TRAY;
        data.hIcon = self.icon;
        let tip = format!("雪耳 · {}", self.headline());
        if action == NIM_MODIFY && tip == self.tray_tip {
            return;
        }
        self.tray_tip = tip.clone();
        let tip = wide(&tip);
        let len = tip.len().min(data.szTip.len() - 1);
        data.szTip[..len].copy_from_slice(&tip[..len]);
        let success = Shell_NotifyIconW(action, &data) != 0;
        if action == NIM_ADD {
            self.tray_ok = success;
        }
    }

    /// 大字状态：正在切换 > 操作失败 > 后台服务报的阶段
    fn headline(&self) -> &'static str {
        if let Some(action) = self.busy {
            match action {
                Action::Start => "正在开启",
                Action::Stop => "正在关闭",
                Action::Exit => "正在退出",
            }
        } else if !self.command_error.is_empty() {
            "操作失败"
        } else {
            self.status.as_ref().map_or(
                if self.ready {
                    "等待状态"
                } else {
                    "未开启"
                },
                |s| s.headline(now_ms(), self.healthy, self.ready),
            )
        }
    }

    /// 在状态下面显示一句话提示，过几秒自己消失
    unsafe fn say(&mut self, text: &str) {
        self.command_error.clear();
        self.notice = Some((text.to_owned(), std::time::Instant::now()));
        self.refresh();
    }

    /// 「热词」那一行右边显示的字；热词表文件变了才重读
    fn hotword_summary(&mut self) -> String {
        let modified = data_dir()
            .ok()
            .and_then(|dir| std::fs::metadata(dir.join("hotwords.txt")).ok())
            .and_then(|meta| meta.modified().ok());
        if let Some((seen, summary)) = &self.hotwords_seen {
            if *seen == modified {
                return summary.clone();
            }
        }
        let summary = crate::prefs::hotword_summary(&read_hotwords().unwrap_or_default());
        self.hotwords_seen = Some((modified, summary.clone()));
        summary
    }

    unsafe fn refresh(&mut self) {
        let previous = (self.ready, self.installed, self.animate);
        self.ready = event("Local\\XueerEar.Ready", true);
        self.installed = event("Local\\XueerEar.Stop", false);
        let read = self
            .path
            .as_deref()
            .and_then(|path| Status::read(path).ok());
        if let Some(status) = read {
            self.status = Some(status);
        }
        self.healthy = self.status.as_ref().is_some_and(|s| alive(s.pid));
        let headline = self.headline();
        if self.first_run
            && self
                .status
                .as_ref()
                .is_some_and(|s| matches!(s.phase.as_str(), "armed" | "recognizing" | "connecting"))
        {
            self.first_run = false;
        }
        if self.first_run && headline == "待机" {
            let hint = format!(
                "点一下{}说话试试",
                crate::prefs::key_name(self.settings.hotkey_vk)
            );
            self.set_text(LABEL_STATUS, &hint);
        } else {
            self.set_text(LABEL_STATUS, headline);
        }
        let microphone = self
            .settings
            .microphone
            .as_ref()
            .map(|id| {
                self.microphones
                    .iter()
                    .find(|(key, _)| key == id)
                    .map(|(_, name)| name.as_str())
                    .unwrap_or("已选麦克风")
            })
            .unwrap_or("系统默认");
        self.set_text(LABEL_MIC_VALUE, microphone);
        self.set_text(
            LABEL_SILENCE_VALUE,
            &format!("{} 秒", self.settings.silence_seconds),
        );
        self.set_text(
            LABEL_HOTKEY_VALUE,
            &crate::prefs::key_name(self.settings.hotkey_vk),
        );
        let hotwords = self.hotword_summary();
        self.set_text(LABEL_HOTWORDS_VALUE, &hotwords);
        self.set_text(
            LABEL_HISTORY_VALUE,
            &crate::prefs::history_summary(&self.settings),
        );
        let keys = xueer_ear_backend::translate::local_keys();
        self.set_text(
            LABEL_TRANSLATE_VALUE,
            &crate::prefs::translate_summary(&keys),
        );
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= std::time::Duration::from_secs(4))
        {
            self.notice = None;
        }
        // 状态下面那一行：出错原因（红字）优先，其次是「已复制」这类提示（灰字）
        let error = if !self.command_error.is_empty() {
            self.command_error.as_str()
        } else {
            self.status
                .as_ref()
                .filter(|s| s.phase == "error")
                .map_or("", |s| s.last_error.as_str())
        };
        let line = match &self.notice {
            Some((text, _)) if error.is_empty() => text.as_str(),
            _ => error,
        };
        self.set_text(LABEL_ERROR, &line.chars().take(180).collect::<String>());
        self.show_error_line();
        let available = self.busy.is_none();
        let mut buttons_changed = previous.1 != self.installed;
        if (IsWindowEnabled(self.control(TOGGLE)) != 0) != available {
            EnableWindow(self.control(TOGGLE), available as i32);
            buttons_changed = true;
        }
        self.set_text(
            TOGGLE,
            if self.installed {
                "关闭后台服务"
            } else {
                "开启后台服务"
            },
        );
        self.animate = self.ready
            && self.healthy
            && self
                .status
                .as_ref()
                .is_some_and(|s| s.phase == "recognizing" && s.peak > 0 && !s.stale(now_ms()));
        let mut animations = 1_i32;
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut animations as *mut i32).cast(),
            0,
        );
        if self.animate
            && animations != 0
            && IsWindowVisible(self.hwnd) != 0
            && IsIconic(self.hwnd) == 0
        {
            SetTimer(self.hwnd, 2, 60, None);
        } else {
            KillTimer(self.hwnd, 2);
        }
        if previous.0 != self.ready || previous.2 != self.animate {
            let rect = RECT {
                left: scale(28, self.dpi),
                top: scale(60, self.dpi),
                right: scale(232, self.dpi),
                bottom: scale(116, self.dpi),
            };
            InvalidateRect(self.hwnd, &rect, 0);
        }
        if buttons_changed {
            InvalidateRect(self.control(TOGGLE), ptr::null(), 0);
        }
        self.tray(if self.tray_ok { NIM_MODIFY } else { NIM_ADD });
    }

    unsafe fn begin(&mut self, action: Action) {
        if self.busy.is_some() {
            if matches!(action, Action::Exit) {
                self.pending_exit = true;
            }
            return;
        }
        self.busy = Some(action);
        self.command_error.clear();
        self.refresh();
        let sender = self.sender.clone();
        let hwnd = self.hwnd as usize;
        thread::spawn(move || {
            let result = backend_command(action);
            let _ = sender.send(Completed { action, result });
            PostMessageW(hwnd as HWND, DONE, 0, 0);
        });
    }

    unsafe fn completed(&mut self) {
        while let Ok(done) = self.receiver.try_recv() {
            self.busy = None;
            match done.result {
                Ok(()) if matches!(done.action, Action::Exit) => {
                    DestroyWindow(self.hwnd);
                    return;
                }
                Ok(()) => self.command_error.clear(),
                Err(error) => {
                    self.command_error = error;
                    self.pending_start = false;
                    self.show();
                }
            }
        }
        if self.pending_start && self.busy.is_none() && !self.pending_exit {
            self.pending_start = false;
            self.begin(Action::Start);
            return;
        }
        if self.pending_exit && self.busy.is_none() {
            self.pending_exit = false;
            self.begin(Action::Exit);
            return;
        }
        self.refresh();
    }

    unsafe fn show(&self) {
        ShowWindow(self.hwnd, SW_RESTORE);
        SetForegroundWindow(self.hwnd);
    }

    unsafe fn check_updates(&mut self, manual: bool) {
        if self.update_checking {
            return;
        }
        self.update_checking = true;
        self.set_text(GITHUB, "GitHub · 正在检查更新");
        let sender = self.update_sender.clone();
        let hwnd = self.hwnd as usize;
        thread::spawn(move || {
            let result = crate::update::latest(env!("CARGO_PKG_VERSION"));
            if sender.send((manual, result)).is_ok() {
                PostMessageW(hwnd as HWND, UPDATE_READY, 0, 0);
            }
        });
    }

    unsafe fn updates_ready(&mut self) {
        while let Ok((manual, result)) = self.update_receiver.try_recv() {
            self.update_checking = false;
            match result {
                Ok(Some(release)) => {
                    self.update_error.clear();
                    let notify = manual
                        || self.update_preferences.skipped_version.as_ref()
                            != Some(&release.version)
                            && self.update_dismissed.as_ref() != Some(&release.version);
                    self.set_text(UPDATE_TITLE, &format!("发现新版本 v{}", release.version));
                    self.set_text(
                        UPDATE_BODY,
                        &format!("当前版本 v{}", env!("CARGO_PKG_VERSION")),
                    );
                    self.update_release = Some(release);
                    if notify {
                        self.update_bubble(true);
                    }
                }
                Ok(None) => {
                    self.update_error.clear();
                    self.update_release = None;
                    if manual {
                        self.set_text(UPDATE_TITLE, "当前已是最新版本");
                        self.set_text(UPDATE_BODY, &format!("雪耳 v{}", env!("CARGO_PKG_VERSION")));
                        self.update_bubble(true);
                    } else {
                        self.update_bubble(false);
                    }
                }
                Err(error) => {
                    self.update_error = error;
                    if manual {
                        self.update_release = None;
                        self.set_text(UPDATE_TITLE, "暂时无法检查更新");
                        self.set_text(UPDATE_BODY, "请稍后再试");
                        self.update_bubble(true);
                    }
                }
            }
            self.set_text(
                GITHUB,
                if self.update_release.is_some() {
                    "GitHub · 有可用更新"
                } else {
                    "GitHub · hardware661/xueer-ear"
                },
            );
            InvalidateRect(self.control(GITHUB), ptr::null(), 0);
        }
    }

    unsafe fn open_release_page(&self, release: &crate::update::Release) {
        ShellExecuteW(
            self.hwnd,
            wide("open").as_ptr(),
            wide(&release.url).as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        );
    }

    /// 「下载更新」：安装版在后台下载安装包、核对校验值，下好就静默安装（安装包会让这个雪耳退出，
    /// 装完用 `--updated` 重新打开）；便携版没有安装目录可换，照旧打开发布页
    unsafe fn download_update(&mut self) {
        let Some(release) = self.update_release.clone() else {
            return;
        };
        if !crate::update::installed() {
            self.open_release_page(&release);
            return;
        }
        if self.update_downloading {
            return;
        }
        self.update_downloading = true;
        self.set_text(UPDATE_TITLE, &format!("正在下载 v{}…", release.version));
        self.set_text(UPDATE_BODY, "下好后自动安装，装完雪耳会自己重开");
        for id in [UPDATE_DOWNLOAD, UPDATE_SKIP] {
            ShowWindow(self.control(id), SW_HIDE);
        }
        self.update_bubble_redraw();
        let sender = self.download_sender.clone();
        let hwnd = self.hwnd as usize;
        thread::spawn(move || {
            let result = crate::update::download_installer(&release);
            if sender.send(result).is_ok() {
                PostMessageW(hwnd as HWND, UPDATE_DOWNLOADED, 0, 0);
            }
        });
    }

    unsafe fn update_downloaded(&mut self) {
        while let Ok(result) = self.download_receiver.try_recv() {
            self.update_downloading = false;
            let launched = result.and_then(|setup| {
                let folder = setup
                    .parent()
                    .map(std::path::Path::as_os_str)
                    .unwrap_or_default();
                let code = ShellExecuteW(
                    self.hwnd,
                    wide("open").as_ptr(),
                    wide_os(setup.as_os_str()).as_ptr(),
                    wide("/SILENT /SUPPRESSMSGBOXES /NORESTART /SELFUPDATE=1").as_ptr(),
                    wide_os(folder).as_ptr(),
                    SW_SHOWNORMAL,
                );
                if code as isize > 32 {
                    Ok(())
                } else {
                    Err(format!("打不开安装包（错误 {}）", code as isize))
                }
            });
            match launched {
                Ok(()) => {
                    let version = self.update_release.as_ref().map(|r| r.version.clone());
                    self.set_text(
                        UPDATE_TITLE,
                        &format!("正在安装 v{}", version.unwrap_or_default()),
                    );
                    self.set_text(UPDATE_BODY, "装好后雪耳会自己重新打开");
                }
                Err(error) => {
                    self.set_text(UPDATE_TITLE, "没能自动更新，已打开发布页");
                    self.set_text(UPDATE_BODY, &error);
                    self.update_error = error;
                    if let Some(release) = self.update_release.clone() {
                        self.open_release_page(&release);
                    }
                }
            }
            if !self.update_visible {
                self.update_bubble(true);
                for id in [UPDATE_DOWNLOAD, UPDATE_SKIP] {
                    ShowWindow(self.control(id), SW_HIDE);
                }
            }
            self.update_bubble_redraw();
        }
    }

    /// 一键更新装好后重新打开：告诉主人已经是新版本了
    unsafe fn show_updated(&mut self) {
        self.update_release = None;
        self.set_text(
            UPDATE_TITLE,
            &format!("已更新到 v{}", env!("CARGO_PKG_VERSION")),
        );
        self.set_text(UPDATE_BODY, "后台服务已经开好，可以接着说话");
        self.update_bubble(true);
    }

    unsafe fn update_bubble(&mut self, show: bool) {
        self.update_visible = show;
        // Native STATIC children paint above the parent's buffered canvas.
        // Hide the labels covered by the popover instead of cutting holes in it.
        for id in [LABEL_STATUS, TOGGLE, GROUP_DICTATION] {
            ShowWindow(self.control(id), if show { SW_HIDE } else { SW_SHOW });
        }
        self.show_error_line();
        for id in [UPDATE_TITLE, UPDATE_BODY, UPDATE_CLOSE] {
            ShowWindow(self.control(id), if show { SW_SHOW } else { SW_HIDE });
        }
        for id in [UPDATE_DOWNLOAD, UPDATE_SKIP] {
            ShowWindow(
                self.control(id),
                if show && self.update_release.is_some() {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
        }
        self.update_bubble_redraw();
    }

    unsafe fn update_bubble_redraw(&self) {
        let rect = RECT {
            left: scale(112, self.dpi),
            top: scale(38, self.dpi),
            right: scale(392, self.dpi),
            bottom: scale(180, self.dpi),
        };
        InvalidateRect(self.hwnd, &rect, 0);
    }

    /// 状态下面那行字：有字才露出来（空的时候让出位置给波形），更新气泡盖着时藏起来
    unsafe fn show_error_line(&self) {
        let control = self.control(LABEL_ERROR);
        let visible = !self.update_visible && GetWindowTextLengthW(control) > 0;
        if (IsWindowVisible(control) != 0) != visible {
            ShowWindow(control, if visible { SW_SHOW } else { SW_HIDE });
            InvalidateRect(self.hwnd, ptr::null(), 0);
        }
    }

    unsafe fn dismiss_update(&mut self, skip: bool) {
        if let Some(release) = self.update_release.as_ref() {
            if skip {
                let previous = self
                    .update_preferences
                    .skipped_version
                    .replace(release.version.clone());
                if let Err(error) = self.update_preferences.save() {
                    self.update_preferences.skipped_version = previous;
                    self.update_error = error;
                    self.set_text(UPDATE_BODY, "跳过设置保存失败，请重试");
                    return;
                }
            }
            self.update_dismissed = Some(release.version.clone());
        }
        if skip {
            self.update_release = None;
            InvalidateRect(self.control(GITHUB), ptr::null(), 0);
        }
        self.update_bubble(false);
    }

    unsafe fn github_menu(&mut self) {
        let mut rect = mem::zeroed();
        GetWindowRect(self.control(GITHUB), &mut rect);
        let choices = vec![
            (1, "打开 GitHub".to_owned(), false),
            (2, "检查更新".to_owned(), false),
        ];
        match crate::menu::popup(
            self.hwnd,
            rect.left,
            rect.bottom + scale(4, self.dpi),
            &choices,
            self.fonts[3],
            self.dpi,
        ) {
            1 => self.action(GITHUB),
            2 => self.check_updates(true),
            _ => {}
        }
    }

    /// 托盘右键菜单：和控制面板里的菜单同一套样子（圆角高亮、同样的字体），不用系统默认的灰菜单。
    unsafe fn menu(&mut self) {
        let choices = crate::prefs::tray_choices(
            self.headline(),
            self.settings.hotkey_vk,
            self.installed,
            self.busy.is_some(),
            OPEN,
            TOGGLE,
            EXIT,
        );
        let mut point: POINT = mem::zeroed();
        GetCursorPos(&mut point);
        // 不先把自己设成前台，点菜单外面时菜单不会消失
        SetForegroundWindow(self.hwnd);
        let selected = crate::menu::popup_at(
            self.hwnd,
            point.x,
            point.y,
            &choices,
            self.fonts[3],
            self.dpi,
        );
        if selected != crate::prefs::NOTHING {
            self.action(selected);
        }
    }

    unsafe fn action(&mut self, id: usize) {
        match id {
            OPEN => self.show(),
            START if !self.installed => self.begin(Action::Start),
            STOP if self.installed => self.begin(Action::Stop),
            TOGGLE => self.begin(if self.installed {
                Action::Stop
            } else {
                Action::Start
            }),
            MINIMIZE => {
                ShowWindow(self.hwnd, SW_MINIMIZE);
            }
            CLOSE => {
                PostMessageW(self.hwnd, WM_CLOSE, 0, 0);
            }
            GITHUB => {
                let result = ShellExecuteW(
                    self.hwnd,
                    wide("open").as_ptr(),
                    wide(env!("CARGO_PKG_HOMEPAGE")).as_ptr(),
                    ptr::null(),
                    ptr::null(),
                    SW_SHOWNORMAL,
                );
                if result as isize <= 32 {
                    self.command_error = "无法打开 GitHub".to_owned();
                    self.refresh();
                }
            }
            UPDATE_DOWNLOAD => self.download_update(),
            UPDATED => self.show_updated(),
            UPDATE_SKIP => self.dismiss_update(true),
            UPDATE_CLOSE => self.dismiss_update(false),
            2 if self.update_visible => self.dismiss_update(false),
            LABEL_MIC_VALUE => self.settings_menu(true),
            LABEL_SILENCE_VALUE => self.settings_menu(false),
            LABEL_HOTKEY_VALUE => self.hotkey_menu(),
            LABEL_HOTWORDS_VALUE => self.hotwords_menu(),
            LABEL_TRANSLATE_VALUE => self.translate_menu(),
            LABEL_HISTORY_VALUE => self.history_menu(),
            LABEL_TEXT_VALUE => self.text_menu(),
            EXIT => self.begin(Action::Exit),
            _ => {}
        }
    }

    fn read_microphones(&mut self) -> Result<(), String> {
        let devices =
            thread::spawn(|| xueer_ear_audio::microphone::devices().map_err(|e| e.to_string()))
                .join()
                .map_err(|_| "无法读取麦克风".to_owned())??;
        self.microphones.clear();
        for (_, device) in devices {
            if !self.microphones.iter().any(|(id, _)| *id == device.id) {
                self.microphones.push((device.id, device.name));
            }
        }
        Ok(())
    }

    unsafe fn settings_menu(&mut self, microphone: bool) {
        if microphone {
            if let Err(error) = self.read_microphones() {
                self.command_error = error;
                self.refresh();
                return;
            }
        }
        let choices = if microphone {
            let mut choices = vec![(
                1000,
                "系统默认".to_owned(),
                self.settings.microphone.is_none(),
            )];
            choices.extend(self.microphones.iter().enumerate().map(|(i, (id, name))| {
                (
                    1001 + i,
                    name.clone(),
                    self.settings.microphone.as_ref() == Some(id),
                )
            }));
            choices
        } else {
            [1, 2, 3, 5, 10]
                .into_iter()
                .enumerate()
                .map(|(i, s)| {
                    (
                        1001 + i,
                        format!("{s} 秒"),
                        self.settings.silence_seconds == s,
                    )
                })
                .collect()
        };
        let mut rect = mem::zeroed();
        GetWindowRect(
            self.control(if microphone {
                LABEL_MIC_VALUE
            } else {
                LABEL_SILENCE_VALUE
            }),
            &mut rect,
        );
        let selected = crate::menu::popup(
            self.hwnd,
            rect.left,
            rect.bottom + scale(4, self.dpi),
            &choices,
            self.fonts[3],
            self.dpi,
        );
        if selected == 0 {
            return;
        }
        let mut candidate = self.settings.clone();
        if microphone {
            candidate.microphone = if selected == 1000 {
                None
            } else {
                self.microphones
                    .get(selected as usize - 1001)
                    .map(|(id, _)| id.clone())
            };
        } else if let Some(seconds) = [1, 2, 3, 5, 10].get(selected as usize - 1001) {
            candidate.silence_seconds = *seconds;
        }
        self.apply(candidate);
    }

    /// 在某个按钮下面弹菜单，返回选中的命令号（没选是 0）
    unsafe fn popup_under(&self, button: usize, choices: &[(usize, String, bool)]) -> usize {
        let mut rect = mem::zeroed();
        GetWindowRect(self.control(button), &mut rect);
        // 值是右对齐的灰字，菜单也贴着它的右边往下弹
        crate::menu::popup_right(
            self.hwnd,
            rect.right,
            rect.bottom + scale(2, self.dpi),
            choices,
            self.fonts[3],
            self.dpi,
        )
    }

    /// 保存设置。改的是要重启后台服务才生效的项，而服务正开着，就自动停一下再开。
    unsafe fn apply(&mut self, candidate: xueer_ear_backend::settings::Settings) {
        match candidate.save_default() {
            Ok(()) => {
                let restart = crate::prefs::needs_restart(&self.settings, &candidate);
                self.settings = candidate;
                self.command_error.clear();
                if restart && self.installed && self.busy.is_none() {
                    self.pending_start = true;
                    self.begin(Action::Stop);
                    return;
                }
            }
            Err(error) => self.command_error = format!("无法保存设置：{error}"),
        }
        self.refresh();
    }

    /// 换快捷键。后台服务自己会发现设置变了，不用重启。
    unsafe fn hotkey_menu(&mut self) {
        let selected = self.popup_under(
            LABEL_HOTKEY_VALUE,
            &crate::prefs::key_choices(&self.settings),
        );
        if let Some(candidate) = crate::prefs::pick_key(&self.settings, selected) {
            self.apply(candidate);
        }
    }

    /// 热词：弹小对话框加一个、用记事本打开整张表，或者开关自带词表。
    unsafe fn hotwords_menu(&mut self) {
        let choices = crate::prefs::hotword_choices(&self.settings);
        match self.popup_under(LABEL_HOTWORDS_VALUE, &choices) {
            crate::prefs::BUILTIN_HOTWORDS => {
                let mut candidate = self.settings.clone();
                candidate.text.builtin_hotwords = !candidate.text.builtin_hotwords;
                self.apply(candidate);
            }
            crate::prefs::ADD_HOTWORD => {
                let Some(line) = crate::hotword_dialog::ask(self.hwnd, self.fonts[3], self.dpi)
                else {
                    return;
                };
                match save_hotword(&line) {
                    Ok(true) => self.say(&format!("已添加：{}", line.replace('=', " → "))),
                    Ok(false) => self.say("热词表里已经有了"),
                    Err(error) => {
                        self.command_error = error;
                        self.refresh();
                    }
                }
            }
            crate::prefs::OPEN_HOTWORDS => {
                if let Err(error) = open_data_file("hotwords.txt") {
                    self.command_error = error;
                    self.refresh();
                }
            }
            _ => {}
        }
    }

    /// 翻译：打开翻译小窗，或者用记事本打开翻译设置。
    unsafe fn translate_menu(&mut self) {
        let keys = xueer_ear_backend::translate::local_keys();
        let result = match self.popup_under(
            LABEL_TRANSLATE_VALUE,
            &crate::prefs::translate_choices(&keys),
        ) {
            crate::prefs::OPEN_TRANSLATOR => std::env::current_exe()
                .map_err(|e| e.to_string())
                .and_then(|exe| {
                    std::process::Command::new(exe)
                        .arg("--translate")
                        .spawn()
                        .map(|_| ())
                        .map_err(|e| format!("打不开翻译小窗：{e}"))
                }),
            crate::prefs::OPEN_TRANSLATE_KEYS => open_data_file("translate.json"),
            _ => Ok(()),
        };
        if let Err(error) = result {
            self.command_error = error;
            self.refresh();
        }
    }

    /// 历史：列最近说过的几句，点一句就复制；也能用记事本打开全部。
    unsafe fn history_menu(&mut self) {
        if !self.settings.history_enabled {
            self.say("历史记录已关闭");
            return;
        }
        let texts = recent_history(crate::prefs::HISTORY_SHOWN, &self.settings);
        let selected =
            self.popup_under(LABEL_HISTORY_VALUE, &crate::prefs::history_choices(&texts));
        let result = if selected == crate::prefs::OPEN_HISTORY {
            open_data_file("history.jsonl")
        } else if let Some(text) = crate::prefs::history_pick(&texts, selected) {
            copy_text(self.hwnd, text).map(|()| self.say("已复制，可以粘贴了"))
        } else {
            Ok(())
        };
        if let Err(error) = result {
            self.command_error = error;
            self.refresh();
        }
    }

    /// 更多设置：文字整理开关、提示音、接管 Win+H、历史保留天数、一次最长听多久。
    unsafe fn text_menu(&mut self) {
        let selected = self.popup_under(LABEL_TEXT_VALUE, &crate::prefs::choices(&self.settings));
        if selected == crate::prefs::COPY_DIAGNOSTICS {
            let text = self.diagnostics();
            match copy_text(self.hwnd, &text) {
                Ok(()) => self.say("诊断信息已复制，提 issue 时贴上就行"),
                Err(error) => {
                    self.command_error = error;
                    self.refresh();
                }
            }
            return;
        }
        if let Some(candidate) = crate::prefs::toggle(&self.settings, selected) {
            let admin = candidate.run_as_admin;
            let changed = admin != self.settings.run_as_admin;
            self.apply(candidate);
            if changed && self.settings.run_as_admin == admin {
                self.say("重新打开雪耳后生效");
            }
        }
    }

    /// 「复制诊断信息」：版本、系统、几项设置、后台状态、service.log 最后几行（不含说过的话）
    unsafe fn diagnostics(&self) -> String {
        let log = data_dir()
            .ok()
            .and_then(|dir| tail(&dir.join("service.log"), 64 * 1024))
            .unwrap_or_default();
        let headline = self.headline();
        let status = match self.status.as_ref() {
            Some(s) if !s.last_error.is_empty() => format!("{headline}（{}）", s.last_error),
            _ => headline.to_owned(),
        };
        let folders: Vec<(&str, String)> = ["TEMP", "LOCALAPPDATA", "APPDATA", "USERPROFILE"]
            .into_iter()
            .filter_map(|name| std::env::var(name).ok().map(|path| (name, path)))
            .collect();
        let folders: Vec<(&str, &str)> = folders.iter().map(|(n, p)| (*n, p.as_str())).collect();
        crate::prefs::diagnostics(
            env!("CARGO_PKG_VERSION"),
            &crate::system::windows_version(),
            &self.settings,
            &status,
            &log,
            &folders,
        )
    }

    unsafe fn draw_button(&self, draw: &DRAWITEMSTRUCT) {
        let width = draw.rcItem.right - draw.rcItem.left;
        let height = draw.rcItem.bottom - draw.rcItem.top;
        let dc = CreateCompatibleDC(draw.hDC);
        let bitmap = CreateCompatibleBitmap(draw.hDC, width, height);
        if dc.is_null() || bitmap.is_null() {
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            if !dc.is_null() {
                DeleteDC(dc);
            }
            self.draw_button_content(draw);
            return;
        }
        let old = SelectObject(dc, bitmap);
        let mut buffered = *draw;
        buffered.hDC = dc;
        self.draw_button_content(&buffered);
        BitBlt(
            draw.hDC,
            draw.rcItem.left,
            draw.rcItem.top,
            width,
            height,
            dc,
            0,
            0,
            SRCCOPY,
        );
        SelectObject(dc, old);
        DeleteObject(bitmap);
        DeleteDC(dc);
    }

    unsafe fn draw_button_content(&self, draw: &DRAWITEMSTRUCT) {
        let disabled = draw.itemState & ODS_DISABLED != 0;
        let pressed = draw.itemState & ODS_SELECTED != 0;
        // 标题栏上的按钮是页面灰底，其余都在白卡片（或更新气泡）上
        let on_page = matches!(draw.CtlID as usize, GITHUB | MINIMIZE | CLOSE);
        FillRect(
            draw.hDC,
            &draw.rcItem,
            if on_page { self.page } else { self.card },
        );
        if draw.CtlID == GITHUB as u32 {
            DrawIconEx(
                draw.hDC,
                (draw.rcItem.right - scale(18, self.dpi)) / 2,
                (draw.rcItem.bottom - scale(18, self.dpi)) / 2,
                self.github_icon,
                scale(18, self.dpi),
                scale(18, self.dpi),
                0,
                ptr::null_mut(),
                DI_NORMAL,
            );
            if self.update_release.as_ref().is_some_and(|release| {
                self.update_preferences.skipped_version.as_ref() != Some(&release.version)
            }) {
                view::update_badge(draw.hDC, self.dpi);
            }
            return;
        }
        if draw.CtlID == UPDATE_CLOSE as u32 {
            view::close_glyph(draw.hDC, self.dpi, draw.rcItem.right, draw.rcItem.bottom);
            return;
        }
        if VALUE_BUTTONS.contains(&(draw.CtlID as usize)) {
            view::value_button(
                draw.hDC,
                self.dpi,
                draw.rcItem.right,
                draw.rcItem.bottom,
                pressed,
            );
            SetBkMode(draw.hDC, TRANSPARENT as i32);
            SetTextColor(draw.hDC, MUTED);
            SelectObject(draw.hDC, self.fonts[0]);
            let mut rect = draw.rcItem;
            rect.left += scale(8, self.dpi);
            rect.right -= scale(20, self.dpi);
            let mut text = [0u16; 512];
            let len = GetWindowTextW(draw.hwndItem, text.as_mut_ptr(), 512);
            DrawTextW(
                draw.hDC,
                text.as_ptr(),
                len,
                &mut rect,
                DT_RIGHT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
            return;
        }
        if matches!(draw.CtlID as usize, MINIMIZE | CLOSE) {
            if pressed {
                let b = CreateSolidBrush(0x00ebe5e5);
                FillRect(draw.hDC, &draw.rcItem, b);
                DeleteObject(b);
            }
            let center_x = draw.rcItem.right / 2;
            let center_y = draw.rcItem.bottom / 2;
            let pen = CreatePen(PS_SOLID, scale(1, self.dpi).max(1), INK);
            let old = SelectObject(draw.hDC, pen);
            let radius = scale(5, self.dpi);
            MoveToEx(
                draw.hDC,
                center_x - radius,
                center_y
                    - if draw.CtlID == CLOSE as u32 {
                        radius
                    } else {
                        0
                    },
                ptr::null_mut(),
            );
            LineTo(
                draw.hDC,
                center_x + radius,
                center_y
                    + if draw.CtlID == CLOSE as u32 {
                        radius
                    } else {
                        0
                    },
            );
            if draw.CtlID == CLOSE as u32 {
                MoveToEx(
                    draw.hDC,
                    center_x - radius,
                    center_y + radius,
                    ptr::null_mut(),
                );
                LineTo(draw.hDC, center_x + radius, center_y - radius);
            }
            SelectObject(draw.hDC, old);
            DeleteObject(pen);
            if draw.itemState & (ODS_FOCUS | ODS_NOFOCUSRECT) == ODS_FOCUS {
                let mut r = draw.rcItem;
                InflateRect(&mut r, -4, -4);
                DrawFocusRect(draw.hDC, &r);
            }
            return;
        }
        let toggle = draw.CtlID == TOGGLE as u32;
        let selected = if draw.CtlID == UPDATE_DOWNLOAD as u32 {
            true
        } else if draw.CtlID == UPDATE_SKIP as u32 {
            false
        } else {
            self.installed
        };
        view::button(
            draw.hDC,
            self.dpi,
            draw.rcItem.right,
            draw.rcItem.bottom,
            toggle,
            selected,
            disabled,
            pressed,
        );
        SetBkMode(draw.hDC, TRANSPARENT as i32);
        SetTextColor(
            draw.hDC,
            if disabled {
                MUTED
            } else if selected {
                GREEN
            } else {
                INK
            },
        );
        SelectObject(draw.hDC, self.fonts[3]);
        let mut rect = draw.rcItem;
        let mut text = [0_u16; 128];
        let len = GetWindowTextW(draw.hwndItem, text.as_mut_ptr(), text.len() as i32);
        if !toggle {
            DrawTextW(
                draw.hDC,
                text.as_ptr(),
                len,
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
        }
        if draw.itemState & (ODS_FOCUS | ODS_NOFOCUSRECT) == ODS_FOCUS {
            InflateRect(&mut rect, -scale(4, self.dpi), -scale(4, self.dpi));
            DrawFocusRect(draw.hDC, &rect);
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        unsafe {
            for font in &self.fonts {
                if !font.is_null() {
                    DeleteObject(*font);
                }
            }
            DeleteObject(self.page);
            DeleteObject(self.card);
        }
    }
}

unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let creation = &*(lparam as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, creation.lpCreateParams as isize);
    }
    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    if raw.is_null() {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    let app = &mut *raw;
    if app.taskbar_created != 0 && message == app.taskbar_created {
        app.tray(NIM_ADD);
        return 0;
    }
    match message {
        UPDATE_READY => {
            app.updates_ready();
            0
        }
        UPDATE_DOWNLOADED => {
            app.update_downloaded();
            0
        }
        WM_CONTEXTMENU if wparam as HWND == app.control(GITHUB) => {
            app.github_menu();
            0
        }
        WM_SETCURSOR if wparam as HWND == app.control(GITHUB) => {
            SetCursor(LoadCursorW(ptr::null_mut(), IDC_HAND));
            1
        }
        WM_NCCALCSIZE => 0,
        WM_NCHITTEST => {
            let mut point = POINT {
                x: (lparam as i16) as i32,
                y: ((lparam >> 16) as i16) as i32,
            };
            ScreenToClient(hwnd, &mut point);
            if point.y >= 0
                && point.y < scale(view::HEADER, app.dpi)
                && point.x >= scale(40, app.dpi)
                && point.x < scale(272, app.dpi)
            {
                HTCAPTION as isize
            } else {
                HTCLIENT as isize
            }
        }
        WM_CREATE => {
            app.hwnd = hwnd;
            if let Err(error) = app.create() {
                MessageBoxW(
                    hwnd,
                    wide(&error).as_ptr(),
                    wide(APP_NAME).as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
                return -1;
            }
            0
        }
        WM_CLOSE => {
            KillTimer(hwnd, 2);
            ShowWindow(hwnd, SW_HIDE);
            0
        }
        WM_COMMAND => {
            app.action(wparam & 0xffff);
            0
        }
        WM_TIMER => {
            if wparam == 3 || wparam == 4 {
                if wparam == 4 {
                    KillTimer(hwnd, 4);
                }
                app.check_updates(false);
            } else if wparam == 2 {
                if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
                    KillTimer(hwnd, 2);
                } else {
                    let rect = RECT {
                        left: scale(88, app.dpi),
                        top: scale(92, app.dpi),
                        right: scale(232, app.dpi),
                        bottom: scale(112, app.dpi),
                    };
                    InvalidateRect(hwnd, &rect, 0);
                }
            } else {
                app.refresh();
            }
            0
        }
        DONE => {
            app.completed();
            0
        }
        TRAY => {
            match lparam as u32 {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => app.show(),
                WM_RBUTTONUP | WM_CONTEXTMENU => app.menu(),
                _ => {}
            }
            0
        }
        WM_DPICHANGED => {
            let work = work_area(hwnd);
            app.dpi = fitted_dpi((wparam as u32 & 0xffff).max(96), work);
            let rect = &*(lparam as *const RECT);
            let width = scale(view::WIDTH, app.dpi);
            let height = scale(view::HEIGHT, app.dpi);
            SetWindowPos(
                hwnd,
                ptr::null_mut(),
                rect.left.clamp(work.left, work.right - width),
                rect.top.clamp(work.top, work.bottom - height),
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            app.layout();
            0
        }
        WM_SIZE => {
            if wparam != SIZE_MINIMIZED as usize && !app.controls.is_empty() {
                app.layout();
            }
            0
        }
        WM_CTLCOLORSTATIC => {
            let dc = wparam as HDC;
            // 标题、分组小标题在灰底上，其余在白卡片上
            let title = [APP_TITLE, GROUP_DICTATION, GROUP_TEXT]
                .iter()
                .any(|&id| lparam as HWND == app.control(id));
            SetBkColor(dc, if title { view::BACKGROUND } else { view::CARD });
            SetTextColor(
                dc,
                if lparam as HWND == app.control(LABEL_ERROR) {
                    // 出错是红字；「已复制」这类提示是灰字
                    let failed = !app.command_error.is_empty()
                        || app.status.as_ref().is_some_and(|s| s.phase == "error");
                    if failed { 0x003f39bd } else { MUTED }
                } else if [
                    APP_TITLE,
                    UPDATE_TITLE,
                    LABEL_STATUS,
                    LABEL_HOTKEY,
                    LABEL_MIC_TITLE,
                    LABEL_SILENCE,
                    LABEL_HOTWORDS,
                    LABEL_HISTORY,
                    LABEL_TRANSLATE,
                    LABEL_TEXT,
                ]
                .iter()
                .any(|&id| lparam as HWND == app.control(id))
                {
                    INK
                } else {
                    MUTED
                },
            );
            if title {
                app.page as isize
            } else {
                app.card as isize
            }
        }
        WM_DRAWITEM => {
            let draw = &*(lparam as *const DRAWITEMSTRUCT);
            let id = draw.CtlID as usize;
            if VALUE_BUTTONS.contains(&id)
                || [
                    TOGGLE,
                    MINIMIZE,
                    CLOSE,
                    GITHUB,
                    UPDATE_DOWNLOAD,
                    UPDATE_SKIP,
                    UPDATE_CLOSE,
                ]
                .contains(&id)
            {
                app.draw_button(draw);
                return 1;
            }
            0
        }
        WM_ERASEBKGND => {
            // WM_PAINT draws the complete invalid region through a memory DC.
            // Erasing first exposes a blank frame during native control updates.
            1
        }
        WM_PAINT => {
            let mut paint: PAINTSTRUCT = mem::zeroed();
            let dc = BeginPaint(hwnd, &mut paint);
            let mut client: RECT = mem::zeroed();
            GetClientRect(hwnd, &mut client);
            let memory = CreateCompatibleDC(dc);
            let bitmap = CreateCompatibleBitmap(dc, client.right, client.bottom);
            let old = SelectObject(memory, bitmap);
            FillRect(memory, &client, app.page);
            DrawIconEx(
                memory,
                scale(16, app.dpi),
                scale(10, app.dpi),
                app.icon,
                scale(22, app.dpi),
                scale(22, app.dpi),
                0,
                ptr::null_mut(),
                DI_NORMAL,
            );
            let peak = if app.animate {
                app.status.as_ref().map_or(0, |s| s.peak)
            } else {
                0
            };
            view::background(memory, app.dpi, app.ready, peak, now_ms() % 60000 / 60);
            if app.update_visible {
                view::update_bubble(memory, app.dpi, app.update_release.is_some());
            }
            BitBlt(dc, 0, 0, client.right, client.bottom, memory, 0, 0, SRCCOPY);
            SelectObject(memory, old);
            DeleteObject(bitmap);
            DeleteDC(memory);
            EndPaint(hwnd, &paint);
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, 1);
            KillTimer(hwnd, 2);
            KillTimer(hwnd, 3);
            KillTimer(hwnd, 4);
            app.tray(NIM_DELETE);
            PostQuitMessage(0);
            0
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

/// Used by installers and normal automation; never terminates a process.
pub fn quit() -> Result<(), String> {
    unsafe {
        let window = FindWindowW(wide(CLASS).as_ptr(), ptr::null());
        if !window.is_null() {
            let mut pid = 0;
            GetWindowThreadProcessId(window, &mut pid);
            let process = Handle(OpenProcess(SYNCHRONIZATION_SYNCHRONIZE, 0, pid));
            if process.0.is_null() {
                return Err("无法等待雪耳退出".to_owned());
            }
            if PostMessageW(window, WM_COMMAND, EXIT, 0) == 0 {
                return Err("无法请求雪耳退出".to_owned());
            }
            if WaitForSingleObject(process.0, 30_000) != WAIT_OBJECT_0 {
                return Err("雪耳尚未退出，安装已暂停".to_owned());
            }
        }
        let stop = Handle(OpenEventW(
            EVENT_MODIFY_STATE | SYNCHRONIZATION_SYNCHRONIZE,
            0,
            wide("Local\\XueerEar.Stop").as_ptr(),
        ));
        if stop.0.is_null() {
            return Ok(());
        }
        let restored = Handle(OpenEventW(
            SYNCHRONIZATION_SYNCHRONIZE,
            0,
            wide("Local\\XueerEar.Restored").as_ptr(),
        ));
        if restored.0.is_null() || SetEvent(stop.0) == 0 {
            return Err("无法请求后台恢复Windows原生识别".to_owned());
        }
        if WaitForSingleObject(restored.0, 30_000) != WAIT_OBJECT_0 {
            return Err("Windows原生识别尚未恢复，安装已暂停".to_owned());
        }
        Ok(())
    }
}

/// 注册窗口类并建好控制面板窗口（还没显示）。`offscreen` 用于拍照核对外观：放到屏幕外、不抢焦点。
unsafe fn open_window(app: &mut App, offscreen: bool) -> HWND {
    let module = GetModuleHandleW(ptr::null());
    let class_name = wide(CLASS);
    let class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(procedure),
        hInstance: module,
        hIcon: app.icon,
        hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
        hbrBackground: ptr::null_mut(),
        lpszClassName: class_name.as_ptr(),
        ..mem::zeroed()
    };
    if RegisterClassW(&class) == 0 {
        return ptr::null_mut();
    }
    let style = WS_POPUP | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
    let work = work_area(ptr::null_mut());
    let dpi = fitted_dpi(GetDpiForSystem().max(96), work);
    let (width, height) = (scale(view::WIDTH, dpi), scale(view::HEIGHT, dpi));
    let (x, y) = if offscreen {
        (-32000, -32000)
    } else {
        (
            work.left + ((work.right - work.left) - width) / 2,
            work.top + ((work.bottom - work.top) - height) / 2,
        )
    };
    let hwnd = CreateWindowExW(
        if offscreen {
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE
        } else {
            0
        },
        class_name.as_ptr(),
        wide(APP_NAME).as_ptr(),
        style,
        x,
        y,
        width,
        height,
        ptr::null_mut(),
        ptr::null_mut(),
        module,
        (app as *mut App).cast(),
    );
    if hwnd.is_null() {
        return hwnd;
    }
    SetWindowPos(
        hwnd,
        ptr::null_mut(),
        0,
        0,
        0,
        0,
        SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
    );
    use windows_sys::Win32::Graphics::Dwm::*;
    let margins = windows_sys::Win32::UI::Controls::MARGINS {
        cxLeftWidth: 1,
        cxRightWidth: 1,
        cyTopHeight: 1,
        cyBottomHeight: 1,
    };
    DwmExtendFrameIntoClientArea(hwnd, &margins);
    let caption = view::BACKGROUND;
    let rounded = 2_u32;
    DwmSetWindowAttribute(
        hwnd,
        DWMWA_CAPTION_COLOR as u32,
        (&caption as *const u32).cast(),
        4,
    );
    DwmSetWindowAttribute(
        hwnd,
        DWMWA_WINDOW_CORNER_PREFERENCE as u32,
        (&rounded as *const u32).cast(),
        4,
    );
    hwnd
}

pub fn run() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let settings = xueer_ear_backend::settings::Settings::load_default().unwrap_or_default();
        // 「以管理员身份运行」开着、自己又不是：以管理员身份重开（会弹 UAC），这个进程退出；
        // 点了「否」就照常以普通身份运行。要在占住单实例之前做，新进程才能接着占
        if settings.run_as_admin && !crate::system::elevated() {
            let args: Vec<String> = std::env::args().skip(1).collect();
            if crate::system::relaunch_elevated(&args.join(" ")) {
                return;
            }
        }
        // 第一次打开（还没有设置文件）：存一份默认设置，直接开好后台服务
        let first_run = xueer_ear_backend::settings::Settings::modified_default().is_none();
        if first_run {
            let _ = settings.save_default();
        }
        let mutex = Handle(CreateMutexW(
            ptr::null(),
            0,
            wide("Local\\XueerEar.ControlPanel").as_ptr(),
        ));
        if !mutex.0.is_null() && GetLastError() == ERROR_ALREADY_EXISTS {
            let hwnd = FindWindowW(wide(CLASS).as_ptr(), ptr::null());
            if !hwnd.is_null() {
                ShowWindow(hwnd, SW_RESTORE);
                SetForegroundWindow(hwnd);
            }
            return;
        }
        let mut app = Box::new(App::new());
        app.first_run = first_run;
        let hwnd = open_window(&mut app, false);
        if hwnd.is_null() {
            return;
        }
        // 开机自启：不弹窗，只在托盘，并自动开启后台服务。第一次打开、一键更新装完重开也直接开好后台服务
        let autostart = std::env::args_os().any(|arg| arg == "--autostart");
        let updated = std::env::args_os().any(|arg| arg == "--updated");
        if autostart || updated || first_run {
            PostMessageW(hwnd, WM_COMMAND, START, 0);
        }
        if updated {
            PostMessageW(hwnd, WM_COMMAND, UPDATED, 0);
        }
        if !autostart {
            ShowWindow(hwnd, SW_SHOW);
        }
        let mut message: MSG = mem::zeroed();
        while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(hwnd, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}

/// 处理一小会儿消息，让窗口把自己画完
unsafe fn settle(milliseconds: u64) {
    let until = std::time::Instant::now() + std::time::Duration::from_millis(milliseconds);
    let mut message: MSG = mem::zeroed();
    while std::time::Instant::now() < until {
        while PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        thread::sleep(std::time::Duration::from_millis(15));
    }
}

/// 把位图存成 PNG
unsafe fn save_bitmap(bitmap: HBITMAP, path: &std::path::Path) -> Result<(), String> {
    use windows_sys::Win32::Graphics::GdiPlus as gp;
    let mut image = ptr::null_mut();
    if gp::GdipCreateBitmapFromHBITMAP(bitmap, ptr::null_mut(), &mut image) != 0 {
        return Err("无法转换位图".to_owned());
    }
    // PNG 编码器
    let png = windows_sys::core::GUID::from_u128(0x557cf406_1a04_11d3_9a73_0000f81ef32e);
    let name = wide(&path.to_string_lossy());
    let saved = gp::GdipSaveImageToFile(image.cast(), name.as_ptr(), &png, ptr::null()) == 0;
    gp::GdipDisposeImage(image.cast());
    if saved {
        Ok(())
    } else {
        Err(format!("无法保存 {}", path.display()))
    }
}

/// 把一个窗口现在的样子存成 PNG
unsafe fn capture(window: HWND, path: &std::path::Path) -> Result<(), String> {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn PrintWindow(window: HWND, dc: HDC, flags: u32) -> i32;
    }
    let mut rect: RECT = mem::zeroed();
    GetWindowRect(window, &mut rect);
    let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
    let screen = GetDC(ptr::null_mut());
    let dc = CreateCompatibleDC(screen);
    let bitmap = CreateCompatibleBitmap(screen, width, height);
    let old = SelectObject(dc, bitmap);
    // 2 = PW_RENDERFULLCONTENT：连同圆角和子控件一起画
    let printed = PrintWindow(window, dc, 2) != 0;
    SelectObject(dc, old);
    let result = if printed {
        save_bitmap(bitmap, path)
    } else {
        Err("无法截取窗口".to_owned())
    };
    DeleteObject(bitmap);
    DeleteDC(dc);
    ReleaseDC(ptr::null_mut(), screen);
    result
}

/// 离屏把控制面板、「添加热词」对话框、托盘菜单和热词菜单各拍一张 PNG 到 `dir`（核对外观用）。
/// 窗口建在屏幕外、不抢焦点、不建托盘图标，不会打扰正在用电脑的人。
pub fn snapshot(dir: &std::path::Path) -> Result<(), String> {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let mut app = Box::new(App::new());
        app.snapshot = true;
        let hwnd = open_window(&mut app, true);
        if hwnd.is_null() {
            return Err("无法创建控制面板".to_owned());
        }
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        settle(900);
        let panel = capture(hwnd, &dir.join("control-panel.png"));
        let dialog = crate::hotword_dialog::offscreen(hwnd, app.fonts[3], app.dpi);
        let asked = if dialog.is_null() {
            Err("无法创建对话框".to_owned())
        } else {
            settle(400);
            let result = capture(dialog, &dir.join("hotword-dialog.png"));
            DestroyWindow(dialog);
            result
        };
        // 托盘右键菜单：按「正在用」的样子画一张，鼠标停在「打开控制面板」上
        let tray = {
            let choices = crate::prefs::tray_choices(
                "待机",
                app.settings.hotkey_vk,
                true,
                false,
                OPEN,
                TOGGLE,
                EXIT,
            );
            let (bitmap, _, _) = crate::menu::render(&choices, app.fonts[3], app.dpi, 2);
            let result = save_bitmap(bitmap, &dir.join("tray-menu.png"));
            DeleteObject(bitmap);
            result
        };
        let translate_menu = {
            let keys = xueer_ear_backend::translate::local_keys();
            let choices = crate::prefs::translate_choices(&keys);
            let (bitmap, _, _) = crate::menu::render(&choices, app.fonts[3], app.dpi, 0);
            let result = save_bitmap(bitmap, &dir.join("translate-menu.png"));
            DeleteObject(bitmap);
            result
        };
        let hotwords = {
            let choices = crate::prefs::hotword_choices(&app.settings);
            let (bitmap, _, _) = crate::menu::render(&choices, app.fonts[3], app.dpi, 0);
            let result = save_bitmap(bitmap, &dir.join("hotword-menu.png"));
            DeleteObject(bitmap);
            result
        };
        let mut translated = Ok(());
        for (busy, name) in [
            (false, "translate-window.png"),
            (true, "translate-busy.png"),
        ] {
            let translator = crate::translate_window::offscreen(busy);
            translated = translated.and(if translator.is_null() {
                Err("无法创建翻译小窗".to_owned())
            } else {
                settle(400);
                let result = capture(translator, &dir.join(name));
                DestroyWindow(translator);
                result
            });
        }
        // 更新气泡：发现新版本；一键更新装好、重新打开以后
        let offer = {
            let release = crate::update::Release {
                version: "9.9.9".into(),
                tag: "v9.9.9".into(),
                url: String::new(),
            };
            let _ = app.update_sender.send((true, Ok(Some(release))));
            SendMessageW(hwnd, UPDATE_READY, 0, 0);
            settle(300);
            capture(hwnd, &dir.join("update-offer.png"))
        };
        SendMessageW(hwnd, WM_COMMAND, UPDATED, 0);
        settle(300);
        let updated = capture(hwnd, &dir.join("update-installed.png"));
        DestroyWindow(hwnd);
        panel
            .and(asked)
            .and(tray)
            .and(hotwords)
            .and(translated)
            .and(translate_menu)
            .and(offer)
            .and(updated)
    }
}
