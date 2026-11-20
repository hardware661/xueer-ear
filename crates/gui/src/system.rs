//! 本机的几件事：这个进程是不是管理员身份、以管理员身份重开自己、Windows 版本（诊断信息用）。
#![allow(unsafe_op_in_unsafe_fn)]
use std::{mem, ptr};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
    System::{
        Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW},
        Threading::{GetCurrentProcess, OpenProcessToken},
    },
    UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
};

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

/// 这个进程是不是以管理员身份在跑
pub fn elevated() -> bool {
    unsafe {
        let mut token: HANDLE = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation: TOKEN_ELEVATION = mem::zeroed();
        let mut size = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        ) != 0;
        CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

/// 以管理员身份重开自己，参数照传（会弹 UAC）。点了「否」或开不了返回 false，调用方照常以普通身份继续
pub fn relaunch_elevated(args: &str) -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            wide("runas").as_ptr(),
            wide(&exe.to_string_lossy()).as_ptr(),
            wide(args).as_ptr(),
            ptr::null(),
            SW_SHOWNORMAL,
        ) as isize
            > 32
    }
}

/// 诊断信息里的系统版本，如「11 25H2（26200.7462）」。注册表里的产品名在 Windows 11 上也写 10，按内部版本号分
pub fn windows_version() -> String {
    let key = wide(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
    let text = |name: &str| -> Option<String> {
        let mut buffer = [0_u16; 64];
        let mut size = mem::size_of_val(&buffer) as u32;
        let ok = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                wide(name).as_ptr(),
                RRF_RT_REG_SZ,
                ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        } == 0;
        let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        ok.then(|| String::from_utf16_lossy(&buffer[..len]))
    };
    let mut revision = 0_u32;
    let mut size = mem::size_of::<u32>() as u32;
    unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            wide("UBR").as_ptr(),
            RRF_RT_REG_DWORD,
            ptr::null_mut(),
            (&mut revision as *mut u32).cast(),
            &mut size,
        );
    }
    let build: u32 = text("CurrentBuild")
        .and_then(|b| b.parse().ok())
        .unwrap_or(0);
    format!(
        "{} {}（{build}.{revision}）",
        if build >= 22000 { "11" } else { "10" },
        text("DisplayVersion").unwrap_or_default()
    )
}
