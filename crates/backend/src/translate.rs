//! 雪耳翻译：微信输入法自动翻译，失败时用本机配置的服务。
//! 配置在 `%LOCALAPPDATA%\XueerEar\translate.json`。
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

/// 按顺序试：微软 → 谷歌 → 智谱。`send` 负责真正发请求，返回（状态码, 响应体）。
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
    let text = validated_text(text)?;
    if keys.is_empty() {
        return Err("还没配翻译 Key".into());
    }
    translate_batches(text, target, |piece| {
        translate_piece(keys, piece, target, &mut send)
    })
}

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

/// 一批要翻的原文：前面原样保留的换行、正文、后面的分隔符
struct Batch<'a> {
    lead: &'a str,
    body: &'a str,
    separator: &'a str,
}

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

fn translate_batches(
    text: &str,
    target: Target,
    mut translate: impl FnMut(&str) -> Result<String, String>,
) -> Result<String, String> {
    let mut out = String::new();
    for batch in batches(text) {
        let translated = if batch.needs_translation() {
            translate(batch.body)?
        } else {
            batch.body.to_owned()
        };
        batch.join_into(&mut out, &translated, target);
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
        let mut all_limited = true;
        for (service, request, parse) in &attempts {
            let Some(request) = request else { continue };
            let reply = send(request);
            all_limited &= matches!(&reply, Ok((429, _)));
            reason = match reply {
                Ok((200, body)) => match parse(&body) {
                    Ok(text) => return Ok(text),
                    Err(error) => error,
                },
                Ok((status, _)) => status_reason(service, status),
                Err(error) => format!("{service} 连不上（{error}）"),
            };
        }
        // 不是限流（Key 不对、连不上）的错，等也没用
        if !all_limited {
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
            ..Keys::default()
        }
    }

    /// 假的微信输入法：每批按 `answer` 回答，记下收到了哪几批
    fn fake_wetype<'a>(
        asked: &'a std::cell::RefCell<Vec<String>>,
        answer: impl Fn(&str) -> Result<String, String> + 'a,
    ) -> impl FnOnce(&[&str], &mut dyn FnMut(usize, Result<String, String>)) + 'a {
        move |pieces, done| {
            for (i, piece) in pieces.iter().enumerate() {
                asked.borrow_mut().push(piece.to_string());
                done(i, answer(piece));
            }
        }
    }

    fn no_progress(_: usize, _: usize) {}

    #[test]
    fn invalid_input_never_reaches_a_service_or_settings() {
        for source in ["  ".to_owned(), "中".repeat(MAX_CHARS + 1)] {
            assert!(
                translate_routed(
                    &source,
                    Target::English,
                    || panic!("invalid input should not load settings"),
                    |_| panic!("invalid input should not reach a keyed provider"),
                    |_, _| panic!("invalid input should not reach WeType"),
                    &no_progress,
                )
                .is_err()
            );
        }
    }

    #[test]
    fn long_text_uses_bounded_requests_and_can_fall_back_between_batches() {
        let source = format!("{}。\n第二段。", "中".repeat(CHUNK_CHARS - 1));
        let asked = std::cell::RefCell::new(Vec::new());
        let out = translate_routed(
            &source,
            Target::English,
            || Ok(both()),
            |request| {
                let body: serde_json::Value = serde_json::from_str(&request.body).unwrap();
                assert_eq!(body[0]["Text"], "第二段。");
                Ok((
                    200,
                    br#"[{"translations":[{"text":"Second paragraph."}]}]"#.to_vec(),
                ))
            },
            fake_wetype(&asked, |piece| {
                assert!(
                    piece.chars().count() <= CHUNK_CHARS,
                    "oversized WeType request"
                );
                if piece == "第二段。" {
                    Err("微信输入法翻译超时".into())
                } else {
                    Ok(
                        "First paragraph is long and goes on for quite a while in English. "
                            .repeat(60),
                    )
                }
            }),
            &no_progress,
        )
        .unwrap();
        assert_eq!(asked.borrow().len(), 2);
        assert!(out.ends_with(".\nSecond paragraph."), "{out}");
    }

    // 2026-11-20 主人粘了约 3000 词英文：微信把硬换行并回段落，换行数对不上，
    // 每批都被丢给智谱、撞上限流等了一分多钟，最后什么都没出。换行变少是正常的
    #[test]
    fn wetype_may_join_hard_wrapped_lines() {
        let source = "This is a line that was\r\nwrapped by hand in the\r\nmiddle of a sentence.";
        let asked = std::cell::RefCell::new(Vec::new());
        let out = translate_routed(
            source,
            Target::Chinese,
            || Ok(both()),
            |_| panic!("WeType answered; keyed providers must not be used"),
            fake_wetype(&asked, |_| Ok("这是一句在句子中间被手动换行的话。".into())),
            &no_progress,
        );
        assert_eq!(out.as_deref(), Ok("这是一句在句子中间被手动换行的话。"));
    }

    #[test]
    fn an_empty_wrong_direction_extra_line_or_cut_short_reply_falls_back() {
        let source = "你好，今天的测试全部通过了，可以发布新版本。\n明天再看一下安装包有没有被杀毒软件误报。";
        for unusable in [
            "  ",
            "你好，世界。",
            "Hello\0\nworld.",
            "Hello,\nall tests\npassed.\nTomorrow",
            "Hello.",
        ] {
            let asked = std::cell::RefCell::new(Vec::new());
            let out = translate_routed(
                source,
                Target::English,
                || Ok(both()),
                |_| {
                    Ok((
                        200,
                        br#"[{"translations":[{"text":"Hello, all tests passed today.\nTomorrow check the installer."}]}]"#.to_vec(),
                    ))
                },
                fake_wetype(&asked, |_| Ok(unusable.into())),
                &no_progress,
            );
            assert_eq!(
                out.as_deref(),
                Ok("Hello, all tests passed today.\nTomorrow check the installer."),
                "{unusable:?}"
            );
        }
    }

    #[test]
    fn every_batch_goes_to_wetype_at_once_and_progress_is_reported() {
        let source = "第一段很长。".repeat(400) + "\n" + &"第二段也很长。".repeat(300);
        let asked = std::cell::RefCell::new(Vec::new());
        let seen = std::cell::RefCell::new(Vec::new());
        let out = translate_routed(
            &source,
            Target::English,
            || Ok(Keys::default()),
            |_| panic!("no keyed providers are configured"),
            move |pieces: &[&str], done: &mut dyn FnMut(usize, Result<String, String>)| {
                asked
                    .borrow_mut()
                    .extend(pieces.iter().map(|p| p.to_string()));
                // 谁先回来先记谁：倒着回
                for i in (0..pieces.len()).rev() {
                    done(
                        i,
                        Ok(format!(
                            "Part {i} {}",
                            "word ".repeat(pieces[i].chars().count() / 2)
                        )),
                    );
                }
                assert!(pieces.len() >= 3, "{}", pieces.len());
            },
            &|done, total| seen.borrow_mut().push((done, total)),
        )
        .unwrap();
        let seen = seen.into_inner();
        assert_eq!(seen.last().map(|p| p.0), seen.last().map(|p| p.1));
        assert!(seen.windows(2).all(|w| w[0].0 < w[1].0));
        // 译文按原文的顺序拼回去
        assert!(out.starts_with("Part 0 "), "{}", &out[..40]);
        assert!(out.find("Part 1 ").unwrap() < out.find("Part 2 ").unwrap());
    }

    // 主人 2026-11-21：翻译成英文就全部变成英文，已经是英文的部分不动；翻译成中文同理
    #[test]
    fn only_the_other_language_is_sent_and_the_rest_stays() {
        let asked = std::cell::RefCell::new(Vec::new());
        let out = translate_routed(
            "The tests all pass.\n今天先这样，明天再发布。",
            Target::English,
            || Ok(Keys::default()),
            |_| panic!("no keyed providers are configured"),
            fake_wetype(&asked, |piece| {
                assert_eq!(piece, "今天先这样，明天再发布。");
                Ok("That's it for today; release tomorrow.".into())
            }),
            &no_progress,
        );
        assert_eq!(
            out.as_deref(),
            Ok("The tests all pass.\nThat's it for today; release tomorrow.")
        );
        let asked = std::cell::RefCell::new(Vec::new());
        let out = translate_routed(
            "The tests all pass.\n今天先这样，明天再发布。",
            Target::Chinese,
            || Ok(Keys::default()),
            |_| panic!("no keyed providers are configured"),
            fake_wetype(&asked, |piece| {
                assert_eq!(piece, "The tests all pass.");
                Ok("测试全部通过了。".into())
            }),
            &no_progress,
        );
        assert_eq!(
            out.as_deref(),
            Ok("测试全部通过了。\n今天先这样，明天再发布。")
        );
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
        let asked = std::cell::RefCell::new(Vec::new());
        let out = translate_routed(
            "We shipped it. 然后我们再测一遍，没问题就发。",
            Target::English,
            || Ok(Keys::default()),
            |_| panic!("no keyed providers are configured"),
            fake_wetype(&asked, |_| {
                Ok("Then we test it again and ship if it's fine.".into())
            }),
            &no_progress,
        );
        assert_eq!(
            out.as_deref(),
            Ok("We shipped it. Then we test it again and ship if it's fine.")
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
    fn nothing_to_translate_returns_the_text_unchanged() {
        let out = translate_routed(
            "Already English.",
            Target::English,
            || Ok(Keys::default()),
            |_| panic!("nothing to translate"),
            |_, _| panic!("nothing to translate"),
            &no_progress,
        );
        assert_eq!(out.as_deref(), Ok("Already English."));
    }

    #[test]
    fn a_wetype_failure_without_keys_reports_the_actual_failure() {
        let asked = std::cell::RefCell::new(Vec::new());
        let err = translate_routed(
            "你好",
            Target::English,
            || Ok(Keys::default()),
            |_| panic!("no keyed providers are configured"),
            fake_wetype(&asked, |_| Err("微信输入法翻译超时".into())),
            &no_progress,
        )
        .unwrap_err();
        assert!(err.contains("微信输入法翻译超时"), "{err}");
    }

    #[test]
    fn empty_config_supports_both_directions_without_keys() {
        for (source, target, expected) in [
            ("你好", Target::English, "Hello"),
            ("Hello", Target::Chinese, "你好"),
        ] {
            let asked = std::cell::RefCell::new(Vec::new());
            let out = translate_routed(
                source,
                target,
                || Ok(Keys::default()),
                |_| panic!("no keyed providers are configured"),
                fake_wetype(&asked, |piece| {
                    assert_eq!(piece, source);
                    Ok(expected.into())
                }),
                &no_progress,
            );
            assert_eq!(out.as_deref(), Ok(expected));
        }
    }

    #[test]
    fn explicit_disable_keeps_the_existing_keyed_flow() {
        let keys: Keys =
            serde_json::from_str(r#"{"wetype_enabled":false,"azure_key":"a"}"#).unwrap();
        let out = translate_routed(
            "你好",
            Target::English,
            || Ok(keys),
            |request| {
                assert!(request.path.ends_with("to=en"));
                Ok((200, br#"[{"translations":[{"text":"Hello"}]}]"#.to_vec()))
            },
            |_, _| panic!("WeType is disabled"),
            &no_progress,
        );
        assert_eq!(out.as_deref(), Ok("Hello"));
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
    fn a_rate_limit_and_an_invalid_key_do_not_retry() {
        let mut calls = 0;
        let err = translate_with(&both(), "你好", |request| {
            calls += 1;
            let status = if request.host == "api.cognitive.microsofttranslator.com" {
                429
            } else {
                401
            };
            Ok((status, Vec::new()))
        });
        assert_eq!(calls, 2);
        assert_eq!(err.unwrap_err(), "谷歌 的 Key 不对或没开通");
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
        let legacy_keys = Keys::load(&dir.path().join("t.json")).unwrap();
        assert_eq!(legacy_keys.google_key, "g");
        assert!(legacy_keys.wetype_enabled);
        std::fs::write(dir.path().join("bad.json"), "{").unwrap();
        assert!(Keys::load(&dir.path().join("bad.json")).is_err());
    }
}

/// 用本机 WinHTTP 发一次 HTTPS POST（走系统代理），返回（状态码, 响应体）
#[cfg(windows)]
pub fn send(request: &Request) -> Result<(u32, Vec<u8>), String> {
    native::post(request)
}

/// 翻译：微信输入法云端主力（免 Key），失败落到有 Key 的微软 → 谷歌 → 智谱
#[cfg(windows)]
pub fn translate(text: &str) -> Result<String, String> {
    translate_entry(text, Target::for_text(text.trim()))
}

/// 译成指定语言（翻译小窗手动选了方向时用）
#[cfg(windows)]
pub fn translate_into(text: &str, target: Target) -> Result<String, String> {
    translate_entry(text, target)
}

/// 同一条连接上最多同时在途几批（实测 3000 词 15 批：一批一批等要 27 秒，16 批同时发 3 秒）
pub const PARALLEL_BATCHES: usize = 16;

/// 翻译进度：已经翻好几批、一共几批（翻译小窗显示「已翻 3/15 段」）
pub type Progress<'a> = &'a dyn Fn(usize, usize);

#[cfg(windows)]
fn translate_entry(text: &str, target: Target) -> Result<String, String> {
    translate_entry_with_progress(text, target, &|_, _| {})
}

/// 译成指定语言，边翻边报进度（翻译小窗用）。从网页、PDF 复制来的英文常在句子中间硬换行，
/// 一行一行翻出来的中文一截一截的，先并回段落；语音翻译不走这里（那里的换行是主人特意说的）
#[cfg(windows)]
pub fn translate_into_with_progress(
    text: &str,
    target: Target,
    progress: Progress,
) -> Result<String, String> {
    translate_entry_with_progress(&unwrap_english(text), target, progress)
}

#[cfg(windows)]
fn translate_entry_with_progress(
    text: &str,
    target: Target,
    progress: Progress,
) -> Result<String, String> {
    translate_routed(
        text,
        target,
        local_keys,
        send,
        |pieces, done| wetype_batches(pieces, done),
        progress,
    )
}

/// 微信输入法：所有批同时排队发（最多 `PARALLEL_BATCHES` 批在途），谁先回来先记谁。
/// `done(i, 结果)` 每批回来调一次；连不上、整体超时时没回来的批记失败。
#[cfg(windows)]
fn wetype_batches(pieces: &[&str], done: &mut dyn FnMut(usize, Result<String, String>)) {
    use std::time::{Duration, Instant};
    // 连接和认证 8 秒；每批从发出起 20 秒没回就算超时（实测单批 1～2.5 秒）
    let connect_by = Instant::now() + Duration::from_secs(8);
    let mut client = match crate::wetype::Client::from_default_device_cancellable(&|| {
        Instant::now() >= connect_by
    }) {
        Ok(client) => client,
        Err(_) => {
            for i in 0..pieces.len() {
                done(i, Err("连不上微信输入法翻译".into()));
            }
            return;
        }
    };
    let mut next = 0;
    let mut flying: Vec<(u32, usize, Instant)> = Vec::new();
    while next < pieces.len() || !flying.is_empty() {
        while next < pieces.len() && flying.len() < PARALLEL_BATCHES {
            match client.send_translation(pieces[next]) {
                Ok(id) => flying.push((id, next, Instant::now())),
                Err(err) => done(next, Err(err.to_string())),
            }
            next += 1;
        }
        flying.retain(|&(_, i, sent)| {
            let late = sent.elapsed() >= Duration::from_secs(20);
            if late {
                done(i, Err("微信输入法翻译超时".into()));
            }
            !late
        });
        match client.poll_translation() {
            Ok(Some((id, result))) => {
                if let Some(at) = flying.iter().position(|f| f.0 == id) {
                    let (_, i, _) = flying.remove(at);
                    done(i, result.map_err(|err| err.to_string()));
                }
            }
            Ok(None) => {}
            Err(_) => {
                for (_, i, _) in flying.drain(..) {
                    done(i, Err("微信输入法翻译连接中断".into()));
                }
                for i in next..pieces.len() {
                    done(i, Err("微信输入法翻译连接中断".into()));
                }
                return;
            }
        }
    }
    client.close();
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

/// 先把所有要翻的批交给微信输入法（同时发），没翻成的批再一批一批交给有 Key 的服务。
/// `wetype(批, 回调)`：每批回来调一次回调（测试里换成假的）。
#[cfg(any(windows, test))]
fn translate_routed(
    text: &str,
    target: Target,
    load_keys: impl FnOnce() -> Result<Keys, String>,
    mut send: impl FnMut(&Request) -> Result<(u32, Vec<u8>), String>,
    wetype: impl FnOnce(&[&str], &mut dyn FnMut(usize, Result<String, String>)),
    progress: Progress,
) -> Result<String, String> {
    let text = validated_text(text)?;
    let keys = load_keys()?;
    let parts = plan(text, target);
    // 每个要翻的段再按长度切批：（第几段, 这一批）
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
    let total = wanted.len();
    let mut results: Vec<Option<Result<String, String>>> = vec![None; jobs.len()];
    let mut finished = 0;
    if keys.wetype_enabled && !wanted.is_empty() {
        let bodies: Vec<&str> = wanted.iter().map(|&j| jobs[j].1.body).collect();
        wetype(&bodies, &mut |n, reply| {
            let j = wanted[n];
            let reply = reply.and_then(|reply| usable_wetype_reply(jobs[j].1.body, target, reply));
            if reply.is_ok() {
                finished += 1;
                progress(finished, total);
            }
            results[j] = Some(reply);
        });
    }
    for &j in &wanted {
        if matches!(results[j], Some(Ok(_))) {
            continue;
        }
        let reason = match results[j].take() {
            Some(Err(reason)) => Some(reason),
            _ if keys.wetype_enabled => Some("微信输入法翻译没有回应".to_owned()),
            _ => None,
        };
        if keys.is_empty() {
            return Err(match reason {
                Some(reason) => format!("{reason}；可在翻译设置里配置备用服务"),
                None => "还没配翻译 Key".into(),
            });
        }
        let reply =
            translate_piece(&keys, jobs[j].1.body, target, &mut send).map_err(
                |err| match reason {
                    Some(reason) => format!("{reason}；备用服务：{err}"),
                    None => err,
                },
            )?;
        finished += 1;
        progress(finished, total);
        results[j] = Some(Ok(reply));
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
            let reply = match results[next_job].take() {
                Some(Ok(reply)) => reply,
                _ => batch.body.to_owned(),
            };
            batch.join_into(&mut translated, &reply, target);
            next_job += 1;
        }
        push_part(&mut out, &translated, target);
    }
    Ok(out)
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
fn usable_wetype_reply(source: &str, target: Target, reply: String) -> Result<String, String> {
    let reply = usable_text(&reply).ok_or("微信输入法没返回有效译文")?;
    // 自动检测由服务端完成；明显答错方向时不能覆盖原文。
    let has_language = source.chars().any(|c| is_cjk(c) || c.is_ascii_alphabetic());
    let has_target = reply.chars().any(|c| match target {
        Target::Chinese => is_cjk(c),
        Target::English => c.is_ascii_alphabetic(),
    });
    if has_language && !has_target {
        return Err("微信输入法返回的译文方向不对".into());
    }
    // 硬换行的长文（从网页、PDF 复制的）微信会把断开的行并回段落，译文换行变少是正常的；
    // 变多不行：听写时多出来的换行在聊天框里会变成回车，把半截话发出去
    let lines = |t: &str| t.bytes().filter(|&c| c == b'\n').count();
    if lines(reply) > lines(source) {
        return Err("微信输入法的译文多出了换行".into());
    }
    if weight(source) >= 60 && weight(reply) * 5 < weight(source) * 2 {
        return Err("微信输入法的译文像是少了一截".into());
    }
    Ok(reply.to_owned())
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
