//! 一句话的豆包识别会话：连上 → StartTask → StartSession → 一帧一帧发 Opus → 末帧 + FinishSession → 等 SessionFinished。
//! 不依赖雪耳的引擎事件，WSL 里也能跑（联网测试见下面的 `live_probe`）。
use super::{
    APP_KEY, Failure, device::Device, frame_payload, session_payload, transcript::Transcript,
    transport::Transport,
};
use crate::Result;
use std::time::{Duration, Instant};
use xueer_ear_protocol::doubao::{FrameState, Request, STATUS_DISCOVERY, STATUS_OK};

/// 握手每一步最多等多久
const HANDSHAKE: Duration = Duration::from_secs(5);

pub struct Session {
    transport: Transport,
    request_id: String,
    started_ms: u64,
    frames: u64,
    transcript: Transcript,
    finished: bool,
}

/// 一条回包带来的变化
#[derive(Debug, PartialEq, Eq)]
pub enum Update {
    /// 这句现在的全文
    Text(String),
    /// 服务端说这句完了（SessionFinished）
    Finished,
}

impl Session {
    pub fn start(device: &Device, now_ms: u64, cancelled: &dyn Fn() -> bool) -> Result<Self> {
        let mut transport = Transport::connect(device, cancelled)?;
        let request_id = request_id()?;
        transport.send(&Request {
            token: APP_KEY,
            method: "StartTask",
            payload: "",
            audio: &[],
            request_id: &request_id,
            frame: None,
        })?;
        expect(&mut transport, "TaskStarted", cancelled)?;
        transport.send(&Request {
            token: APP_KEY,
            method: "StartSession",
            payload: &session_payload(&device.did),
            audio: &[],
            request_id: &request_id,
            frame: None,
        })?;
        expect(&mut transport, "SessionStarted", cancelled)?;
        Ok(Self {
            transport,
            request_id,
            started_ms: now_ms,
            frames: 0,
            transcript: Transcript::default(),
            finished: false,
        })
    }

    /// 发一帧 20 ms 的 Opus 包
    pub fn send_audio(&mut self, packet: &[u8]) -> Result<()> {
        let frame = if self.frames == 0 {
            FrameState::First
        } else {
            FrameState::Middle
        };
        self.transport.send(&Request {
            token: "",
            method: "TaskRequest",
            payload: &frame_payload(self.started_ms, self.frames),
            audio: packet,
            request_id: &self.request_id,
            frame: Some(frame),
        })?;
        self.frames += 1;
        Ok(())
    }

    /// 这句说完了：发末帧和 FinishSession，之后等 `Update::Finished`
    pub fn finish_audio(&mut self) -> Result<()> {
        self.transport.send(&Request {
            token: "",
            method: "TaskRequest",
            payload: &frame_payload(self.started_ms, self.frames),
            audio: &[],
            request_id: &self.request_id,
            frame: Some(FrameState::Last),
        })?;
        self.transport.send(&Request {
            token: APP_KEY,
            method: "FinishSession",
            payload: "",
            audio: &[],
            request_id: &self.request_id,
            frame: None,
        })
    }

    /// 读一条回包。文字变了给 `Text`，这句完了给 `Finished`；服务端报失败返回 [`Failure`]。
    pub fn poll(&mut self) -> Result<Option<Update>> {
        let Some(response) = self.transport.poll()? else {
            return Ok(None);
        };
        if response.failed() {
            return Err(Box::new(Failure {
                status: response.status,
                message: response.message,
                // 一个字都没给就失败：多半是这台虚拟设备不被认了
                rejected: self.transcript.text().is_empty(),
            }));
        }
        if !response.result.is_empty()
            && let Some(text) = self.transcript.update(&response.result)
        {
            return Ok(Some(Update::Text(text)));
        }
        if response.event == "SessionFinished" {
            self.finished = true;
            return Ok(Some(Update::Finished));
        }
        Ok(None)
    }

    pub fn text(&self) -> String {
        self.transcript.text()
    }

    pub fn finished(&self) -> bool {
        self.finished
    }

    /// 已经发了多少帧（每帧 20 ms）
    pub fn frames(&self) -> u64 {
        self.frames
    }

    pub fn close(&mut self) {
        self.transport.close();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.close();
    }
}

/// 握手的一步：等到 `event`；中途失败或状态码不对就报 [`Failure`]
fn expect(transport: &mut Transport, event: &str, cancelled: &dyn Fn() -> bool) -> Result<()> {
    let deadline = Instant::now() + HANDSHAKE;
    while Instant::now() < deadline {
        if cancelled() {
            return Err("已取消".into());
        }
        let Some(response) = transport.poll()? else {
            continue;
        };
        if response.failed() || (response.event == event && response.status != STATUS_OK) {
            return Err(Box::new(Failure {
                rejected: response.status == STATUS_DISCOVERY || response.event == "SessionFailed",
                status: response.status,
                message: response.message,
            }));
        }
        if response.event == event {
            return Ok(());
        }
    }
    Err(format!("豆包握手超时：等 {event}").into())
}

fn request_id() -> Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| "random generation failed")?;
    Ok(super::device::uuid(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 联网实测（默认不跑）：DOUBAO_DEVICE_FILE 指向测试用的设备文件（没有就用 curl 注册一台），
    /// DOUBAO_PROBE_WAV 是 16 kHz 单声道 WAV。按真实语速发完，打印识别结果。
    /// `cargo test -p xueer-ear-backend live_probe -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_probe() {
        use super::super::device::{Identity, REGISTER_HOSTS, parse_registration, registration};
        let path = Device::path().unwrap();
        let device = match Device::load(&path).unwrap() {
            Some(device) => device,
            None => {
                let identity = Identity::random(now_ms()).unwrap();
                let request = registration(REGISTER_HOSTS[0], &identity);
                let mut curl = std::process::Command::new("curl");
                curl.args(["-sS", "-w", "\n%{http_code}", "-X", "POST"]);
                for (k, v) in &request.headers {
                    curl.args(["-H", &format!("{k}: {v}")]);
                }
                let output = curl
                    .args(["--data-binary", &request.body])
                    .arg(format!("https://{}{}", request.host, request.path))
                    .output()
                    .unwrap();
                let text = String::from_utf8_lossy(&output.stdout);
                let (body, status) = text.rsplit_once('\n').unwrap();
                let did =
                    parse_registration(status.trim().parse().unwrap(), body.as_bytes()).unwrap();
                let device = Device {
                    did,
                    phone: identity_phone(&identity),
                };
                device.save(&path).unwrap();
                device
            }
        };
        println!("device {}", device.did);
        let wav = std::env::var_os("DOUBAO_PROBE_WAV").expect("DOUBAO_PROBE_WAV");
        let samples = xueer_ear_audio::wave::read(std::path::Path::new(&wav)).unwrap();
        let started = Instant::now();
        let mut session = Session::start(&device, now_ms(), &|| false).unwrap();
        println!("handshake {} ms", started.elapsed().as_millis());
        let mut encoder = xueer_ear_audio::encoder::OpusStream::new().unwrap();
        let frame = xueer_ear_audio::encoder::FRAME_SAMPLES;
        let begin = Instant::now();
        for (i, pcm) in samples.chunks(frame).enumerate() {
            let mut pcm = pcm.to_vec();
            pcm.resize(frame, 0);
            let chunk = encoder.encode(&pcm).unwrap();
            for packet in xueer_ear_audio::encoder::packets(&chunk).unwrap() {
                session.send_audio(packet).unwrap();
            }
            while begin.elapsed() < Duration::from_millis(i as u64 * 20) {
                if let Some(update) = session.poll().unwrap() {
                    println!("{:>6} ms {update:?}", begin.elapsed().as_millis());
                }
            }
        }
        session.finish_audio().unwrap();
        let ended = Instant::now();
        while !session.finished() && ended.elapsed() < Duration::from_secs(10) {
            if let Some(update) = session.poll().unwrap() {
                println!("{:>6} ms {update:?}", begin.elapsed().as_millis());
            }
        }
        println!(
            "final after {} ms: {}",
            ended.elapsed().as_millis(),
            session.text()
        );
        assert!(session.finished());
        assert!(!session.text().is_empty());
    }

    fn identity_phone(identity: &super::super::device::Identity) -> super::super::device::Phone {
        super::super::device::Phone::pick(identity.seed)
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }
}
