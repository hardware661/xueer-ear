//! Anonymous GitHub release checks. Run `latest` on a worker, never the UI thread.
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

pub const REPOSITORY_URL: &str = "https://github.com/hardware661/xueer-ear";
const MAX_BODY: usize = 256 * 1024;
const MAX_PREFERENCES: usize = 4 * 1024;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    /// 发布时用的标签原文（带不带 v 都有过），下载地址里要用它
    pub tag: String,
    pub url: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Preferences {
    pub skipped_version: Option<String>,
}

impl Preferences {
    pub fn load() -> Result<Self, String> {
        Self::load_path(&preferences_path()?)
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_path(&preferences_path()?)
    }

    fn normalized(&self) -> Result<Self, String> {
        Ok(Self {
            skipped_version: self
                .skipped_version
                .as_deref()
                .map(|value| {
                    version(value)
                        .map(|value| version_text(value))
                        .ok_or_else(|| "跳过版本号无效".to_owned())
                })
                .transpose()?,
        })
    }

    fn load_path(path: &Path) -> Result<Self, String> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(format!("读取更新偏好失败：{error}")),
        };
        if file.metadata().map_err(|e| e.to_string())?.len() > MAX_PREFERENCES as u64 {
            return Err("更新偏好超过 4 KiB".into());
        }
        let mut body = Vec::with_capacity(128);
        file.take(MAX_PREFERENCES as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|e| e.to_string())?;
        if body.len() > MAX_PREFERENCES {
            return Err("更新偏好超过 4 KiB".into());
        }
        let value: Self = serde_json::from_slice(&body).map_err(|_| "更新偏好格式无效")?;
        value.normalized()
    }

    fn save_path(&self, path: &Path) -> Result<(), String> {
        let body = serde_json::to_vec(&self.normalized()?).map_err(|_| "无法编码更新偏好")?;
        if body.len() > MAX_PREFERENCES {
            return Err("更新偏好超过 4 KiB".into());
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary = path.with_file_name(format!(
            ".update-preferences.{}.{time}.{sequence}.tmp",
            std::process::id()
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        let result = (|| -> std::io::Result<()> {
            file.write_all(&body)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.map_err(|e| format!("保存更新偏好失败：{e}"))
    }
}

fn preferences_path() -> Result<PathBuf, String> {
    let root = std::env::var_os("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .ok_or("LOCALAPPDATA 不可用")?;
    Ok(PathBuf::from(root).join("XueerEar/update-preferences.json"))
}

fn version(value: &str) -> Option<[u64; 3]> {
    if value.len() > 63 {
        return None;
    }
    let value = value.strip_prefix('v').unwrap_or(value);
    let mut parts = value.split('.');
    let mut result = [0; 3];
    for number in &mut result {
        let text = parts.next()?;
        if text.is_empty()
            || !text.bytes().all(|b| b.is_ascii_digit())
            || text.len() > 1 && text.starts_with('0')
        {
            return None;
        }
        *number = text.parse().ok()?;
    }
    parts.next().is_none().then_some(result)
}

fn version_text(value: [u64; 3]) -> String {
    format!("{}.{}.{}", value[0], value[1], value[2])
}

#[derive(Deserialize)]
struct Asset {
    name: String,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

/// Only stable, newer versions with both published Windows packages are offered.
/// URLs from JSON are deliberately neither deserialized nor used.
pub fn parse(body: &[u8], current: &str) -> Result<Option<Release>, String> {
    let current = version(current).ok_or("当前版本号无效")?;
    if body.len() > MAX_BODY {
        return Err("发布信息超过 256 KiB".into());
    }
    let release: GithubRelease =
        serde_json::from_slice(body).map_err(|_| "GitHub 发布信息格式无效")?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let available = version(&release.tag_name).ok_or("发布版本号无效")?;
    if available <= current {
        return Ok(None);
    }
    let text = version_text(available);
    let portable = format!("xueer-ear-{text}-windows-x64-portable.zip");
    let setup = format!("xueer-ear-{text}-windows-x64-setup.exe");
    if !release.assets.iter().any(|asset| asset.name == portable)
        || !release.assets.iter().any(|asset| asset.name == setup)
    {
        return Ok(None);
    }
    Ok(Some(Release {
        version: text,
        url: format!("{REPOSITORY_URL}/releases/tag/{}", release.tag_name),
        tag: release.tag_name,
    }))
}

/// 安装版（同目录有卸载程序）才能一键更新；便携版只打开发布页
pub fn installed() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("unins000.exe").is_file()))
        .unwrap_or(false)
}

/// SHA256SUMS.txt 里某个文件的校验值（小写十六进制）
pub fn expected_sha256(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.trim().split_once(char::is_whitespace)?;
        let file = file.trim().trim_start_matches('*');
        (file == name && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    })
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 下载的安装包最大多少（现在约 4 MB）
const MAX_INSTALLER: usize = 64 * 1024 * 1024;

/// 一键更新：从这一版的发布页下载安装包和校验文件，核对 SHA-256 后存进临时目录，返回安装包路径
#[cfg(windows)]
pub fn download_installer(release: &Release) -> Result<PathBuf, String> {
    let name = format!("xueer-ear-{}-windows-x64-setup.exe", release.version);
    // 标签在 parse 里已经核对过只是版本号，可以直接放进路径
    let base = format!("/hardware661/xueer-ear/releases/download/{}/", release.tag);
    let sums = native::download(&format!("{base}SHA256SUMS.txt"), 16 * 1024)?;
    let expected = expected_sha256(&String::from_utf8_lossy(&sums), &name)
        .ok_or("校验文件里没有这个安装包")?;
    let setup = native::download(&format!("{base}{name}"), MAX_INSTALLER)?;
    if sha256_hex(&setup) != expected {
        return Err("下载的安装包校验不对".into());
    }
    let dir = std::env::temp_dir().join(format!("xueer-ear-update-{}", release.version));
    fs::create_dir_all(&dir).map_err(|e| format!("存不了安装包：{e}"))?;
    let path = dir.join(&name);
    fs::write(&path, &setup).map_err(|e| format!("存不了安装包：{e}"))?;
    Ok(path)
}

#[cfg(not(windows))]
pub fn latest(_current: &str) -> Result<Option<Release>, String> {
    Err("此平台不支持在线检查更新".into())
}

#[cfg(windows)]
pub fn latest(current: &str) -> Result<Option<Release>, String> {
    let current = version_text(version(current).ok_or("当前版本号无效")?);
    native::fetch(&current)?
        .map(|body| parse(&body, &current))
        .transpose()
        .map(Option::flatten)
}

#[cfg(windows)]
mod native {
    use super::MAX_BODY;
    use std::{
        ffi::c_void,
        mem, ptr,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{Foundation::GetLastError, Networking::WinHttp::*};

    struct Handle(*mut c_void);
    impl Handle {
        fn checked(raw: *mut c_void) -> Result<Self, String> {
            if raw.is_null() {
                Err(error())
            } else {
                Ok(Self(raw))
            }
        }
    }
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                WinHttpCloseHandle(self.0);
            }
        }
    }
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    fn error() -> String {
        format!("连不上 GitHub（错误 {}）", unsafe { GetLastError() })
    }
    fn check(value: i32) -> Result<(), String> {
        if value == 0 { Err(error()) } else { Ok(()) }
    }

    fn option(handle: &Handle, name: u32, value: u32) -> Result<(), String> {
        check(unsafe {
            WinHttpSetOption(
                handle.0,
                name,
                (&value as *const u32).cast(),
                mem::size_of::<u32>() as u32,
            )
        })
    }

    /// GitHub 发布页的下载地址会跳到 *.githubusercontent.com：手动跟最多 4 次，
    /// 只认 https 的 github.com 和 *.githubusercontent.com，别的地址一律不去
    pub(super) fn download(path: &str, limit: usize) -> Result<Vec<u8>, String> {
        let deadline = Instant::now() + Duration::from_secs(180);
        let (mut host, mut path) = ("github.com".to_owned(), path.to_owned());
        for _ in 0..5 {
            match get(&host, &path, limit, deadline)? {
                Got::Body(body) => return Ok(body),
                Got::Moved(location) => {
                    let rest = location
                        .strip_prefix("https://")
                        .ok_or("下载地址跳到了不认识的地方")?;
                    let (next_host, next_path) = rest
                        .split_once('/')
                        .map_or((rest, "/".to_owned()), |(h, p)| (h, format!("/{p}")));
                    if next_host != "github.com" && !next_host.ends_with(".githubusercontent.com") {
                        return Err("下载地址跳到了不认识的地方".into());
                    }
                    (host, path) = (next_host.to_owned(), next_path);
                }
            }
        }
        Err("下载地址跳转太多".into())
    }

    enum Got {
        Body(Vec<u8>),
        Moved(String),
    }

    fn get(host: &str, path: &str, limit: usize, deadline: Instant) -> Result<Got, String> {
        unsafe {
            let session = Handle::checked(WinHttpOpen(
                wide(&format!("xueer-ear/{}", env!("CARGO_PKG_VERSION"))).as_ptr(),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                ptr::null(),
                ptr::null(),
                0,
            ))?;
            check(WinHttpSetTimeouts(
                session.0, 10_000, 10_000, 30_000, 30_000,
            ))?;
            let connection = Handle::checked(WinHttpConnect(
                session.0,
                wide(host).as_ptr(),
                INTERNET_DEFAULT_HTTPS_PORT,
                0,
            ))?;
            let request = Handle::checked(WinHttpOpenRequest(
                connection.0,
                wide("GET").as_ptr(),
                wide(path).as_ptr(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                WINHTTP_FLAG_SECURE,
            ))?;
            option(
                &request,
                WINHTTP_OPTION_DISABLE_FEATURE,
                WINHTTP_DISABLE_REDIRECTS | WINHTTP_DISABLE_COOKIES,
            )?;
            option(
                &request,
                WINHTTP_OPTION_AUTOLOGON_POLICY,
                WINHTTP_AUTOLOGON_SECURITY_LEVEL_HIGH,
            )?;
            check(WinHttpSendRequest(
                request.0,
                ptr::null(),
                0,
                ptr::null(),
                0,
                0,
                0,
            ))?;
            check(WinHttpReceiveResponse(request.0, ptr::null_mut()))?;
            let mut status = 0_u32;
            let mut length = mem::size_of::<u32>() as u32;
            check(WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                ptr::null(),
                (&mut status as *mut u32).cast(),
                &mut length,
                ptr::null_mut(),
            ))?;
            if matches!(status, 301 | 302 | 303 | 307 | 308) {
                let mut location = vec![0_u16; 4096];
                let mut size = (location.len() * 2) as u32;
                check(WinHttpQueryHeaders(
                    request.0,
                    WINHTTP_QUERY_LOCATION,
                    ptr::null(),
                    location.as_mut_ptr().cast(),
                    &mut size,
                    ptr::null_mut(),
                ))?;
                return Ok(Got::Moved(String::from_utf16_lossy(
                    &location[..size as usize / 2],
                )));
            }
            if status != 200 {
                return Err(format!("下载更新失败（HTTP {status}）"));
            }
            let mut body = Vec::with_capacity(64 * 1024);
            let mut buffer = vec![0_u8; 64 * 1024];
            loop {
                if Instant::now() >= deadline {
                    return Err("下载更新超时".into());
                }
                let mut received = 0_u32;
                check(WinHttpReadData(
                    request.0,
                    buffer.as_mut_ptr().cast(),
                    buffer.len() as u32,
                    &mut received,
                ))?;
                if received == 0 {
                    return Ok(Got::Body(body));
                }
                let received = received as usize;
                if received > buffer.len() || body.len() + received > limit {
                    return Err("下载的文件太大".into());
                }
                body.extend_from_slice(&buffer[..received]);
            }
        }
    }

    pub(super) fn fetch(version: &str) -> Result<Option<Vec<u8>>, String> {
        unsafe {
            let session = Handle::checked(WinHttpOpen(
                wide(&format!("xueer-ear/{version}")).as_ptr(),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                ptr::null(),
                ptr::null(),
                0,
            ))?;
            check(WinHttpSetTimeouts(session.0, 5000, 5000, 5000, 5000))?;
            // This WinHTTP option counts connection attempts, not additional retries.
            option(&session, WINHTTP_OPTION_CONNECT_RETRIES, 1)?;
            let connection = Handle::checked(WinHttpConnect(
                session.0,
                wide("api.github.com").as_ptr(),
                INTERNET_DEFAULT_HTTPS_PORT,
                0,
            ))?;
            let request = Handle::checked(WinHttpOpenRequest(
                connection.0,
                wide("GET").as_ptr(),
                wide("/repos/hardware661/xueer-ear/releases/latest").as_ptr(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                WINHTTP_FLAG_SECURE,
            ))?;
            // A fresh anonymous session has no application token/cookie store;
            // redirects must never change the fixed update origin.
            option(
                &request,
                WINHTTP_OPTION_DISABLE_FEATURE,
                WINHTTP_DISABLE_REDIRECTS | WINHTTP_DISABLE_COOKIES,
            )?;
            option(
                &request,
                WINHTTP_OPTION_AUTOLOGON_POLICY,
                WINHTTP_AUTOLOGON_SECURITY_LEVEL_HIGH,
            )?;
            let headers =
                wide("Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28\r\n");
            check(WinHttpSendRequest(
                request.0,
                headers.as_ptr(),
                (headers.len() - 1) as u32,
                ptr::null(),
                0,
                0,
                0,
            ))?;
            check(WinHttpReceiveResponse(request.0, ptr::null_mut()))?;
            let mut status = 0_u32;
            let mut length = mem::size_of::<u32>() as u32;
            check(WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                ptr::null(),
                (&mut status as *mut u32).cast(),
                &mut length,
                ptr::null_mut(),
            ))?;
            if status == 404 {
                return Ok(None);
            }
            if status != 200 {
                return Err(format!("检查更新失败（HTTP {status}）"));
            }
            let mut body = Vec::with_capacity(16 * 1024);
            let mut buffer = [0_u8; 8192];
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if Instant::now() >= deadline {
                    return Err("读取发布信息超时".into());
                }
                let mut received = 0_u32;
                check(WinHttpReadData(
                    request.0,
                    buffer.as_mut_ptr().cast(),
                    buffer.len() as u32,
                    &mut received,
                ))?;
                if received == 0 {
                    break;
                }
                let received = received as usize;
                if received > buffer.len() || body.len() + received > MAX_BODY {
                    return Err("发布信息超过 256 KiB".into());
                }
                body.extend_from_slice(&buffer[..received]);
            }
            Ok(Some(body))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(tag: &str) -> serde_json::Value {
        let normalized = tag.strip_prefix('v').unwrap_or(tag);
        serde_json::json!({
            "tag_name": tag, "draft": false, "prerelease": false,
            "html_url": "https://example.invalid/redirect",
            "assets": [
                {"name": format!("xueer-ear-{normalized}-windows-x64-portable.zip"), "browser_download_url":"https://example.invalid/zip"},
                {"name": format!("xueer-ear-{normalized}-windows-x64-setup.exe"), "browser_download_url":"https://example.invalid/exe"}
            ]
        })
    }
    fn encoded(value: &serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(value).unwrap()
    }

    /// 联网核对：真从 GitHub 下 1.1.18 的安装包、核对校验值（`cargo test -- --ignored`）
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn downloads_and_verifies_a_published_installer() {
        let release = Release {
            version: "1.1.18".into(),
            tag: "v1.1.18".into(),
            url: String::new(),
        };
        let path = download_installer(&release).unwrap();
        assert!(fs::metadata(&path).unwrap().len() > 1_000_000);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn the_installer_checksum_is_read_from_sha256sums() {
        let sums = "AB12cd34ab12cd34ab12cd34ab12cd34ab12cd34ab12cd34ab12cd34ab12cd34  xueer-ear-1.2.0-windows-x64-portable.zip\n\
                    0000000000000000000000000000000000000000000000000000000000000001 *xueer-ear-1.2.0-windows-x64-setup.exe\n";
        assert_eq!(
            expected_sha256(sums, "xueer-ear-1.2.0-windows-x64-setup.exe").as_deref(),
            Some("0000000000000000000000000000000000000000000000000000000000000001")
        );
        assert_eq!(
            expected_sha256(sums, "xueer-ear-1.2.0-windows-x64-portable.zip").as_deref(),
            Some("ab12cd34ab12cd34ab12cd34ab12cd34ab12cd34ab12cd34ab12cd34ab12cd34")
        );
        assert!(expected_sha256(sums, "xueer-ear-1.2.1-windows-x64-setup.exe").is_none());
        assert!(
            expected_sha256(
                "deadbeef  xueer-ear-1.2.0-windows-x64-setup.exe",
                "xueer-ear-1.2.0-windows-x64-setup.exe"
            )
            .is_none()
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn numerical_versions_compare_correctly_and_urls_ignore_json_redirects() {
        let release = parse(&encoded(&fixture("v0.1.10")), "0.1.9")
            .unwrap()
            .unwrap();
        assert_eq!(release.version, "0.1.10");
        assert_eq!(
            release.url,
            "https://github.com/hardware661/xueer-ear/releases/tag/v0.1.10"
        );
        assert!(
            parse(&encoded(&fixture("0.1.9")), "v0.1.10")
                .unwrap()
                .is_none()
        );
        assert!(
            parse(&encoded(&fixture("v0.1.10")), "0.1.10")
                .unwrap()
                .is_none()
        );
        assert!(
            parse(&encoded(&fixture("1.0.0")), "0.99.99")
                .unwrap()
                .is_some()
        );
        assert_eq!(
            parse(&encoded(&fixture("0.1.10")), "0.1.9")
                .unwrap()
                .unwrap()
                .url,
            "https://github.com/hardware661/xueer-ear/releases/tag/0.1.10"
        );
    }

    #[test]
    fn draft_prerelease_and_incomplete_or_wrong_version_packages_are_hidden() {
        for key in ["draft", "prerelease"] {
            let mut value = fixture("v1.2.3");
            value[key] = true.into();
            assert!(parse(&encoded(&value), "1.2.2").unwrap().is_none());
        }
        for missing in [0, 1] {
            let mut value = fixture("v1.2.3");
            value["assets"].as_array_mut().unwrap().remove(missing);
            assert!(parse(&encoded(&value), "1.2.2").unwrap().is_none());
        }
        let mut value = fixture("v1.2.3");
        value["assets"][0]["name"] = "xueer-ear-1.2.2-windows-x64-portable.zip".into();
        assert!(parse(&encoded(&value), "1.2.2").unwrap().is_none());
    }

    #[test]
    fn tags_cannot_inject_paths_queries_or_non_stable_versions() {
        for tag in [
            "v1.2.3/../../evil",
            "v1.2.3?redirect=evil",
            "v1.2.3%2Fbad",
            "v1.2.3\n",
            "v1.2.3-beta",
            "1.2.3+build",
            "1.2",
            "1.2.3.4",
            "01.2.3",
            "1.-2.3",
            "1.2.18446744073709551616",
        ] {
            assert!(parse(&encoded(&fixture(tag)), "1.0.0").is_err());
        }
        assert!(parse(&encoded(&fixture("v1.2.3")), "bad-current").is_err());
    }

    #[test]
    fn release_json_is_bounded_and_malformed_flags_are_errors() {
        assert!(parse(&vec![b' '; MAX_BODY + 1], "1.0.0").is_err());
        assert!(parse(b"{", "1.0.0").is_err());
        let mut value = fixture("v1.2.3");
        value["draft"] = "false".into();
        assert!(parse(&encoded(&value), "1.0.0").is_err());
    }

    struct Files {
        root: PathBuf,
        path: PathBuf,
    }
    impl Files {
        fn new() -> Self {
            let time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "xueer-ear-update-test-{}-{time}-{}",
                std::process::id(),
                TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            Self {
                path: root.join("update-preferences.json"),
                root,
            }
        }
    }
    impl Drop for Files {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
            let _ = fs::remove_dir(&self.root);
        }
    }

    #[test]
    fn skipped_version_persists_normalizes_and_can_be_cleared_atomically() {
        let files = Files::new();
        assert_eq!(
            Preferences::load_path(&files.path).unwrap(),
            Preferences::default()
        );
        Preferences {
            skipped_version: Some("v0.1.10".into()),
        }
        .save_path(&files.path)
        .unwrap();
        assert_eq!(
            Preferences::load_path(&files.path)
                .unwrap()
                .skipped_version
                .as_deref(),
            Some("0.1.10")
        );
        let before = fs::read(&files.path).unwrap();
        assert!(
            Preferences {
                skipped_version: Some("bad\nversion".into())
            }
            .save_path(&files.path)
            .is_err()
        );
        assert_eq!(fs::read(&files.path).unwrap(), before);
        Preferences::default().save_path(&files.path).unwrap();
        assert_eq!(
            Preferences::load_path(&files.path).unwrap(),
            Preferences::default()
        );
        assert_eq!(fs::read_dir(&files.root).unwrap().count(), 1);
    }

    #[test]
    fn corrupt_and_oversized_preferences_are_not_silently_overwritten() {
        let files = Files::new();
        for body in [
            b"{".as_slice(),
            br#"{"skipped_version":"v1.2.3-beta"}"#,
            br#"{"skipped_version":3}"#,
        ] {
            fs::write(&files.path, body).unwrap();
            assert!(Preferences::load_path(&files.path).is_err());
        }
        fs::write(&files.path, vec![b' '; MAX_PREFERENCES + 1]).unwrap();
        assert!(Preferences::load_path(&files.path).is_err());
    }
}
