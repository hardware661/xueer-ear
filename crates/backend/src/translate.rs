//! 翻译（主人 2026-11-19）：微软翻译做主力，谷歌 Gemini 免费 Key 做备用。
//! Key 只在本机 `%LOCALAPPDATA%\XueerEar\translate.json`，不进仓库：
//! `{"azure_key": "…", "azure_region": "eastasia", "google_key": "…"}`，哪个没填就跳过哪个。
//! 中文翻成英文；原文基本是英文时翻成中文（翻译小窗用）。
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Keys {
    pub azure_key: String,
    pub azure_region: String,
    pub google_key: String,
}

impl Keys {
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read(path) {
            Ok(bytes) if bytes.len() <= 8 * 1024 => {
                serde_json::from_slice(&bytes).map_err(|_| "翻译设置文件坏了".to_string())
            }
            Ok(_) => Err("翻译设置文件太大".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(_) => Err("读不了翻译设置".into()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.azure_key.trim().is_empty() && self.google_key.trim().is_empty()
    }
}

/// 译成哪种语言：英文为主的原文翻成中文，其他翻成英文
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    English,
    Chinese,
}

impl Target {
    pub fn for_text(text: &str) -> Self {
        let cjk = text.chars().filter(|c| is_cjk(*c)).count();
        let latin = text.chars().filter(|c| c.is_ascii_alphabetic()).count();
        if cjk == 0 && latin > 0 {
            Self::Chinese
        } else {
            Self::English
        }
    }
    fn azure_code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Chinese => "zh-Hans",
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::English => "natural English",
            Self::Chinese => "简体中文",
        }
    }
}

fn is_cjk(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{f900}'..='\u{faff}')
}

/// 一次 HTTPS 请求：主机、路径、请求头、请求体
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub host: &'static str,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// 单次翻译最长多少字（大段话够用；更长的请用翻译小窗分段）
pub const MAX_CHARS: usize = 5000;

pub fn azure_request(keys: &Keys, text: &str, target: Target) -> Option<Request> {
    let key = keys.azure_key.trim();
    if key.is_empty() {
        return None;
    }
    let mut headers = vec![
        ("Ocp-Apim-Subscription-Key".to_string(), key.to_string()),
        (
            "Content-Type".into(),
            "application/json; charset=UTF-8".into(),
        ),
    ];
    let region = keys.azure_region.trim();
    if !region.is_empty() {
        headers.push(("Ocp-Apim-Subscription-Region".into(), region.to_string()));
    }
    Some(Request {
        host: "api.cognitive.microsofttranslator.com",
        path: format!("/translate?api-version=3.0&to={}", target.azure_code()),
        headers,
        body: serde_json::json!([{ "Text": text }]).to_string(),
    })
}

pub fn parse_azure(body: &[u8]) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| "微软翻译返回的内容看不懂".to_string())?;
    value[0]["translations"][0]["text"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "微软翻译没给结果".to_string())
}

const GOOGLE_MODEL: &str = "gemini-3.5-flash-lite";

pub fn google_request(keys: &Keys, text: &str, target: Target) -> Option<Request> {
    let key = keys.google_key.trim();
    if key.is_empty() {
        return None;
    }
    let system = format!(
        "Translate the user's text into {}. Keep line breaks, names and code as they are. \
         The text is spoken dictation: drop filler words, keep the meaning. Output only the translation.",
        target.name()
    );
    Some(Request {
        host: "generativelanguage.googleapis.com",
        path: "/v1beta/openai/chat/completions".into(),
        headers: vec![
            ("Authorization".into(), format!("Bearer {key}")),
            ("Content-Type".into(), "application/json".into()),
        ],
        body: serde_json::json!({
            "model": GOOGLE_MODEL,
            "reasoning_effort": "low",
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": text},
            ],
        })
        .to_string(),
    })
}

pub fn parse_google(body: &[u8]) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| "谷歌翻译返回的内容看不懂".to_string())?;
    value["choices"][0]["message"]["content"]
        .as_str()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| "谷歌翻译没给结果".to_string())
}

/// HTTP 状态码 → 给主人看的一句原因
pub fn status_reason(service: &str, status: u32) -> String {
    match status {
        401 | 403 => format!("{service} 的 Key 不对或没开通"),
        429 => format!("{service} 额度用完或太频繁"),
        _ => format!("{service} 出错（HTTP {status}）"),
    }
}

/// 按顺序试：微软 → 谷歌。`send` 负责真正发请求，返回（状态码, 响应体）。
/// 都没配 Key、文字太长或都失败时返回原因（最后一个失败的原因）。
pub fn translate_with(
    keys: &Keys,
    text: &str,
    mut send: impl FnMut(&Request) -> Result<(u32, Vec<u8>), String>,
) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("没有要翻译的字".into());
    }
    if text.chars().count() > MAX_CHARS {
        return Err(format!("一次最多翻译 {MAX_CHARS} 字"));
    }
    if keys.is_empty() {
        return Err("还没配翻译 Key".into());
    }
    let target = Target::for_text(text);
    let attempts: [(&str, Option<Request>, fn(&[u8]) -> Result<String, String>); 2] = [
        ("微软翻译", azure_request(keys, text, target), parse_azure),
        ("谷歌", google_request(keys, text, target), parse_google),
    ];
    let mut reason = String::new();
    for (service, request, parse) in attempts {
        let Some(request) = request else { continue };
        match send(&request) {
            Ok((200, body)) => match parse(&body) {
                Ok(text) => return Ok(text),
                Err(error) => reason = error,
            },
            Ok((status, _)) => reason = status_reason(service, status),
            Err(error) => reason = format!("{service} 连不上（{error}）"),
        }
    }
    Err(reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn both() -> Keys {
        Keys {
            azure_key: "a".into(),
            azure_region: "eastasia".into(),
            google_key: "g".into(),
        }
    }

    #[test]
    fn chinese_goes_to_english_and_english_to_chinese() {
        assert_eq!(Target::for_text("今天吃什么"), Target::English);
        assert_eq!(Target::for_text("用 Claude Code 跑测试"), Target::English);
        assert_eq!(Target::for_text("What to eat today?"), Target::Chinese);
    }

    #[test]
    fn azure_is_tried_first_and_google_is_the_fallback() {
        let mut hosts = Vec::new();
        let out = translate_with(&both(), "你好", |r| {
            hosts.push(r.host);
            Ok(match r.host {
                "api.cognitive.microsofttranslator.com" => (429, vec![]),
                _ => (
                    200,
                    br#"{"choices":[{"message":{"content":" Hello \n"}}]}"#.to_vec(),
                ),
            })
        });
        assert_eq!(out.as_deref(), Ok("Hello"));
        assert_eq!(
            hosts,
            [
                "api.cognitive.microsofttranslator.com",
                "generativelanguage.googleapis.com"
            ]
        );
    }

    #[test]
    fn azure_answer_is_used_when_it_works() {
        let out = translate_with(&both(), "你好", |r| {
            assert!(r.path.ends_with("to=en"));
            assert!(
                r.headers
                    .iter()
                    .any(|(k, v)| k == "Ocp-Apim-Subscription-Region" && v == "eastasia")
            );
            Ok((
                200,
                br#"[{"translations":[{"text":"Hello","to":"en"}]}]"#.to_vec(),
            ))
        });
        assert_eq!(out.as_deref(), Ok("Hello"));
    }

    #[test]
    fn only_configured_services_are_called() {
        let keys = Keys {
            google_key: "g".into(),
            ..Keys::default()
        };
        let mut calls = 0;
        let out = translate_with(&keys, "你好", |_| {
            calls += 1;
            Err("timeout".into())
        });
        assert_eq!(calls, 1);
        assert_eq!(out, Err("谷歌 连不上（timeout）".into()));
        assert_eq!(
            translate_with(&Keys::default(), "你好", |_| unreachable!()),
            Err("还没配翻译 Key".into())
        );
    }

    #[test]
    fn bad_key_reason_is_plain() {
        let keys = Keys {
            azure_key: "a".into(),
            ..Keys::default()
        };
        assert_eq!(
            translate_with(&keys, "你好", |_| Ok((401, vec![]))),
            Err("微软翻译 的 Key 不对或没开通".into())
        );
    }

    #[test]
    fn missing_settings_file_means_no_keys() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            Keys::load(&dir.path().join("translate.json")),
            Ok(Keys::default())
        );
        std::fs::write(dir.path().join("t.json"), "{\"google_key\":\"g\"}").unwrap();
        assert_eq!(
            Keys::load(&dir.path().join("t.json")).unwrap().google_key,
            "g"
        );
        std::fs::write(dir.path().join("bad.json"), "{").unwrap();
        assert!(Keys::load(&dir.path().join("bad.json")).is_err());
    }
}

/// 用本机 WinHTTP 发一次 HTTPS POST（走系统代理），返回（状态码, 响应体）
#[cfg(windows)]
pub fn send(request: &Request) -> Result<(u32, Vec<u8>), String> {
    native::post(request)
}

/// 读本机翻译 Key，按微软 → 谷歌翻译
#[cfg(windows)]
pub fn translate(text: &str) -> Result<String, String> {
    let dir = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA 不可用")?;
    let keys = Keys::load(&std::path::PathBuf::from(dir).join("XueerEar/translate.json"))?;
    translate_with(&keys, text, send)
}

#[cfg(windows)]
mod native {
    use super::Request;
    use std::{ffi::c_void, mem, ptr};
    use windows_sys::Win32::{Foundation::GetLastError, Networking::WinHttp::*};

    const MAX_BODY: usize = 1024 * 1024;

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
        format!("系统错误 {}", unsafe { GetLastError() })
    }
    fn check(value: i32) -> Result<(), String> {
        if value == 0 { Err(error()) } else { Ok(()) }
    }

    pub(super) fn post(request: &Request) -> Result<(u32, Vec<u8>), String> {
        unsafe {
            let session = Handle::checked(WinHttpOpen(
                wide("xueer-ear").as_ptr(),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                ptr::null(),
                ptr::null(),
                0,
            ))?;
            check(WinHttpSetTimeouts(session.0, 5000, 5000, 15000, 30000))?;
            let connection = Handle::checked(WinHttpConnect(
                session.0,
                wide(request.host).as_ptr(),
                INTERNET_DEFAULT_HTTPS_PORT,
                0,
            ))?;
            let handle = Handle::checked(WinHttpOpenRequest(
                connection.0,
                wide("POST").as_ptr(),
                wide(&request.path).as_ptr(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                WINHTTP_FLAG_SECURE,
            ))?;
            let disable = WINHTTP_DISABLE_REDIRECTS | WINHTTP_DISABLE_COOKIES;
            check(WinHttpSetOption(
                handle.0,
                WINHTTP_OPTION_DISABLE_FEATURE,
                (&disable as *const u32).cast(),
                mem::size_of::<u32>() as u32,
            ))?;
            let headers: String = request
                .headers
                .iter()
                .map(|(k, v)| format!("{k}: {v}\r\n"))
                .collect();
            let headers = wide(&headers);
            let body = request.body.as_bytes();
            check(WinHttpSendRequest(
                handle.0,
                headers.as_ptr(),
                (headers.len() - 1) as u32,
                body.as_ptr().cast(),
                body.len() as u32,
                body.len() as u32,
                0,
            ))?;
            check(WinHttpReceiveResponse(handle.0, ptr::null_mut()))?;
            let mut status = 0_u32;
            let mut length = mem::size_of::<u32>() as u32;
            check(WinHttpQueryHeaders(
                handle.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                ptr::null(),
                (&mut status as *mut u32).cast(),
                &mut length,
                ptr::null_mut(),
            ))?;
            let mut out = Vec::new();
            let mut buffer = [0_u8; 8192];
            loop {
                let mut received = 0_u32;
                check(WinHttpReadData(
                    handle.0,
                    buffer.as_mut_ptr().cast(),
                    buffer.len() as u32,
                    &mut received,
                ))?;
                if received == 0 {
                    break;
                }
                let received = (received as usize).min(buffer.len());
                if out.len() + received > MAX_BODY {
                    return Err("返回内容太大".into());
                }
                out.extend_from_slice(&buffer[..received]);
            }
            Ok((status, out))
        }
    }
}
