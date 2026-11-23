//! WeType turns over our own socket. Capture/VAD and native UI contracts are shared.
use super::{Counts, Event};
use crate::{Result, wetype::Client};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use xueer_ear_protocol::{transcript::SpeechEvent, wetype};

pub(super) struct Turn {
    client: Client,
    key: String,
    sequence: u32,
    audio_bytes: u64,
    last_text: String,
    final_result: bool,
}

impl Turn {
    pub(super) fn start(stop: &AtomicBool) -> Result<Option<Self>> {
        let client = match Client::from_default_device_cancellable(&|| stop.load(Ordering::Acquire))
        {
            Ok(client) => client,
            Err(_) if stop.load(Ordering::Acquire) => return Ok(None),
            Err(error) => return Err(error),
        };
        if stop.load(Ordering::Acquire) {
            return Ok(None);
        }
        let mut entropy = [0; 16];
        getrandom::fill(&mut entropy).map_err(|_| "session random generation failed")?;
        let key = format!(
            "{:024}",
            u128::from_le_bytes(entropy) % 1_000_000_000_000_000_000_000_000
        );
        Ok(Some(Self {
            client,
            key,
            sequence: 0,
            audio_bytes: 0,
            last_text: String::new(),
            final_result: false,
        }))
    }

    pub(super) fn upload(&mut self, audio: &[u8]) -> Result<()> {
        if self.final_result {
            return Ok(());
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or("speech sequence exceeded")?;
        self.audio_bytes = self
            .audio_bytes
            .checked_add(audio.len() as u64)
            .ok_or("speech size exceeded")?;
        let body = wetype::encode_request(&wetype::Request {
            session_key: &self.key,
            audio: Some(audio),
            chunk_sequence: Some(self.sequence),
            end: false,
            total_audio_bytes: self.audio_bytes,
            target_process: "wechat-winh.exe",
        })?;
        self.client.send_voice(&body)?;
        Ok(())
    }

    fn end(&mut self, first: bool) -> Result<()> {
        let sequence = first
            .then(|| {
                self.sequence
                    .checked_add(1)
                    .ok_or("speech sequence exceeded")
            })
            .transpose()?;
        let body = wetype::encode_request(&wetype::Request {
            session_key: &self.key,
            audio: None,
            chunk_sequence: sequence,
            end: true,
            total_audio_bytes: self.audio_bytes,
            target_process: "wechat-winh.exe",
        })?;
        self.client.send_voice(&body)?;
        Ok(())
    }

    pub(super) fn poll(
        &mut self,
        stop: &AtomicBool,
        counts: &mut Counts,
        output: &mut impl FnMut(Event) -> Result<()>,
    ) -> Result<()> {
        let Some(response) = self.client.poll()? else {
            return Ok(());
        };
        counts.responses += 1;
        let response = wetype::decode_response(&response.body)?;
        if response.session_key != self.key {
            return Err("unexpected WeType speech session".into());
        }
        if self.final_result || stop.load(Ordering::Acquire) {
            return Ok(());
        }
        let text = response.polished_text.unwrap_or(response.raw_text);
        let final_result = response.is_final();
        if text != self.last_text || final_result {
            self.last_text = text.into();
            self.final_result = final_result;
            if final_result {
                counts.finals += 1;
            } else {
                counts.partials += 1;
            }
            output(Event::Speech(SpeechEvent {
                text: text.into(),
                final_result,
            }))?;
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
            self.end(true)?;
            let deadline = Instant::now() + Duration::from_secs(4);
            let mut last_end = Instant::now();
            while !self.final_result && Instant::now() < deadline && !stop.load(Ordering::Acquire) {
                self.poll(stop, counts, output)?;
                if !self.final_result && last_end.elapsed() >= Duration::from_millis(100) {
                    // End responses can still contain revisable raw/polished text.
                    // Keep requesting the final snapshot until its actual flag.
                    self.end(false)?;
                    last_end = Instant::now();
                }
            }
        }
        if stop.load(Ordering::Acquire) {
            return Ok(String::new());
        }
        if !self.final_result && !self.last_text.is_empty() {
            eprintln!("本地收句：等待最终结果超时");
            // Bound finalization without leaving native composition stuck forever.
            counts.finals += 1;
            output(Event::Speech(SpeechEvent {
                text: self.last_text.clone(),
                final_result: true,
            }))?;
        }
        Ok(std::mem::take(&mut self.last_text))
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        self.client.close();
    }
}
