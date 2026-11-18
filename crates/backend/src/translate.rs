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
    /// 智谱免费模型（和工具站同一把），谷歌限流时的第三备用
    pub zhipu_key: String,
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
        self.azure_key.trim().is_empty()
            && self.google_key.trim().is_empty()
            && self.zhipu_key.trim().is_empty()
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

/// 一次请求最多送这么多字；更长的按段落（段太长再按句子）切开，逐段翻译再拼回去。
/// 段落很短也会先合并到接近这个长度再送，免得一篇几十段的文章发几十次请求撞上免费档的每分钟次数
pub const CHUNK_CHARS: usize = 1500;
/// 整段最长多少字（输入框上限，约 5 万汉字，正常用不到）
pub const MAX_CHARS: usize = 50_000;

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
const ZHIPU_MODEL: &str = "glm-4.7-flash";

pub fn zhipu_request(keys: &Keys, text: &str, target: Target) -> Option<Request> {
    let key = keys.zhipu_key.trim();
    if key.is_empty() {
        return None;
    }
    let mut request = chat_request(key, text, target, ZHIPU_MODEL);
    request.host = "open.bigmodel.cn";
    request.path = "/api/paas/v4/chat/completions".into();
    let mut body: serde_json::Value = serde_json::from_str(&request.body).ok()?;
    body.as_object_mut()?.remove("reasoning_effort");
    body["thinking"] = serde_json::json!({"type": "disabled"});
    request.body = body.to_string();
    Some(request)
}

pub fn google_request(keys: &Keys, text: &str, target: Target) -> Option<Request> {
    let key = keys.google_key.trim();
    if key.is_empty() {
        return None;
    }
    Some(chat_request(key, text, target, GOOGLE_MODEL))
}

fn chat_request(key: &str, text: &str, target: Target, model: &str) -> Request {
    let system = format!(
        "Translate the user's text into {}. Keep line breaks, names and code as they are. \
         The text is spoken dictation: drop filler words, keep the meaning. Output only the translation.",
        target.name()
    );
    Request {
        host: "generativelanguage.googleapis.com",
        path: "/v1beta/openai/chat/completions".into(),
        headers: vec![
            ("Authorization".into(), format!("Bearer {key}")),
            ("Content-Type".into(), "application/json".into()),
        ],
        body: serde_json::json!({
            "model": model,
            "reasoning_effort": "low",
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": text},
            ],
        })
        .to_string(),
    }
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

/// 限流后等几秒再试（测试里不真等）
fn pause(seconds: u64) {
    #[cfg(not(test))]
    std::thread::sleep(std::time::Duration::from_secs(seconds));
    #[cfg(test)]
    let _ = seconds;
}

/// HTTP 状态码 → 给主人看的一句原因
pub fn status_reason(service: &str, status: u32) -> String {
    match status {
        401 | 403 => format!("{service} 的 Key 不对或没开通"),
        429 => format!("{service} 一分钟内用得太多，等一会儿再试"),
        _ => format!("{service} 出错（HTTP {status}）"),
    }
}

/// 按顺序试：微软 → 谷歌。`send` 负责真正发请求，返回（状态码, 响应体）。
/// 都没配 Key、文字太长或都失败时返回原因（最后一个失败的原因）。
pub fn translate_with(
    keys: &Keys,
    text: &str,
    send: impl FnMut(&Request) -> Result<(u32, Vec<u8>), String>,
) -> Result<String, String> {
    translate_to(keys, text, Target::for_text(text.trim()), send)
}

/// 指定译成哪种语言（翻译小窗手动选了方向时用）
pub fn translate_to(
    keys: &Keys,
    text: &str,
    target: Target,
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
    // 长文按段切开逐段翻（2026-10 工具站长文翻译失败的教训：一次送太长会被截断或超时）
    let mut out = String::new();
    for (piece, separator) in merge(text, chunks(text, CHUNK_CHARS), CHUNK_CHARS) {
        let translated = translate_piece(keys, piece, target, &mut send)?;
        // 译成英文时，两段在同一行接上要隔一个空格（「code.The」）
        if target == Target::English
            && !out.is_empty()
            && !out.ends_with(char::is_whitespace)
            && !translated.starts_with(char::is_whitespace)
        {
            out.push(' ');
        }
        out.push_str(&translated);
        out.push_str(separator);
    }
    Ok(out)
}

/// 切成不超过 `limit` 字的段：先按换行切；单行太长按句末标点切；一句还太长就按字数硬切。
/// 返回（这段文字, 它后面原来的分隔符），把它们依次拼回去和原文一字不差。
pub fn chunks(text: &str, limit: usize) -> Vec<(&str, &str)> {
    let limit = limit.max(1);
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let (line, newline, after) = match rest.find('\n') {
            Some(i) => (&rest[..i], &rest[i..i + 1], &rest[i + 1..]),
            None => (rest, "", ""),
        };
        let mut pieces = split_line(line, limit);
        match pieces.last_mut() {
            Some(last) => last.1 = newline,
            None => out.push((line, newline)),
        }
        out.extend(pieces);
        rest = after;
    }
    out
}

/// 把相邻的短段合并成不超过 `limit` 字的一批（中间的换行原样留在批里，译文里也会保留）。
/// 返回的每批都是原文里连续的一段，拼回去和原文一样。
pub fn merge<'a>(
    text: &'a str,
    parts: Vec<(&'a str, &'a str)>,
    limit: usize,
) -> Vec<(&'a str, &'a str)> {
    let base = text.as_ptr() as usize;
    let mut out: Vec<(&str, &str)> = Vec::new();
    let mut start: Option<usize> = None;
    let mut end = 0;
    let mut count = 0;
    for (piece, separator) in parts {
        let at = piece.as_ptr() as usize - base;
        let piece_end = at + piece.len();
        let size = piece.chars().count() + separator.chars().count();
        if let Some(s) = start {
            if count + size > limit {
                out.push((&text[s..end], ""));
                start = None;
                count = 0;
            }
        }
        if start.is_none() {
            start = Some(at);
        }
        end = piece_end + separator.len();
        count += size;
    }
    if let Some(s) = start {
        out.push((&text[s..end], ""));
    }
    // 每批末尾的换行挪到分隔符里（译文常把结尾换行吃掉）
    out.into_iter()
        .map(|(batch, _)| {
            let trimmed = batch.trim_end_matches(['\n', '\r']);
            (trimmed, &batch[trimmed.len()..])
        })
        .collect()
}

/// 一行切成不超过 `limit` 字的几段（分隔符都是空的，切点尽量落在句末标点后面）
fn split_line(line: &str, limit: usize) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut start = 0;
    while line[start..].chars().count() > limit {
        let window: Vec<(usize, char)> = line[start..].char_indices().take(limit).collect();
        let hard = start + window.last().map_or(0, |&(i, c)| i + c.len_utf8());
        let cut = window
            .iter()
            .rev()
            .find(|(_, c)| "。！？.!?；;".contains(*c))
            .map(|&(i, c)| start + i + c.len_utf8())
            .filter(|&cut| cut > start)
            .unwrap_or(hard);
        out.push((&line[start..cut], ""));
        start = cut;
    }
    if start < line.len() {
        out.push((&line[start..], ""));
    }
    out
}

fn translate_piece(
    keys: &Keys,
    text: &str,
    target: Target,
    send: &mut impl FnMut(&Request) -> Result<(u32, Vec<u8>), String>,
) -> Result<String, String> {
    if text.trim().is_empty() {
        return Ok(text.to_string());
    }
    let attempts: [(&str, Option<Request>, fn(&[u8]) -> Result<String, String>); 3] = [
        ("微软翻译", azure_request(keys, text, target), parse_azure),
        ("谷歌", google_request(keys, text, target), parse_google),
        ("智谱", zhipu_request(keys, text, target), parse_google),
    ];
    let mut reason = String::new();
    // 免费档每分钟次数有限（谷歌约 10 次）。为了体感，被限流的那家先跳过、马上换下一家；
    // 只有每一家都被限流时才等一会儿再从头试（8 秒、20 秒）。主人 2026-11-19：不能影响体感
    for wait in [0, 8, 20] {
        if wait > 0 {
            pause(wait);
        }
        let mut limited = false;
        for (service, request, parse) in &attempts {
            let Some(request) = request else { continue };
            match send(request) {
                Ok((200, body)) => match parse(&body) {
                    Ok(text) => return Ok(text),
                    Err(error) => reason = error,
                },
                Ok((429, _)) => {
                    reason = status_reason(service, 429);
                    limited = true;
                }
                Ok((status, _)) => reason = status_reason(service, status),
                Err(error) => reason = format!("{service} 连不上（{error}）"),
            }
        }
        // 不是限流（Key 不对、连不上）的错，等也没用
        if !limited {
            break;
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
            zhipu_key: String::new(),
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
            ],
            "微软限流马上换谷歌"
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
    fn rate_limited_service_is_retried_then_the_next_one_is_used() {
        let keys = Keys {
            google_key: "g".into(),
            zhipu_key: "z".into(),
            ..Keys::default()
        };
        let mut google = 0;
        let out = translate_with(&keys, "你好", |r| {
            if r.host == "generativelanguage.googleapis.com" {
                google += 1;
                return Ok((429, vec![]));
            }
            assert!(r.body.contains("glm-4.7-flash") && !r.body.contains("reasoning_effort"));
            Ok((
                200,
                br#"{"choices":[{"message":{"content":"Hello"}}]}"#.to_vec(),
            ))
        });
        assert_eq!(out.as_deref(), Ok("Hello"));
        assert_eq!(google, 1, "谷歌限流就马上换智谱，不干等");
        // 第二次重试成功就不用换
        let mut calls = 0;
        let out = translate_with(&keys, "你好", |_| {
            calls += 1;
            Ok(if calls < 2 {
                (429, vec![])
            } else {
                (
                    200,
                    br#"{"choices":[{"message":{"content":"Hi"}}]}"#.to_vec(),
                )
            })
        });
        assert_eq!(out.as_deref(), Ok("Hi"));
    }

    #[test]
    fn an_explicit_direction_wins_over_detection() {
        let keys = Keys {
            azure_key: "a".into(),
            ..Keys::default()
        };
        // 原文是中文，但主人把方向反转成「英 → 中」：照样按译成中文发
        let out = translate_to(&keys, "你好", Target::Chinese, |r| {
            assert!(r.path.ends_with("to=zh-Hans"), "{}", r.path);
            Ok((
                200,
                r#"[{"translations":[{"text":"你好"}]}]"#.as_bytes().to_vec(),
            ))
        });
        assert_eq!(out.as_deref(), Ok("你好"));
    }

    #[test]
    fn only_when_every_service_is_limited_does_it_wait_and_retry() {
        let keys = Keys {
            google_key: "g".into(),
            zhipu_key: "z".into(),
            ..Keys::default()
        };
        let mut calls = Vec::new();
        let out = translate_with(&keys, "你好", |r| {
            calls.push(r.host);
            // 第一轮两家都限流，第二轮谷歌好了
            Ok(if calls.len() <= 2 {
                (429, vec![])
            } else {
                (
                    200,
                    br#"{"choices":[{"message":{"content":"Hi"}}]}"#.to_vec(),
                )
            })
        });
        assert_eq!(out.as_deref(), Ok("Hi"));
        assert_eq!(
            calls,
            [
                "generativelanguage.googleapis.com",
                "open.bigmodel.cn",
                "generativelanguage.googleapis.com"
            ]
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
    fn long_text_is_split_by_paragraph_and_sentence_and_joined_back() {
        let text = "第一段。\n第二段很长。第二段第二句。\n\n第三段";
        let parts = chunks(text, 8);
        let joined: String = parts.iter().map(|(p, s)| format!("{p}{s}")).collect();
        assert_eq!(joined, text, "切开再拼回去要和原文一样");
        assert!(
            parts.iter().all(|(p, _)| p.chars().count() <= 8),
            "{parts:?}"
        );
        // 每段单独去翻，结果按原来的换行拼
        let keys = Keys {
            google_key: "g".into(),
            ..Keys::default()
        };
        let mut calls = 0;
        let out = translate_with(&keys, text, |r| {
            calls += 1;
            let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
            let said = body["messages"][1]["content"].as_str().unwrap().to_string();
            let reply =
                serde_json::json!({"choices":[{"message":{"content": format!("<{said}>")}}]});
            Ok((200, reply.to_string().into_bytes()))
        })
        .unwrap();
        // 短文合成一批送一次，换行留在里面
        assert_eq!(calls, 1);
        assert_eq!(out, format!("<{text}>"));
    }

    #[test]
    fn short_paragraphs_are_sent_together() {
        // 主人粘过的说明文字：几十个短段，一段一请求会撞上谷歌每分钟 10 次
        let text = "第一段。\n\n- 第二条\n- 第三条\n\n**第四段**\n".repeat(20);
        let batches = merge(&text, chunks(&text, CHUNK_CHARS), CHUNK_CHARS);
        assert_eq!(batches.len(), 1, "{batches:?}");
        let joined: String = batches.iter().map(|(p, s)| format!("{p}{s}")).collect();
        assert_eq!(joined, text);
        let long = "很长的一句话没有标点".repeat(400);
        let batches = merge(&long, chunks(&long, 1500), 1500);
        assert!(batches.len() >= 3 && batches.iter().all(|(p, _)| p.chars().count() <= 1500));
    }

    #[test]
    fn a_ten_thousand_character_text_is_accepted() {
        let text = "今天吃什么。".repeat(1700);
        let keys = Keys {
            google_key: "g".into(),
            ..Keys::default()
        };
        let out = translate_with(&keys, &text, |r| {
            assert!(r.body.chars().count() < CHUNK_CHARS * 2 + 600);
            Ok((
                200,
                br#"{"choices":[{"message":{"content":"x"}}]}"#.to_vec(),
            ))
        });
        // 同一行切开的几段，英文译文之间隔一个空格
        assert_eq!(
            out.as_deref().map(|t| t.contains("x x")),
            Ok(true),
            "{out:?}"
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
    translate_with(&local_keys()?, text, send)
}

/// 译成指定语言
#[cfg(windows)]
pub fn translate_into(text: &str, target: Target) -> Result<String, String> {
    translate_to(&local_keys()?, text, target, send)
}

/// 本机 %LOCALAPPDATA%\XueerEar\translate.json 里的 Key
pub fn local_keys() -> Result<Keys, String> {
    let dir = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA 不可用")?;
    Keys::load(&std::path::PathBuf::from(dir).join("XueerEar/translate.json"))
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

#[cfg(test)]
mod chunk_fuzz {
    use super::*;

    #[test]
    fn chunks_never_panic_and_always_rejoin_exactly() {
        // 主人粘过的那种：中英混排、markdown、空行、全角标点、很长的段
        let pieces = [
            "雪耳 1.1.8 已经装好。",
            "**翻译小窗**\n",
            "- 两个框都能滑。",
            "\n\n",
            "Ctrl+A、Ctrl+C",
            "（推测）",
            "“引号”",
            "a.b!c?",
            "😀",
            "\r\n",
            "一",
            "。",
            "\n",
            " ",
            "——",
            "…",
        ];
        let mut seed = 7u64;
        for round in 0..3000 {
            let mut text = String::new();
            let n = (round % 400) + 1;
            for _ in 0..n {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                text.push_str(pieces[(seed >> 33) as usize % pieces.len()]);
            }
            for limit in [1, 2, 3, 7, 50, CHUNK_CHARS] {
                let parts = chunks(&text, limit);
                let joined: String = parts.iter().map(|(p, s)| format!("{p}{s}")).collect();
                assert_eq!(joined, text, "limit {limit} round {round}");
                assert!(parts.iter().all(|(p, _)| p.chars().count() <= limit));
            }
        }
    }
}

/// 边打字边自动翻译的节奏（主人 2026-11-19 定）：这次打字已经自动翻了几次 → 下一次至少隔多久。
/// 第 1～3 次隔 1 秒、4～6 次 2 秒、7～9 次 4 秒、之后 8 秒；按回车、点「翻译」、清空输入框后从头算。
/// 一分钟最多自动翻十来次，不会撞上谷歌免费档每分钟约 10 次的上限。
#[derive(Debug, Default, Clone)]
pub struct Pace {
    done: u32,
}

impl Pace {
    /// 下一次自动翻译离上一次至少隔几毫秒
    pub fn gap_ms(&self) -> u32 {
        match self.done {
            0..=2 => 1000,
            3..=5 => 2000,
            6..=8 => 4000,
            _ => 8000,
        }
    }
    /// 自动翻了一次
    pub fn auto(&mut self) {
        self.done += 1;
    }
    /// 按回车、点按钮、清空：从头算
    pub fn reset(&mut self) {
        self.done = 0;
    }
}

#[cfg(test)]
mod pace_tests {
    use super::Pace;

    #[test]
    fn gaps_grow_with_each_automatic_translation_and_reset() {
        let mut pace = Pace::default();
        let mut gaps = Vec::new();
        for _ in 0..11 {
            gaps.push(pace.gap_ms());
            pace.auto();
        }
        assert_eq!(
            gaps,
            [
                1000, 1000, 1000, 2000, 2000, 2000, 4000, 4000, 4000, 8000, 8000
            ]
        );
        pace.reset();
        assert_eq!(pace.gap_ms(), 1000);
        // 一分钟里自动翻不超过十来次
        let mut pace = Pace::default();
        let (mut t, mut n) = (0, 0);
        while t + pace.gap_ms() <= 60_000 {
            t += pace.gap_ms();
            pace.auto();
            n += 1;
        }
        assert!(n <= 14, "{n}");
    }
}
