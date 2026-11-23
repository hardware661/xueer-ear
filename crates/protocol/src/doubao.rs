//! Doubao IME streaming ASR frames (protobuf over WebSocket).
//!
//! An independent implementation of the request/response shapes that several
//! unofficial clients documented publicly (VoiceKey, doubao-asr-free,
//! doubao-asr-web). Requests: 2 token, 3 service, 5 method, 6 JSON payload,
//! 7 audio (one 20 ms Opus packet), 8 request id, 9 frame state. Responses:
//! 4 event, 5 status (20000000 = OK), 6 message, 7 result JSON. Transport,
//! device registration and the result JSON live in the backend.
use crate::wire::{self, Error, Value};

pub const SERVICE: &str = "ASR";
pub const STATUS_OK: u64 = 20_000_000;
/// "service discovery failure": random upstream hiccup, or a device the
/// service no longer accepts. Retry once, then register a new device.
pub const STATUS_DISCOVERY: u64 = 50_700_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameState {
    First = 1,
    Middle = 3,
    Last = 9,
}

/// One frame to the service. Empty strings and audio are omitted on the wire.
pub struct Request<'a> {
    pub token: &'a str,
    pub method: &'a str,
    pub payload: &'a str,
    pub audio: &'a [u8],
    pub request_id: &'a str,
    pub frame: Option<FrameState>,
}

pub fn encode_request(request: &Request) -> Vec<u8> {
    let mut output = Vec::with_capacity(request.audio.len() + request.payload.len() + 64);
    if !request.token.is_empty() {
        wire::put_bytes(&mut output, 2, request.token.as_bytes());
    }
    wire::put_bytes(&mut output, 3, SERVICE.as_bytes());
    wire::put_bytes(&mut output, 5, request.method.as_bytes());
    if !request.payload.is_empty() {
        wire::put_bytes(&mut output, 6, request.payload.as_bytes());
    }
    if !request.audio.is_empty() {
        wire::put_bytes(&mut output, 7, request.audio);
    }
    wire::put_bytes(&mut output, 8, request.request_id.as_bytes());
    if let Some(frame) = request.frame {
        wire::put_integer(&mut output, 9, frame as u64);
    }
    output
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Response {
    /// TaskStarted, SessionStarted, SessionFinished, TaskFailed, SessionFailed;
    /// empty on plain result frames.
    pub event: String,
    pub status: u64,
    pub message: String,
    /// Result JSON (`{"results": [...]}`), or empty.
    pub result: String,
}

impl Response {
    pub fn failed(&self) -> bool {
        self.event.ends_with("Failed")
    }
}

pub fn decode_response(data: &[u8]) -> Result<Response, Error> {
    if data.len() > wire::MAX_FRAME {
        return Err(Error::TooLarge);
    }
    let mut response = Response::default();
    for field in wire::fields(data)? {
        let field = field?;
        match (field.number, field.value) {
            (4, Value::Bytes(value)) => response.event = utf8(value)?,
            (5, Value::Varint(value)) => response.status = value,
            (6, Value::Bytes(value)) => response.message = utf8(value)?,
            (7, Value::Bytes(value)) => response.result = utf8(value)?,
            (4..=7, _) => return Err(Error::WrongType),
            _ => {}
        }
    }
    Ok(response)
}

fn utf8(value: &[u8]) -> Result<String, Error> {
    String::from_utf8(value.to_vec()).map_err(|_| Error::InvalidUtf8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_task_carries_token_service_method_and_id_only() {
        let bytes = encode_request(&Request {
            token: "k",
            method: "StartTask",
            payload: "",
            audio: &[],
            request_id: "r",
            frame: None,
        });
        let mut expected = vec![0x12, 1, b'k', 0x1a, 3, b'A', b'S', b'R', 0x2a, 9];
        expected.extend_from_slice(b"StartTask");
        expected.extend_from_slice(&[0x42, 1, b'r']);
        assert_eq!(bytes, expected);
    }

    #[test]
    fn audio_frames_carry_payload_audio_and_frame_state() {
        let bytes = encode_request(&Request {
            token: "",
            method: "TaskRequest",
            payload: "{}",
            audio: &[7, 8],
            request_id: "r",
            frame: Some(FrameState::Middle),
        });
        assert_eq!(wire::text(&bytes, 2).unwrap(), None);
        assert_eq!(wire::text(&bytes, 5).unwrap(), Some("TaskRequest"));
        assert_eq!(wire::text(&bytes, 6).unwrap(), Some("{}"));
        assert_eq!(wire::bytes(&bytes, 7).unwrap(), Some(&[7_u8, 8][..]));
        assert_eq!(wire::integer(&bytes, 9).unwrap(), Some(3));
    }

    #[test]
    fn responses_decode_event_status_message_and_result() {
        let mut data = Vec::new();
        wire::put_bytes(&mut data, 2, b"task");
        wire::put_bytes(&mut data, 4, b"SessionFailed");
        wire::put_integer(&mut data, 5, STATUS_DISCOVERY);
        wire::put_bytes(&mut data, 6, b"service discovery failure");
        wire::put_bytes(&mut data, 7, br#"{"results":[]}"#);
        let response = decode_response(&data).unwrap();
        assert!(response.failed());
        assert_eq!(response.status, STATUS_DISCOVERY);
        assert_eq!(response.message, "service discovery failure");
        assert_eq!(response.result, r#"{"results":[]}"#);
    }

    #[test]
    fn a_bare_result_frame_has_no_event() {
        let mut data = Vec::new();
        wire::put_bytes(&mut data, 7, br#"{"results":[{"text":"hi"}]}"#);
        let response = decode_response(&data).unwrap();
        assert_eq!(response.event, "");
        assert!(!response.failed());
        assert_eq!(response.status, 0);
    }

    #[test]
    fn wrong_field_types_and_bad_text_are_rejected() {
        let mut data = Vec::new();
        wire::put_integer(&mut data, 4, 1);
        assert_eq!(decode_response(&data), Err(Error::WrongType));
        let mut data = Vec::new();
        wire::put_bytes(&mut data, 7, &[0xff, 0xfe]);
        assert_eq!(decode_response(&data), Err(Error::InvalidUtf8));
    }
}
