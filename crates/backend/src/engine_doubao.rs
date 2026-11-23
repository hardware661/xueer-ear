//! 豆包这条的一句话：一段话一个豆包会话。接口和微信那条的 Turn 一样（start / upload / poll / finish），
//! 好让引擎按句在两条之间切换。
use super::{Counts, Event};
use crate::{
    Result,
    doubao::{
        Failure,
        device::Device,
        session::{Session, Update},
    },
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use xueer_ear_protocol::transcript::SpeechEvent;

pub(super) struct Turn {
    session: Session,
    last_text: String,
    final_result: bool,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// 服务端不认这台虚拟设备了：删掉，下一句重新注册一台
fn forget_rejected(error: &(dyn std::error::Error + Send + Sync + 'static)) {
    if error.downcast_ref::<Failure>().is_some_and(|f| f.rejected) {
        Device::forget();
    }
}

impl Turn {
    pub(super) fn start(stop: &AtomicBool) -> Result<Option<Self>> {
        let cancelled = || stop.load(Ordering::Acquire);
        let device = Device::load_or_register(now_ms())?;
        if cancelled() {
            return Ok(None);
        }
        match Session::start(&device, now_ms(), &cancelled) {
            Ok(session) => Ok(Some(Self {
                session,
                last_text: String::new(),
                final_result: false,
            })),
            Err(_) if cancelled() => Ok(None),
            Err(error) => {
                forget_rejected(error.as_ref());
                Err(error)
            }
        }
    }

    pub(super) fn upload(&mut self, chunk: &[u8]) -> Result<()> {
        if self.final_result {
            return Ok(());
        }
        for packet in xueer_ear_audio::encoder::packets(chunk)? {
            self.session.send_audio(packet)?;
        }
        Ok(())
    }

    pub(super) fn poll(
        &mut self,
        stop: &AtomicBool,
        counts: &mut Counts,
        output: &mut impl FnMut(Event) -> Result<()>,
    ) -> Result<()> {
        let update = match self.session.poll() {
            Ok(update) => update,
            Err(error) => {
                forget_rejected(error.as_ref());
                return Err(error);
            }
        };
        let Some(update) = update else {
            return Ok(());
        };
        counts.responses += 1;
        if self.final_result || stop.load(Ordering::Acquire) {
            return Ok(());
        }
        match update {
            Update::Text(text) => {
                self.last_text.clone_from(&text);
                counts.partials += 1;
                output(Event::Speech(SpeechEvent {
                    text,
                    final_result: false,
                }))?;
            }
            Update::Finished => {
                self.final_result = true;
                counts.finals += 1;
                output(Event::Speech(SpeechEvent {
                    text: self.session.text(),
                    final_result: true,
                }))?;
            }
        }
        Ok(())
    }

    pub(super) fn finish(
        mut self,
        stop: &AtomicBool,
        counts: &mut Counts,
        output: &mut impl FnMut(Event) -> Result<()>,
    ) -> Result<String> {
        if stop.load(Ordering::Acquire) {
            return Ok(String::new());
        }
        if !self.final_result {
            self.session.finish_audio()?;
            // 豆包说完后会把整段再认一遍（实测 6 秒的话 0.35 秒回来）；长句多给一点，最多再加 10 秒
            let extra = Duration::from_millis((self.session.frames() * 5).min(10_000));
            let deadline = Instant::now() + Duration::from_secs(4) + extra;
            while !self.final_result && Instant::now() < deadline && !stop.load(Ordering::Acquire) {
                self.poll(stop, counts, output)?;
            }
        }
        if stop.load(Ordering::Acquire) {
            return Ok(String::new());
        }
        if !self.final_result && !self.last_text.is_empty() {
            eprintln!("本地收句：等待豆包最终结果超时");
            counts.finals += 1;
            output(Event::Speech(SpeechEvent {
                text: self.last_text.clone(),
                final_result: true,
            }))?;
            return Ok(std::mem::take(&mut self.last_text));
        }
        Ok(self.session.text())
    }
}
