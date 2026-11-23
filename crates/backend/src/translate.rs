//! 雪耳翻译：长文先切段，交给服务池（`translate_pool`）分段翻：火山、微软、B 站、微信输入法四家免费的，
//! 本机配了 Key 的微软正式版、谷歌 Gemini、智谱排在后面（主人 2026-11-24 定）。
//! Key 和「用不用微信输入法」在 `%LOCALAPPDATA%\XueerEar\translate.json`。
pub use crate::https::Request;
#[cfg(windows)]
use crate::https::send;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Keys {
    pub wetype_enabled: bool,
    pub azure_key: String,
    pub azure_region: String,
    pub google_key: String,
    /// 智谱免费模型（和工具站同一把），谷歌限流时的第三备用
    pub zhipu_key: String,
}

impl Default for Keys {
    fn default() -> Self {
        Self {
            wetype_enabled: true,
            azure_key: String::new(),
            azure_region: String::new(),
            google_key: String::new(),
            zhipu_key: String::new(),
        }
    }
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
}

/// 译成哪种语言：英文为主的原文翻成中文，其他翻成英文
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    English,
    Chinese,
}

impl Target {
    /// 自动方向：有中文字就翻成英文，一个中文字都没有、有英文字母才翻成中文
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

/// 一段最多这么多字；更长的按段落（段太长再按句子）切开，分段翻译再拼回去（主人 2026-11-24：
/// 1 万字切成二三十段，每段各自计时）。段落很短也会先合并到接近这个长度再送，免得发太多次请求
pub const CHUNK_CHARS: usize = 600;
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
        .and_then(usable_text)
        .map(str::to_owned)
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
        .and_then(usable_text)
        .map(str::to_owned)
        .ok_or_else(|| "谷歌翻译没给结果".to_string())
}

fn usable_text(text: &str) -> Option<&str> {
    let text = text.trim();
    (!text.is_empty() && !text.contains('\0')).then_some(text)
}

/// HTTP 状态码 → 给主人看的一句原因
pub fn status_reason(service: &str, status: u32) -> String {
    match status {
        401 | 403 => format!("{service} 的 Key 不对或没开通"),
        429 => format!("{service} 一分钟内用得太多，等一会儿再试"),
        _ => format!("{service} 出错（HTTP {status}）"),
    }
}

#[cfg(any(windows, test))]
fn validated_text(text: &str) -> Result<&str, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("没有要翻译的字".into());
    }
    if text.chars().count() > MAX_CHARS {
        return Err(format!("一次最多翻译 {MAX_CHARS} 字"));
    }
    Ok(text)
}

#[cfg(any(windows, test))]
/// 一批要翻的原文：前面原样保留的换行、正文、后面的分隔符
struct Batch<'a> {
    lead: &'a str,
    body: &'a str,
    separator: &'a str,
}

#[cfg(any(windows, test))]
impl Batch<'_> {
    fn needs_translation(&self) -> bool {
        !self.body.trim().is_empty()
    }

    /// 把这批的译文接到 `out` 后面：原样补回前面的换行和后面的分隔符
    fn join_into(&self, out: &mut String, translated: &str, target: Target) {
        out.push_str(self.lead);
        // 同一行切开的英文译文之间要留空格（「code.The」）。
        if target == Target::English
            && !out.is_empty()
            && !translated.is_empty()
            && !out.ends_with(char::is_whitespace)
            && !translated.starts_with(char::is_whitespace)
        {
            out.push(' ');
        }
        out.push_str(translated);
        out.push_str(self.separator);
    }
}

#[cfg(any(windows, test))]
fn batches(text: &str) -> Vec<Batch<'_>> {
    merge(text, chunks(text, CHUNK_CHARS), CHUNK_CHARS)
        .into_iter()
        .map(|(piece, separator)| {
            let body = piece.trim_start_matches(['\n', '\r']);
            Batch {
                lead: &piece[..piece.len() - body.len()],
                body,
                separator,
            }
        })
        .collect()
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

/// 原文里的一段：原样留着，还是要翻
#[cfg(any(windows, test))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct Part {
    range: std::ops::Range<usize>,
    translate: bool,
}

#[cfg(any(windows, test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lang {
    Chinese,
    English,
    /// 没有文字：标点、数字、空白、换行
    Neutral,
}

#[cfg(any(windows, test))]
fn lang_of(piece: &str) -> Lang {
    if piece.chars().any(is_cjk) {
        Lang::Chinese
    } else if piece.chars().any(|c| c.is_ascii_alphabetic()) {
        Lang::English
    } else {
        Lang::Neutral
    }
}

#[cfg(any(windows, test))]
/// 切成可以单独判断语言的小块：中文的句号、问号、分号、逗号、冒号后面断开；
/// 英文的标点后面跟着空白或中文时断开（「3.5」「1,000」不断）；换行单独成一块
fn pieces(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let end = i + c.len_utf8();
        let next = chars.peek().map(|&(_, n)| n);
        if c == '\n' {
            if start < i {
                out.push(start..i);
            }
            out.push(i..end);
            start = end;
            continue;
        }
        let cut = match c {
            '。' | '！' | '？' | '；' | '，' | '：' => true,
            '.' | '!' | '?' | ';' | ',' | ':' => {
                next.is_none_or(|n| n.is_whitespace() || is_cjk(n))
            }
            _ => false,
        };
        if cut {
            out.push(start..end);
            start = end;
        }
    }
    if start < text.len() {
        out.push(start..text.len());
    }
    out
}

#[cfg(any(windows, test))]
/// 译成 `target` 时哪几段要翻（主人 2026-11-21：翻译成英文就全部变成英文，已经是英文的部分不动；
/// 翻译成中文同理）。另一种语言的小块连成一段整段去翻，这样微信输入法自动判断的方向一定对；
/// 没有文字的小块跟着前一段走；要翻的段首尾的空白和换行留在外面原样保留
fn plan(text: &str, target: Target) -> Vec<Part> {
    let mut parts: Vec<Part> = Vec::new();
    for range in pieces(text) {
        let translate = match lang_of(&text[range.clone()]) {
            Lang::Chinese => target == Target::English,
            Lang::English => target == Target::Chinese,
            Lang::Neutral => parts.last().is_some_and(|last| last.translate),
        };
        match parts.last_mut() {
            Some(last) if last.translate == translate => last.range.end = range.end,
            _ => parts.push(Part { range, translate }),
        }
    }
    let mut out: Vec<Part> = Vec::new();
    let keep = |range: std::ops::Range<usize>, out: &mut Vec<Part>| {
        if range.is_empty() {
            return;
        }
        match out.last_mut() {
            Some(last) if !last.translate => last.range.end = range.end,
            _ => out.push(Part {
                range,
                translate: false,
            }),
        }
    };
    for part in parts {
        if !part.translate {
            keep(part.range, &mut out);
            continue;
        }
        let piece = &text[part.range.clone()];
        let body = piece.trim();
        if body.is_empty() {
            keep(part.range, &mut out);
            continue;
        }
        let lead = piece.len() - piece.trim_start().len();
        let start = part.range.start + lead;
        let end = start + body.len();
        keep(part.range.start..start, &mut out);
        out.push(Part {
            range: start..end,
            translate: true,
        });
        keep(end..part.range.end, &mut out);
    }
    out
}

#[cfg(any(windows, test))]
/// 把一段（原文或译文）接到 `out` 后面。译成英文时，英文和英文之间补一个空格，
/// 接在英文后面的中文逗号、冒号、句号换成英文的
fn push_part(out: &mut String, part: &str, target: Target) {
    if target == Target::English
        && part.chars().next().is_some_and(|c| c.is_alphanumeric())
        && let Some(last) = out.chars().last()
    {
        let ascii = |c: char| match c {
            '，' => Some(','),
            '：' => Some(':'),
            '。' => Some('.'),
            '；' => Some(';'),
            _ => None,
        };
        if let Some(swap) = ascii(last) {
            out.pop();
            out.push(swap);
            out.push(' ');
        } else if !last.is_whitespace() && last.is_ascii() && !"([{\"'".contains(last) {
            out.push(' ');
        }
    }
    out.push_str(part);
}

#[cfg(any(windows, test))]
/// 把英文段落里的硬换行并回空格：一行够长（40 字以上，硬换行一般在 70～80 列）、以字母、数字或逗号结尾，
/// 下一行以小写字母开头，说明这句话被拦腰折断了。空行（分段）、短行的清单、标题、句末换行都原样保留；
/// `\r\n` 当一个换行
fn unwrap_english(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = String::with_capacity(text.len());
    for (i, line) in lines.iter().enumerate() {
        let body = line.strip_suffix('\r').unwrap_or(line);
        out.push_str(body);
        let Some(next) = lines.get(i + 1) else {
            if body.len() != line.len() {
                out.push('\r');
            }
            continue;
        };
        let ends_mid_sentence = body.trim().chars().count() >= 40
            && body
                .trim_end()
                .chars()
                .last()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == ',' || c == ';' || c == '-');
        let next_continues = next
            .trim_start()
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase());
        let english = !body.chars().any(is_cjk) && !next.chars().any(is_cjk);
        if english && ends_mid_sentence && next_continues {
            if !body.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.push_str(&line[body.len()..]);
            out.push('\n');
        }
    }
    out
}

/// 开发排查：把原文按 `size` 字切批，只问微信输入法，打印每批的字数、换行数、用时、结果。
/// `parallel` 是同时在途的请求数（同一条连接）
#[cfg(windows)]
pub fn probe_wetype(text: &str, size: usize, parallel: usize) -> Result<(), String> {
    use std::time::{Duration, Instant};
    let text = validated_text(text)?;
    let started = Instant::now();
    let mut client = crate::wetype::Client::from_default_device()
        .map_err(|_| "连不上微信输入法翻译".to_owned())?;
    println!("connected {} ms", started.elapsed().as_millis());
    let pieces: Vec<&str> = merge(text, chunks(text, size), size)
        .into_iter()
        .map(|(piece, _)| piece)
        .collect();
    let lines = |t: &str| t.bytes().filter(|&c| c == b'\n').count();
    let mut next = 0;
    let mut flying: Vec<(u32, usize, Instant)> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(120);
    while (next < pieces.len() || !flying.is_empty()) && Instant::now() < deadline {
        while next < pieces.len() && flying.len() < parallel.max(1) {
            let id = client
                .send_translation(pieces[next])
                .map_err(|e| e.to_string())?;
            flying.push((id, next, Instant::now()));
            next += 1;
        }
        let Some((id, reply)) = client.poll_translation().map_err(|e| e.to_string())? else {
            continue;
        };
        let Some(at) = flying.iter().position(|f| f.0 == id) else {
            continue;
        };
        let (_, n, sent) = flying.remove(at);
        let piece = pieces[n];
        match reply {
            Ok(reply) => println!(
                "batch {n}: chars={} lines={} -> chars={} lines={} {} ms (at {} ms) | {:?}",
                piece.chars().count(),
                lines(piece),
                reply.chars().count(),
                lines(&reply),
                sent.elapsed().as_millis(),
                started.elapsed().as_millis(),
                reply.chars().take(60).collect::<String>()
            ),
            Err(error) => println!("batch {n}: error {error} {} ms", sent.elapsed().as_millis()),
        }
    }
    println!(
        "total {} ms, unanswered {}",
        started.elapsed().as_millis(),
        flying.len()
    );
    Ok(())
}

#[cfg(any(windows, test))]
/// 译文能不能用：空的、方向不对（该是英文的里一个英文字母都没有）、比原文多出换行、
/// 明显少了一截（原文 60 个字以上、译文不到四成）都不能用，换一家
pub(crate) fn usable_reply(source: &str, target: Target, reply: &str) -> Result<(), String> {
    let reply = usable_text(reply).ok_or("没返回有效译文")?;
    let has_language = source.chars().any(|c| is_cjk(c) || c.is_ascii_alphabetic());
    let has_target = reply.chars().any(|c| match target {
        Target::Chinese => is_cjk(c),
        Target::English => c.is_ascii_alphabetic(),
    });
    if has_language && !has_target {
        return Err("译文方向不对".into());
    }
    // 硬换行的长文（从网页、PDF 复制的）会被并回段落，译文换行变少是正常的；变多不行
    let lines = |t: &str| t.bytes().filter(|&c| c == b'\n').count();
    if lines(reply) > lines(source) {
        return Err("译文多出了换行".into());
    }
    if weight(source) >= 60 && weight(reply) * 5 < weight(source) * 2 {
        return Err("译文像是少了一截".into());
    }
    Ok(())
}

#[cfg(any(windows, test))]
/// 文字量：一个汉字约等于三个英文字母（实测中英互译时这个量大致不变），用来发现译文少了一截
fn weight(text: &str) -> usize {
    text.chars()
        .map(|c| {
            if is_cjk(c) {
                3
            } else {
                usize::from(c.is_ascii_alphanumeric())
            }
        })
        .sum()
}

/// 本机 %LOCALAPPDATA%\XueerEar\translate.json 里的 Key
pub fn local_keys() -> Result<Keys, String> {
    let dir = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA 不可用")?;
    Keys::load(&std::path::PathBuf::from(dir).join("XueerEar/translate.json"))
}

/// 翻译服务（排在前面的四家都免费、不用 Key；后面三家要在 translate.json 里配 Key）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Service {
    /// 火山翻译（浏览器插件用的接口）
    Volcengine,
    /// 微软翻译（Edge 浏览器自带翻译用的接口）
    Microsoft,
    /// B 站 Index-Translate 官方免费接口（小模型）
    Bilibili,
    /// 微信输入法云端翻译
    Wetype,
    /// 微软翻译正式版（要 Key）
    Azure,
    /// 谷歌 Gemini（要 Key）
    Google,
    /// 智谱（要 Key）
    Zhipu,
}

impl Service {
    pub const ALL: [Self; 7] = [
        Self::Volcengine,
        Self::Microsoft,
        Self::Bilibili,
        Self::Wetype,
        Self::Azure,
        Self::Google,
        Self::Zhipu,
    ];

    /// 命令行里用的名字
    pub fn id(self) -> &'static str {
        match self {
            Self::Volcengine => "volcengine",
            Self::Microsoft => "microsoft",
            Self::Bilibili => "bilibili",
            Self::Wetype => "wetype",
            Self::Azure => "azure",
            Self::Google => "google",
            Self::Zhipu => "zhipu",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|service| service.id() == id)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Volcengine => "火山",
            Self::Microsoft => "微软",
            Self::Bilibili => "B站",
            Self::Wetype => "微信",
            Self::Azure => "微软正式版",
            Self::Google => "谷歌",
            Self::Zhipu => "智谱",
        }
    }

    /// 还没实测过时，估计翻一段要多少毫秒（2026-11-24 华为上测的：火山 0.1～0.4 秒、微软 0.4～0.8 秒、
    /// B 站小模型 0.3～1 秒；微信实测 1～2.5 秒）。之后按本机最近的实测排先后
    pub fn expected_ms(self) -> u32 {
        match self {
            Self::Volcengine => 300,
            Self::Microsoft => 600,
            Self::Bilibili => 900,
            Self::Wetype => 1500,
            Self::Azure => 800,
            Self::Google => 2500,
            Self::Zhipu => 3000,
        }
    }
}

/// 这次能用哪些服务：四家免费的（微信可以在 translate.json 里关掉），再加配了 Key 的
pub fn services(keys: &Keys) -> Vec<Service> {
    let mut out = vec![Service::Volcengine, Service::Microsoft, Service::Bilibili];
    if keys.wetype_enabled {
        out.push(Service::Wetype);
    }
    if !keys.azure_key.trim().is_empty() {
        out.push(Service::Azure);
    }
    if !keys.google_key.trim().is_empty() {
        out.push(Service::Google);
    }
    if !keys.zhipu_key.trim().is_empty() {
        out.push(Service::Zhipu);
    }
    out
}

/// 火山：地址结尾要带斜杠（不带是 307 跳转）；源语言留空自动认
pub fn volcengine_request(text: &str, target: Target) -> Request {
    Request {
        host: "translate.volcengine.com",
        path: "/crx/translate/v1/".into(),
        headers: vec![("Content-Type".into(), "application/json".into())],
        body: serde_json::json!({
            "source_language": "",
            "target_language": match target {
                Target::English => "en",
                Target::Chinese => "zh",
            },
            "text": text,
        })
        .to_string(),
    }
}

pub fn parse_volcengine(body: &[u8]) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| "火山返回的内容看不懂".to_string())?;
    let code = value["base_resp"]["status_code"].as_i64().unwrap_or(0);
    if code != 0 {
        return Err(format!("火山出错（{code}）"));
    }
    value["translation"]
        .as_str()
        .and_then(usable_text)
        .map(str::to_owned)
        .ok_or_else(|| "火山没给结果".to_string())
}

/// 微软：Edge 浏览器自带翻译用的接口，不用 Key，请求体是一组字符串
pub fn microsoft_request(text: &str, target: Target) -> Request {
    Request {
        host: "edge.microsoft.com",
        path: format!(
            "/translate/translatetext?from=&to={}&isEnterpriseClient=false",
            target.azure_code()
        ),
        headers: vec![("Content-Type".into(), "application/json".into())],
        body: serde_json::json!([text]).to_string(),
    }
}

/// B 站 Index-Translate（官方免费公开接口，OpenAI 兼容）：小模型最快；关掉思考，只要译文
pub fn bilibili_request(text: &str, target: Target) -> Request {
    let name = match target {
        Target::English => "英语",
        Target::Chinese => "中文",
    };
    Request {
        host: "index-translate.bilibili.com",
        path: "/v1/chat/completions".into(),
        headers: vec![("Content-Type".into(), "application/json".into())],
        body: serde_json::json!({
            "model": "Index-Translate-2B",
            "temperature": 0.0,
            "max_tokens": (text.chars().count() * 4).clamp(256, 4096),
            "stream": false,
            "chat_template_kwargs": {"enable_thinking": false},
            "messages": [{
                "role": "user",
                "content": format!("请将以下文本翻译为{name}，直接输出翻译结果，不要进行任何解释。\n\n{}", text.trim()),
            }],
        })
        .to_string(),
    }
}

pub fn parse_bilibili(body: &[u8]) -> Result<String, String> {
    let text = parse_google(body).map_err(|_| "B站没给结果".to_string())?;
    // 万一带了思考过程，只要后面的译文
    let text = match text.split_once("</think>") {
        Some((_, after)) => after.trim().to_owned(),
        None => text,
    };
    usable_text(&text)
        .map(str::to_owned)
        .ok_or_else(|| "B站没给结果".to_string())
}

/// 走 HTTPS 的服务这次要发的请求（微信输入法不走这里；没配 Key 的返回 None）
pub fn http_request(service: Service, keys: &Keys, text: &str, target: Target) -> Option<Request> {
    match service {
        Service::Volcengine => Some(volcengine_request(text, target)),
        Service::Microsoft => Some(microsoft_request(text, target)),
        Service::Bilibili => Some(bilibili_request(text, target)),
        Service::Azure => azure_request(keys, text, target),
        Service::Google => google_request(keys, text, target),
        Service::Zhipu => zhipu_request(keys, text, target),
        Service::Wetype => None,
    }
}

/// 解析 HTTPS 服务的回答（状态码已经是 200）
pub fn parse_reply(service: Service, body: &[u8]) -> Result<String, String> {
    match service {
        Service::Volcengine => parse_volcengine(body),
        Service::Microsoft | Service::Azure => parse_azure(body),
        Service::Bilibili => parse_bilibili(body),
        Service::Google | Service::Zhipu => parse_google(body),
        Service::Wetype => Err("微信输入法不走 HTTPS".into()),
    }
}

/// HTTP 状态码分成三种：限流、Key 不对、别的
pub fn http_failure(service: Service, status: u32) -> crate::translate_pool::Failure {
    use crate::translate_pool::Failure;
    let reason = status_reason(service.name(), status);
    match status {
        429 => Failure::Limited(reason),
        401 | 403 => Failure::Denied(reason),
        _ => Failure::Other(reason),
    }
}

/// 翻一篇的结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub text: String,
    /// 一共几段、几段没翻成（没翻成的段原样留着）
    pub total: usize,
    pub failed: usize,
    /// 没翻成的原因（各家的）
    pub reasons: Vec<String>,
    /// 一行日志：各家翻了几段、补位几次（不含内容）
    pub summary: String,
}

/// 切段、只把另一种语言的部分交给服务池去翻、按原来的样子拼回去
#[cfg(any(windows, test))]
#[allow(clippy::too_many_arguments)]
fn translate_planned(
    text: &str,
    target: Target,
    services: &[Service],
    translator: std::sync::Arc<dyn crate::translate_pool::Translator>,
    stats: &std::sync::Mutex<Option<crate::translate_pool::Stats>>,
    progress: Progress,
    timing: &crate::translate_pool::Timing,
) -> Result<Outcome, String> {
    let text = validated_text(text)?;
    let parts = plan(text, target);
    // 每个要翻的部分再按长度切段：（第几部分, 这一段）
    let jobs: Vec<(usize, Batch)> = parts
        .iter()
        .enumerate()
        .filter(|(_, part)| part.translate)
        .flat_map(|(n, part)| {
            batches(&text[part.range.clone()])
                .into_iter()
                .map(move |batch| (n, batch))
        })
        .collect();
    let wanted: Vec<usize> = (0..jobs.len())
        .filter(|&j| jobs[j].1.needs_translation())
        .collect();
    let report = crate::translate_pool::run(
        wanted.iter().map(|&j| jobs[j].1.body.to_owned()).collect(),
        target,
        services,
        translator,
        stats,
        &|source, reply| usable_reply(source, target, reply),
        progress,
        timing,
    );
    let mut replies: Vec<Option<String>> = vec![None; jobs.len()];
    let mut used: std::collections::BTreeMap<Service, usize> = Default::default();
    let mut reasons = Vec::new();
    for (&j, result) in wanted.iter().zip(report.results) {
        match result {
            Ok((reply, service)) => {
                *used.entry(service).or_default() += 1;
                replies[j] = Some(reply);
            }
            Err(reason) => reasons.push(reason),
        }
    }
    let mut out = String::new();
    let mut next_job = 0;
    for (n, part) in parts.iter().enumerate() {
        let original = &text[part.range.clone()];
        if !part.translate {
            push_part(&mut out, original, target);
            continue;
        }
        let mut translated = String::new();
        while next_job < jobs.len() && jobs[next_job].0 == n {
            let batch = &jobs[next_job].1;
            let reply = replies[next_job]
                .take()
                .unwrap_or_else(|| batch.body.to_owned());
            batch.join_into(&mut translated, &reply, target);
            next_job += 1;
        }
        push_part(&mut out, &translated, target);
    }
    let summary = format!(
        "{} 字 {} 段：{}{}{}",
        text.chars().count(),
        wanted.len(),
        used.iter()
            .map(|(service, n)| format!("{}{n}段", service.name()))
            .collect::<Vec<_>>()
            .join("、"),
        if report.hedges > 0 {
            format!("，补位 {} 次", report.hedges)
        } else {
            String::new()
        },
        if reasons.is_empty() {
            String::new()
        } else {
            format!("，{} 段没翻成", reasons.len())
        }
    );
    Ok(Outcome {
        text: out,
        total: wanted.len(),
        failed: reasons.len(),
        reasons,
        summary,
    })
}

/// 翻译进度：已经翻好几段、一共几段（翻译小窗显示「已翻 3/15 段」）
pub type Progress<'a> = &'a dyn Fn(usize, usize);

/// 同一条微信输入法连接上最多同时在途几段（实测 3000 词 15 批：一批一批等要 27 秒，16 批同时发 3 秒）
pub const PARALLEL_BATCHES: usize = 16;

/// 往 %LOCALAPPDATA%\XueerEar\translate.log 记一行（只记字数、段数、哪家、多久，不记内容）
pub fn log_line(what: &str) {
    use std::io::Write;
    let Some(dir) = std::env::var_os("LOCALAPPDATA") else {
        return;
    };
    let path = std::path::PathBuf::from(dir)
        .join("XueerEar")
        .join("translate.log");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let _ = writeln!(file, "{at} {what}");
    }
}

/// 翻译（方向按内容自动定）
#[cfg(windows)]
pub fn translate(text: &str) -> Result<String, String> {
    translate_into(text, Target::for_text(text.trim()))
}

/// 语音「翻译成英文 / 中文」：整框换成译文，所以每一段都要翻成才算成
#[cfg(windows)]
pub fn translate_into(text: &str, target: Target) -> Result<String, String> {
    let outcome = live::translate(text, target, None, &|_, _| {}, "语音")?;
    if outcome.failed > 0 {
        return Err(format!(
            "{} 段没翻成（{}）",
            outcome.failed,
            outcome.reasons.first().map_or("", String::as_str)
        ));
    }
    Ok(outcome.text)
}

/// 翻译小窗：边翻边报进度。从网页、PDF 复制来的英文常在句子中间硬换行，一行一行翻出来的中文一截一截的，
/// 先并回段落；语音翻译不走这里。有的段没翻成就原样留着（`Outcome::failed` 说几段），全都没翻成才算失败。
/// `only`：命令行排查时只用这一家
#[cfg(windows)]
pub fn translate_into_with_progress(
    text: &str,
    target: Target,
    only: Option<Service>,
    progress: Progress,
) -> Result<Outcome, String> {
    let outcome = live::translate(&unwrap_english(text), target, only, progress, "小窗")?;
    if outcome.total > 0 && outcome.failed == outcome.total {
        return Err(outcome
            .reasons
            .first()
            .cloned()
            .unwrap_or_else(|| "没翻成".into()));
    }
    Ok(outcome)
}

/// 真的去翻：HTTPS 的几家走 WinHTTP，微信输入法这一篇开一条连接
#[cfg(windows)]
mod live {
    use super::*;
    use crate::translate_pool::{Failure, STATS, Timing, Translator};
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex, mpsc},
        time::{Duration, Instant},
    };

    pub(super) fn translate(
        text: &str,
        target: Target,
        only: Option<Service>,
        progress: Progress,
        from: &str,
    ) -> Result<Outcome, String> {
        let keys = local_keys()?;
        let services = match only {
            Some(service) => vec![service],
            None => services(&keys),
        };
        let started = Instant::now();
        let translator: Arc<dyn Translator> = Arc::new(Live {
            keys,
            wetype: Mutex::new(None),
        });
        let outcome = translate_planned(
            text,
            target,
            &services,
            translator,
            &STATS,
            progress,
            &Timing::REAL,
        );
        match &outcome {
            Ok(outcome) => log_line(&format!(
                "{from}翻译 {}，{} ms",
                outcome.summary,
                started.elapsed().as_millis()
            )),
            Err(reason) => log_line(&format!("{from}翻译失败：{reason}")),
        }
        outcome
    }

    struct Live {
        keys: Keys,
        /// 微信输入法：第一次用到时开一条连接，这一篇的段都从这条连接走
        wetype: Mutex<Option<mpsc::Sender<Ask>>>,
    }

    impl Translator for Live {
        fn translate(&self, service: Service, text: &str, target: Target) -> Result<String, Failure> {
            if service == Service::Wetype {
                return self.ask_wetype(text);
            }
            let request = http_request(service, &self.keys, text, target)
                .ok_or_else(|| Failure::Other("没配 Key".into()))?;
            match send(&request) {
                Ok((200, body)) => parse_reply(service, &body).map_err(Failure::Other),
                Ok((status, _)) => Err(http_failure(service, status)),
                Err(error) => Err(Failure::Other(format!("连不上（{error}）"))),
            }
        }
    }

    /// 交给微信输入法连接线程的一段：原文和回话的地方
    type Ask = (String, mpsc::Sender<Result<String, String>>);

    impl Live {
        fn ask_wetype(&self, text: &str) -> Result<String, Failure> {
            let (reply, answer) = mpsc::channel();
            {
                let mut worker = self.wetype.lock().unwrap_or_else(|e| e.into_inner());
                let sender = worker.get_or_insert_with(spawn_wetype);
                sender
                    .send((text.to_owned(), reply))
                    .map_err(|_| Failure::Other("微信输入法翻译连接中断".into()))?;
            }
            match answer.recv_timeout(Duration::from_secs(25)) {
                Ok(result) => result.map_err(Failure::Other),
                Err(_) => Err(Failure::Other("超时".into())),
            }
        }
    }

    /// 开一条微信输入法连接，在自己的线程里收段、发出去、把回来的译文交回去。
    /// 连接和认证最多等 8 秒；一段 20 秒没回算超时；这一篇翻完（没人再交段）就关掉
    fn spawn_wetype() -> mpsc::Sender<Ask> {
        let (sender, asks) = mpsc::channel::<Ask>();
        std::thread::spawn(move || {
            let connect_by = Instant::now() + Duration::from_secs(8);
            let client = crate::wetype::Client::from_default_device_cancellable(&|| {
                Instant::now() >= connect_by
            });
            let mut client = match client {
                Ok(client) => client,
                Err(_) => {
                    for (_, reply) in asks.iter() {
                        let _ = reply.send(Err("连不上微信输入法翻译".into()));
                    }
                    return;
                }
            };
            let mut flying: HashMap<u32, (mpsc::Sender<Result<String, String>>, Instant)> =
                HashMap::new();
            let mut open = true;
            while open || !flying.is_empty() {
                loop {
                    match asks.try_recv() {
                        Ok((text, reply)) => match client.send_translation(&text) {
                            Ok(id) => {
                                flying.insert(id, (reply, Instant::now()));
                            }
                            Err(error) => {
                                let _ = reply.send(Err(error.to_string()));
                            }
                        },
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            open = false;
                            break;
                        }
                    }
                    if flying.len() >= PARALLEL_BATCHES {
                        break;
                    }
                }
                flying.retain(|_, (reply, sent)| {
                    let late = sent.elapsed() >= Duration::from_secs(20);
                    if late {
                        let _ = reply.send(Err("微信输入法翻译超时".into()));
                    }
                    !late
                });
                match client.poll_translation() {
                    Ok(Some((id, result))) => {
                        if let Some((reply, _)) = flying.remove(&id) {
                            let _ = reply.send(result.map_err(|error| error.to_string()));
                        }
                    }
                    Ok(None) => {
                        if flying.is_empty() && open {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                    }
                    Err(_) => {
                        for (_, (reply, _)) in flying.drain() {
                            let _ = reply.send(Err("微信输入法翻译连接中断".into()));
                        }
                        for (_, reply) in asks.try_iter() {
                            let _ = reply.send(Err("微信输入法翻译连接中断".into()));
                        }
                        return;
                    }
                }
            }
            client.close();
        });
        sender
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translate_pool::{Failure, Stats, Timing, Translator};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// 假的服务：按 `answer(哪家, 原文)` 回答，记下问了哪家哪段
    struct Fake<F: Fn(Service, &str) -> Result<String, Failure> + Send + Sync> {
        answer: F,
        asked: Mutex<Vec<(Service, String)>>,
    }

    impl<F: Fn(Service, &str) -> Result<String, Failure> + Send + Sync> Translator for Fake<F> {
        fn translate(&self, service: Service, text: &str, _: Target) -> Result<String, Failure> {
            self.asked
                .lock()
                .unwrap()
                .push((service, text.to_owned()));
            (self.answer)(service, text)
        }
    }

    const QUICK: Timing = Timing {
        hedge_base: Duration::from_millis(300),
        hedge_per_300: Duration::from_millis(100),
        hedge_max: Duration::from_millis(800),
        attempt_limit: Duration::from_secs(2),
    };

    fn translate_with<F: Fn(Service, &str) -> Result<String, Failure> + Send + Sync + 'static>(
        text: &str,
        target: Target,
        answer: F,
    ) -> (Result<Outcome, String>, Vec<(Service, String)>) {
        let fake = Arc::new(Fake {
            answer,
            asked: Mutex::new(Vec::new()),
        });
        let stats = Mutex::new(Some(Stats::default()));
        let out = translate_planned(
            text,
            target,
            &[Service::Volcengine, Service::Microsoft, Service::Bilibili],
            fake.clone(),
            &stats,
            &|_, _| {},
            &QUICK,
        );
        let asked = fake.asked.lock().unwrap().clone();
        (out, asked)
    }

    #[test]
    fn only_the_other_language_is_sent_and_the_rest_stays() {
        let (out, asked) = translate_with(
            "We shipped it. 然后我们再测一遍，没问题就发。",
            Target::English,
            |_, _| Ok("Then we test it again and ship if it's fine.".into()),
        );
        assert_eq!(
            out.unwrap().text,
            "We shipped it. Then we test it again and ship if it's fine."
        );
        assert_eq!(
            asked,
            [(Service::Volcengine, "然后我们再测一遍，没问题就发。".to_owned())]
        );
    }

    #[test]
    fn nothing_to_translate_returns_the_text_unchanged() {
        let (out, asked) = translate_with("Already English.", Target::English, |_, _| {
            panic!("nothing to translate")
        });
        assert_eq!(out.unwrap().text, "Already English.");
        assert!(asked.is_empty());
    }

    // 一家回来的译文不能用（空的、方向不对、多出换行、少了一截），换下一家
    #[test]
    fn an_unusable_reply_goes_to_the_next_service() {
        let source = "你好，今天的测试全部通过了，可以发布新版本。\n明天再看一下安装包有没有被杀毒软件误报。";
        for unusable in [
            "  ",
            "你好，世界。",
            "Hello,\nall tests\npassed.\nTomorrow",
            "Hello.",
        ] {
            let (out, asked) = translate_with(source, Target::English, move |service, _| {
                Ok(match service {
                    Service::Volcengine => unusable.to_owned(),
                    _ => "Hello, all tests passed today.\nTomorrow check the installer.".into(),
                })
            });
            assert_eq!(
                out.unwrap().text,
                "Hello, all tests passed today.\nTomorrow check the installer.",
                "{unusable:?}"
            );
            assert_eq!(asked.len(), 2);
        }
    }

    // 主人 2026-11-24：1 万字也要能翻；切成几十段分给各家，不是整篇 5 秒算失败
    #[test]
    fn a_ten_thousand_character_text_is_split_into_pieces_and_joined_back() {
        let text = "今天吃什么。明天去哪里？".repeat(830);
        // 假译文：英文字母的量和原文差不多（一个汉字约三个字母），能过「少了一截」的检查
        let (out, asked) = translate_with(&text, Target::English, |_, piece| {
            Ok(format!("[{}]", "x".repeat(piece.chars().count() * 3)))
        });
        let outcome = out.unwrap();
        assert_eq!(outcome.failed, 0);
        assert!(asked.len() >= 15, "{}", asked.len());
        assert!(
            asked
                .iter()
                .all(|(_, piece)| piece.chars().count() <= CHUNK_CHARS)
        );
        // 同一行切开的几段，英文译文之间隔一个空格
        assert!(
            outcome.text.contains("] ["),
            "{}",
            outcome.text.chars().take(80).collect::<String>()
        );
    }

    #[test]
    fn failed_pieces_keep_the_original_and_are_counted() {
        let (out, _) = translate_with("第一段。\n\n第二段。", Target::English, |_, piece| {
            if piece.contains("第二段") {
                Err(Failure::Other("出错".into()))
            } else {
                Ok("Paragraph.".into())
            }
        });
        let outcome = out.unwrap();
        // 短段合成一段送：一起失败
        assert_eq!(outcome.total, 1);
        assert_eq!(outcome.failed, 1);
        assert!(outcome.summary.contains("1 段没翻成"), "{}", outcome.summary);
        assert_eq!(outcome.text, "第一段。\n\n第二段。");
    }

    #[test]
    fn invalid_input_never_reaches_a_service() {
        for source in ["  ".to_owned(), "中".repeat(MAX_CHARS + 1)] {
            let (out, asked) = translate_with(&source, Target::English, |_, _| {
                panic!("invalid input should not be translated")
            });
            assert!(out.is_err());
            assert!(asked.is_empty());
        }
    }

    #[test]
    fn the_free_services_come_first_and_keyed_ones_only_when_configured() {
        assert_eq!(
            services(&Keys::default()),
            [
                Service::Volcengine,
                Service::Microsoft,
                Service::Bilibili,
                Service::Wetype
            ]
        );
        let keys = Keys {
            wetype_enabled: false,
            google_key: "g".into(),
            ..Keys::default()
        };
        assert_eq!(
            services(&keys),
            [
                Service::Volcengine,
                Service::Microsoft,
                Service::Bilibili,
                Service::Google
            ]
        );
    }

    // 2026-11-24 在华为上真请求过的三家：请求长什么样、回答怎么读
    #[test]
    fn the_free_services_are_asked_and_read_the_way_they_answered() {
        let volc = volcengine_request("今天天气不错", Target::English);
        assert_eq!(volc.host, "translate.volcengine.com");
        assert_eq!(volc.path, "/crx/translate/v1/");
        let body: serde_json::Value = serde_json::from_str(&volc.body).unwrap();
        assert_eq!(body["target_language"], "en");
        assert_eq!(body["source_language"], "");
        assert_eq!(
            parse_volcengine(br#"{"translation":"The weather is nice today","detected_language":"zh","probability":1,"base_resp":{"status_code":0,"status_message":""}}"#),
            Ok("The weather is nice today".into())
        );
        assert!(parse_volcengine(br#"{"base_resp":{"status_code":1001}}"#).is_err());
        let ms = microsoft_request("Hello", Target::Chinese);
        assert_eq!(ms.host, "edge.microsoft.com");
        assert!(ms.path.contains("to=zh-Hans") && ms.path.contains("from=&"));
        assert_eq!(ms.body, r#"["Hello"]"#);
        assert_eq!(
            parse_reply(
                Service::Microsoft,
                r#"[{"detectedLanguage":{"language":"en","score":1.0},"translations":[{"text":"你好","to":"zh-Hans"}]}]"#
                    .as_bytes()
            ),
            Ok("你好".into())
        );
        let bili = bilibili_request("Hello", Target::Chinese);
        let body: serde_json::Value = serde_json::from_str(&bili.body).unwrap();
        assert_eq!(body["model"], "Index-Translate-2B");
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
        assert!(body["messages"][0]["content"].as_str().unwrap().contains("翻译为中文"));
        assert_eq!(
            parse_bilibili(
                r#"{"choices":[{"message":{"content":"<think>\n</think>\n\n你好"}}]}"#.as_bytes()
            ),
            Ok("你好".into())
        );
        assert!(matches!(
            http_failure(Service::Volcengine, 429),
            Failure::Limited(_)
        ));
        assert!(matches!(
            http_failure(Service::Azure, 401),
            Failure::Denied(_)
        ));
    }

    #[test]
    fn chinese_and_english_in_one_line_are_split_by_sentence() {
        let parts = |text: &'static str, target| -> Vec<(&'static str, bool)> {
            plan(text, target)
                .into_iter()
                .map(|p| (&text[p.range], p.translate))
                .collect()
        };
        assert_eq!(
            parts(
                "We shipped it. 然后我们再测一遍，没问题就发。",
                Target::English
            ),
            [
                ("We shipped it. ", false),
                ("然后我们再测一遍，没问题就发。", true)
            ]
        );
        // 中文句子里夹着的英文词跟着整句一起翻，不单独留下
        assert_eq!(
            parts("用 Claude Code 跑一下测试。", Target::English),
            [("用 Claude Code 跑一下测试。", true)]
        );
        // 英文句子里的数字、标点不会把句子切开
        assert_eq!(
            parts("Version 3.5, released on 1,000 machines.", Target::Chinese),
            [("Version 3.5, released on 1,000 machines.", true)]
        );
        // 两种语言一句一句交替时，各翻各的，留下的原样
        assert_eq!(
            parts("好的。Sounds good. 那就这样。", Target::Chinese),
            [
                ("好的。", false),
                ("Sounds good.", true),
                (" 那就这样。", false)
            ]
        );
    }

    #[test]
    fn hard_wrapped_english_is_joined_back_into_paragraphs() {
        assert_eq!(
            unwrap_english(
                "This license was wrapped by hand in the middle,\r\nof a sentence that goes on for a while and\r\nends here.\r\n\r\nNext paragraph."
            ),
            "This license was wrapped by hand in the middle, of a sentence that goes on for a while and ends here.\r\n\r\nNext paragraph."
        );
        // 短行的清单、列表、标题、句末换行、中文都不并
        for kept in [
            "apples\nbananas\noranges",
            "Steps:\n- run the tests\n- ship it",
            "The release notes are ready and the tests pass.\nanother sentence starts here",
            "TERMS AND CONDITIONS\n\n0. Definitions.",
            "第一行\n第二行",
        ] {
            assert_eq!(unwrap_english(kept), kept);
        }
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
    }

    #[test]
    fn short_paragraphs_are_sent_together() {
        let text = "第一段。\n\n- 第二条\n- 第三条\n\n**第四段**\n".repeat(10);
        let batches = merge(&text, chunks(&text, CHUNK_CHARS), CHUNK_CHARS);
        assert_eq!(batches.len(), 1, "{batches:?}");
        let joined: String = batches.iter().map(|(p, s)| format!("{p}{s}")).collect();
        assert_eq!(joined, text);
        let long = "很长的一句话没有标点".repeat(400);
        let batches = merge(&long, chunks(&long, CHUNK_CHARS), CHUNK_CHARS);
        assert!(
            batches.len() >= 6
                && batches
                    .iter()
                    .all(|(p, _)| p.chars().count() <= CHUNK_CHARS)
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
        let legacy_keys = Keys::load(&dir.path().join("t.json")).unwrap();
        assert_eq!(legacy_keys.google_key, "g");
        assert!(legacy_keys.wetype_enabled);
        std::fs::write(dir.path().join("bad.json"), "{").unwrap();
        assert!(Keys::load(&dir.path().join("bad.json")).is_err());
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
