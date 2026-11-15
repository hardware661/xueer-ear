//! 晓晓提示音：构建时打包进程序，本地异步播放，不走音箱。设置里能关。
use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

pub enum Cue {
    /// 快捷键模式这次什么也没识别出来
    Missed,
    /// 系统组件变了，退回原生 Win+H、改用快捷键
    Fallback,
}

static MISSED: &[u8] = include_bytes!("../../../assets/sounds/missed.wav");
static FALLBACK: &[u8] = include_bytes!("../../../assets/sounds/fallback.wav");

pub fn play(cue: Cue, enabled: bool) {
    if !enabled {
        return;
    }
    let bytes = match cue {
        Cue::Missed => MISSED,
        Cue::Fallback => FALLBACK,
    };
    // SND_MEMORY 时第一个参数是指向 WAV 数据的指针；数据是 static，异步播放期间一直有效
    unsafe {
        PlaySoundW(
            bytes.as_ptr().cast(),
            std::ptr::null_mut(),
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        );
    }
}
