//! 豆包输入法的云端识别，纯协议直连：匿名注册一台虚拟手机拿设备号，WebSocket 把 20 ms 一帧的 Opus 发给识别服务。
//! 不装豆包、不登录、不调用豆包的程序文件。协议细节参考了公开资料（VoiceKey、doubao-asr-free、doubao-asr-web），
//! 代码是雪耳自己写的。帧格式在 `xueer_ear_protocol::doubao`。
pub mod device;
pub mod session;
pub mod transcript;
pub mod transport;

/// 官方客户端内置的识别 app key（公开客户端常量；按设备下发的 key 有些设备会被拒）
pub const APP_KEY: &str = "OrnqKvSSrs";

/// 一句话的会话参数：16 kHz 单声道 Opus、要标点、二遍识别（服务端按自己的规则决定用不用）。
/// `context` 是热词提示（见 [`context`]），没有就不带。
pub fn session_payload(device_id: &str, context: Option<&str>) -> String {
    let mut payload = serde_json::json!({
        "audio_info": {"channel": 1, "format": "speech_opus", "sample_rate": 16000},
        "enable_punctuation": true,
        "enable_speech_rejection": false,
        "extra": {
            "app_name": "oime",
            "cell_compress_rate": 8,
            "did": device_id,
            "enable_asr_threepass": false,
            "enable_asr_twopass": true,
            "input_mode": "stream",
        },
    });
    if let Some(context) = context {
        payload["extra"]["context"] = context.into();
    }
    payload.to_string()
}

/// 热词提示：官方客户端在会话参数的 extra.context 里带的 base64 JSON，这里只放热词，
/// 不放主人输入框里的字、程序名和位置。没有热词返回 None。
pub fn context(hotwords: &[String]) -> Option<String> {
    if hotwords.is_empty() {
        return None;
    }
    let words: Vec<serde_json::Value> = hotwords
        .iter()
        .map(|word| serde_json::json!({"word": word, "class": "user", "frequency": 0}))
        .collect();
    let json = serde_json::json!({
        "hotwords": words,
        "trackingInfo": {"start_type": "shortcut_long"},
    });
    Some(base64(json.to_string().as_bytes()))
}

/// 标准 base64（带补位）
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0_u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// 每帧音频附带的时间戳：这句开始的毫秒数 + 第几帧 × 20 ms
pub fn frame_payload(started_ms: u64, frame: u64) -> String {
    format!(
        r#"{{"extra":{{}},"timestamp_ms":{}}}"#,
        started_ms.saturating_add(frame.saturating_mul(20))
    )
}

/// 豆包这边出的错。`rejected` 表示服务端不认这台虚拟设备（会话失败且一个字没给），下次换一台重新注册。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub status: u64,
    pub message: String,
    pub rejected: bool,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "豆包识别失败 {}：{}", self.status, self.message)
    }
}

impl std::error::Error for Failure {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_payload_asks_for_16k_opus_with_punctuation() {
        let value: serde_json::Value = serde_json::from_str(&session_payload("123", None)).unwrap();
        assert_eq!(value["audio_info"]["format"], "speech_opus");
        assert_eq!(value["audio_info"]["sample_rate"], 16000);
        assert_eq!(value["enable_punctuation"], true);
        assert_eq!(value["extra"]["did"], "123");
        assert!(value["extra"].get("context").is_none());
        let with: serde_json::Value =
            serde_json::from_str(&session_payload("123", Some("abc"))).unwrap();
        assert_eq!(with["extra"]["context"], "abc");
    }

    #[test]
    fn base64_matches_the_standard_alphabet_and_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64("雪耳".as_bytes()), "6Zuq6ICz");
    }

    #[test]
    fn the_context_carries_only_hotwords() {
        assert_eq!(context(&[]), None);
        let encoded = context(&["雪耳".into(), "Husic".into()]).unwrap();
        assert!(encoded.starts_with("eyJob3R3b3JkcyI6"), "{encoded}");
        assert!(!encoded.contains(['{', '"']));
    }

    #[test]
    fn frame_timestamps_advance_20ms_per_frame() {
        assert_eq!(
            frame_payload(1000, 0),
            r#"{"extra":{},"timestamp_ms":1000}"#
        );
        assert_eq!(
            frame_payload(1000, 3),
            r#"{"extra":{},"timestamp_ms":1060}"#
        );
    }
}
