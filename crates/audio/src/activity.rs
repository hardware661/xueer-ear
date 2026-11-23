//! Local, sample-clocked turn detection. No audio leaves this module while idle.
use crate::{
    denoise::Denoiser,
    encoder::{FRAME_SAMPLES, SAMPLE_RATE},
    room::Room,
};
use std::{borrow::Cow, collections::VecDeque};
use webrtc_vad::{SampleRate, Vad, VadMode};

pub const SILENCE_SAMPLES: usize = SAMPLE_RATE as usize * 3;
/// 开口前多留一点（原 360 ms；主人历史里句首常丢一两个字，2026-11-19 加到 600 ms）
pub const PREROLL_SAMPLES: usize = SAMPLE_RATE as usize * 600 / 1000;
const ATTACK_FRAMES: usize = 3; // 60 ms; one click must not open a connection.
/// 停了至少这么久再开口才算「又开始说」（字与字之间的小停顿不算），见 [`ActivityEvent::Voice`]
const VOICE_GAP_SAMPLES: usize = SAMPLE_RATE as usize * 200 / 1000;

#[derive(Debug, PartialEq, Eq)]
pub enum ActivityEvent {
    /// 停了一会儿（至少 200 ms）后又开口了：前面停了多少毫秒；这一轮还没听到过说话时是 `u32::MAX`。
    /// 快捷键听写拿它判断语音指令前有没有停够（主人 2026-11-24：先停 1 秒，再单独说指令）
    Voice(u32),
    /// Includes the onset and up to 600 ms of preceding audio.
    Started(Vec<i16>),
    Audio(Vec<i16>),
    /// The preceding Audio includes the configured amount of detected silence.
    Ended,
}

pub struct Activity {
    vad: Vad,
    pending: Vec<i16>,
    /// 和 `pending` 对齐的、用来判断有没有人说话的那份声音（没开降噪时不用）
    pending_detect: Vec<i16>,
    denoise: Option<Denoised>,
    preroll: VecDeque<i16>,
    attack: usize,
    silent: usize,
    silence_samples: usize,
    active: bool,
    noise: f64,
    /// 上次听到说话以后连着多少个样本没说话；还没听到过说话是 None
    quiet: Option<usize>,
    /// 连着几帧是说话（不到 `ATTACK_FRAMES` 帧的算杂音，不算开口）
    run: usize,
    /// 这一轮说话比底噪响多少倍（看原声，不看降噪后的）
    room: Room,
}

/// 断句看降噪后的声音，发出去的是原声：原声按降噪的延迟排队，和降噪后的声音逐个样本对齐
struct Denoised {
    denoiser: Denoiser,
    raw: VecDeque<i16>,
}

impl Denoised {
    fn new() -> Self {
        let denoiser = Denoiser::resume();
        let raw = std::iter::repeat_n(0, denoiser.delay()).collect();
        Self { denoiser, raw }
    }
}

impl Default for Activity {
    fn default() -> Self {
        Self::with_silence_seconds(3).expect("valid default silence threshold")
    }
}

impl Activity {
    pub fn with_silence_seconds(seconds: u32) -> Result<Self, &'static str> {
        if !matches!(seconds, 1 | 2 | 3 | 5 | 10) {
            return Err("unsupported silence threshold");
        }
        Ok(Self {
            vad: Vad::new_with_rate_and_mode(SampleRate::Rate16kHz, VadMode::Quality),
            pending: Vec::with_capacity(FRAME_SAMPLES),
            pending_detect: Vec::with_capacity(FRAME_SAMPLES),
            denoise: None,
            preroll: VecDeque::with_capacity(PREROLL_SAMPLES),
            attack: 0,
            silent: 0,
            silence_samples: SAMPLE_RATE as usize * seconds as usize,
            active: false,
            noise: 10.0,
            quiet: None,
            run: 0,
            room: Room::default(),
        })
    }

    /// 判断有没有人说话时先降噪（风扇、空调这种一直响的底噪不再让它以为一直有人在说话）。
    /// 发出去的仍是原声，只晚 12 ms：云端对原声认得更准（见 `denoise` 模块说明）。
    pub fn denoised(mut self) -> Self {
        self.denoise = Some(Denoised::new());
        self
    }

    pub fn silence_samples(&self) -> usize {
        self.silence_samples
    }

    /// 这一轮到目前为止屋里吵不吵
    pub fn room(&self) -> &Room {
        &self.room
    }

    /// WASAPI packet boundaries do not affect the silence deadline.
    pub fn push(&mut self, pcm: &[i16]) -> Vec<ActivityEvent> {
        let (detect, audio): (Cow<[i16]>, Cow<[i16]>) = match self.denoise.as_mut() {
            None => (Cow::Borrowed(pcm), Cow::Borrowed(pcm)),
            Some(denoise) => {
                denoise.raw.extend(pcm);
                let clean = denoise.denoiser.process(pcm);
                let raw: Vec<i16> = denoise.raw.drain(..clean.len()).collect();
                (Cow::Owned(clean), Cow::Owned(raw))
            }
        };
        let mut events = Vec::new();
        for (&heard, &sample) in detect.iter().zip(audio.iter()) {
            self.pending.push(sample);
            self.pending_detect.push(heard);
            if self.pending.len() == FRAME_SAMPLES {
                let frame = std::mem::replace(&mut self.pending, Vec::with_capacity(FRAME_SAMPLES));
                let heard =
                    std::mem::replace(&mut self.pending_detect, Vec::with_capacity(FRAME_SAMPLES));
                // Remove DC bias when measuring energy; WebRTC supplies the actual
                // speech/noise decision, rather than treating all loud sound as voice.
                let mean = heard.iter().map(|&s| f64::from(s)).sum::<f64>() / FRAME_SAMPLES as f64;
                let rms = (heard
                    .iter()
                    .map(|&s| (f64::from(s) - mean).powi(2))
                    .sum::<f64>()
                    / FRAME_SAMPLES as f64)
                    .sqrt();
                let voice = self
                    .vad
                    .is_voice_segment(&heard)
                    .expect("fixed 16 kHz / 20 ms VAD frame")
                    && rms >= (self.noise * 2.5).max(80.0);
                if !self.active && !voice {
                    // Follow slowly changing background, without learning speech as noise.
                    self.noise = self.noise * 0.98 + rms.min(3000.0) * 0.02;
                }
                self.room.frame(frame_rms(&frame), voice);
                self.pause(voice, &mut events);
                self.frame(frame, voice, &mut events);
            }
        }
        events
    }

    /// 停了一会儿后又开口时报一次停了多久。连着 `ATTACK_FRAMES` 帧都是说话才算开口，
    /// 一下咔哒声、短促的杂音照样算没说话
    fn pause(&mut self, voice: bool, events: &mut Vec<ActivityEvent>) {
        if !voice {
            let blip = if self.run < ATTACK_FRAMES {
                self.run
            } else {
                0
            };
            self.quiet = self.quiet.map(|quiet| quiet + (blip + 1) * FRAME_SAMPLES);
            self.run = 0;
            return;
        }
        self.run += 1;
        if self.run < ATTACK_FRAMES {
            return;
        }
        if self.run == ATTACK_FRAMES {
            match self.quiet {
                None => events.push(ActivityEvent::Voice(u32::MAX)),
                Some(quiet) if quiet >= VOICE_GAP_SAMPLES => events.push(ActivityEvent::Voice(
                    u32::try_from(quiet * 1000 / SAMPLE_RATE as usize).unwrap_or(u32::MAX),
                )),
                Some(_) => {}
            }
        }
        self.quiet = Some(0);
    }

    fn frame(&mut self, frame: Vec<i16>, voice: bool, events: &mut Vec<ActivityEvent>) {
        if self.active {
            self.silent = if voice { 0 } else { self.silent + frame.len() };
            events.push(ActivityEvent::Audio(frame));
            if self.silent >= self.silence_samples {
                self.active = false;
                self.attack = 0;
                self.silent = 0;
                events.push(ActivityEvent::Ended);
            }
        } else {
            self.preroll.extend(frame);
            if self.preroll.len() > PREROLL_SAMPLES {
                self.preroll.drain(..self.preroll.len() - PREROLL_SAMPLES);
            }
            self.attack = if voice { self.attack + 1 } else { 0 };
            if self.attack >= ATTACK_FRAMES {
                self.active = true;
                self.silent = 0;
                events.push(ActivityEvent::Started(self.preroll.drain(..).collect()));
            }
        }
    }

    /// Explicit stop / finite WAV EOF flushes a partial PCM frame and the turn.
    pub fn finish(&mut self) -> Vec<ActivityEvent> {
        let mut events = Vec::new();
        if self.active {
            // 还在排队等降噪结果的原声也是这句的尾巴
            let mut tail = std::mem::take(&mut self.pending);
            if let Some(denoise) = self.denoise.as_mut() {
                tail.extend(denoise.raw.drain(..));
            }
            if !tail.is_empty() {
                events.push(ActivityEvent::Audio(tail));
            }
            self.active = false;
            events.push(ActivityEvent::Ended);
        }
        // 先放下旧的（把学到的底噪留下来），再接着它开始
        if self.denoise.take().is_some() {
            self.denoise = Some(Denoised::new());
        }
        self.pending.clear();
        self.pending_detect.clear();
        self.preroll.clear();
        self.attack = 0;
        self.silent = 0;
        events
    }
}

/// 一帧原声的音量：去掉直流后的均方根
fn frame_rms(frame: &[i16]) -> f64 {
    let mean = frame.iter().map(|&s| f64::from(s)).sum::<f64>() / frame.len().max(1) as f64;
    (frame
        .iter()
        .map(|&s| (f64::from(s) - mean).powi(2))
        .sum::<f64>()
        / frame.len().max(1) as f64)
        .sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice(samples: usize) -> Vec<i16> {
        // A voiced harmonic excitation with slowly varying pitch and amplitude.
        let mut phase = 0.0_f64;
        (0..samples)
            .map(|i| {
                phase += std::f64::consts::TAU * (145.0 + 25.0 * (i as f64 / 1400.0).sin())
                    / f64::from(SAMPLE_RATE);
                ((phase.sin() + 0.4 * (phase * 3.0).sin() + 0.2 * (phase * 7.0).sin())
                    * (2400.0 + 900.0 * (i as f64 / 530.0).sin())) as i16
            })
            .collect()
    }

    fn pauses(events: &[ActivityEvent]) -> Vec<u32> {
        events
            .iter()
            .filter_map(|e| match e {
                ActivityEvent::Voice(ms) => Some(*ms),
                _ => None,
            })
            .collect()
    }

    // 主人 2026-11-24：语音指令前要停够 1 秒。开口时报前面停了多久，字之间的小停顿不报
    #[test]
    fn each_new_start_of_speech_reports_how_long_the_pause_before_it_was() {
        let mut gate = Activity::with_silence_seconds(3).unwrap();
        gate.push(&vec![0; 8_000]);
        // 这一轮第一次开口：前面没有话
        assert_eq!(pauses(&gate.push(&voice(8_000))), [u32::MAX]);
        // 停 100 ms 接着说：不算
        gate.push(&vec![0; 1_600]);
        assert_eq!(pauses(&gate.push(&voice(8_000))), [] as [u32; 0]);
        // 停 1.2 秒再说（还在同一句里）
        gate.push(&vec![0; 19_200]);
        let resumed = pauses(&gate.push(&voice(8_000)));
        assert_eq!(resumed.len(), 1);
        assert!((1_150..=1_250).contains(&resumed[0]), "{resumed:?}");
        // 停 4 秒（一句已经说完）再开口
        gate.push(&vec![0; 64_000]);
        let next = pauses(&gate.push(&voice(8_000)));
        assert!((3_950..=4_050).contains(&next[0]), "{next:?}");
    }

    #[test]
    fn quiet_noise_and_single_click_do_not_activate() {
        let mut gate = Activity::default();
        let mut noise = vec![0; SAMPLE_RATE as usize * 5];
        for (i, s) in noise.iter_mut().enumerate() {
            *s = (i % 5) as i16 - 2;
        }
        noise[16_000] = 21_000;
        assert!(gate.push(&noise).is_empty());
        assert!(gate.finish().is_empty());
        assert!(gate.preroll.len() <= PREROLL_SAMPLES);
    }

    #[test]
    fn three_seconds_are_sample_counted_and_speaking_rearms() {
        let mut gate = Activity::default();
        gate.push(&vec![0; 16_000]);
        let first = gate.push(&voice(16_000));
        let starts: Vec<_> = first
            .iter()
            .filter_map(|e| match e {
                ActivityEvent::Started(pcm) => Some(pcm),
                _ => None,
            })
            .collect();
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0].len(), PREROLL_SAMPLES);
        // Exactly 2999.9375 ms is insufficient, independent of poll timing.
        let before = gate.push(&vec![0; SILENCE_SAMPLES - 1]);
        assert!(!before.iter().any(|e| matches!(e, ActivityEvent::Ended)));
        assert_eq!(gate.push(&[0]).last(), Some(&ActivityEvent::Ended));
        assert!(gate.push(&vec![0; 160_000]).is_empty());
        let second = gate.push(&voice(16_000));
        assert_eq!(
            second
                .iter()
                .filter(|e| matches!(e, ActivityEvent::Started(_)))
                .count(),
            1
        );
        assert_eq!(
            gate.push(&vec![0; SILENCE_SAMPLES]).last(),
            Some(&ActivityEvent::Ended)
        );
        assert!(gate.finish().is_empty());
    }

    #[test]
    fn short_pause_preserves_turn_and_eof_flushes_partial_frame() {
        let mut gate = Activity::default();
        assert!(
            gate.push(&voice(16_000))
                .iter()
                .any(|e| matches!(e, ActivityEvent::Started(_)))
        );
        assert!(
            !gate
                .push(&vec![0; 32_000])
                .iter()
                .any(|e| matches!(e, ActivityEvent::Ended))
        );
        assert!(
            !gate
                .push(&voice(16_003))
                .iter()
                .any(|e| matches!(e, ActivityEvent::Started(_)))
        );
        let tail = gate.finish();
        assert!(matches!(&tail[0], ActivityEvent::Audio(pcm) if pcm.len() == 3));
        assert_eq!(tail.last(), Some(&ActivityEvent::Ended));
        assert!(gate.finish().is_empty());
    }

    #[test]
    fn every_configured_threshold_ends_at_its_exact_pcm_sample_and_rearms() {
        for seconds in [1, 2, 3, 5, 10] {
            let mut gate = Activity::with_silence_seconds(seconds).unwrap();
            assert!(
                gate.push(&voice(16_000))
                    .iter()
                    .any(|e| matches!(e, ActivityEvent::Started(_)))
            );
            let samples = SAMPLE_RATE as usize * seconds as usize;
            assert_eq!(gate.silence_samples(), samples);
            assert!(
                !gate
                    .push(&vec![0; samples - 1])
                    .iter()
                    .any(|e| matches!(e, ActivityEvent::Ended))
            );
            assert_eq!(gate.push(&[0]).last(), Some(&ActivityEvent::Ended));
            assert!(gate.push(&vec![0; 16_000]).is_empty());
            assert!(
                gate.push(&voice(16_000))
                    .iter()
                    .any(|e| matches!(e, ActivityEvent::Started(_)))
            );
        }
        for invalid in [0, 4, 6, 11, u32::MAX] {
            assert!(Activity::with_silence_seconds(invalid).is_err());
        }
    }

    /// 可复现的白噪声（风扇声的简化）
    fn hiss(samples: usize, amplitude: f64) -> Vec<i16> {
        let mut seed = 0x2545_f491_u32;
        (0..samples)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                ((f64::from(seed) / f64::from(u32::MAX) * 2. - 1.) * amplitude) as i16
            })
            .collect()
    }

    /// 2 秒噪声，2 秒说话（带同样的噪声），再 4 秒噪声；返回各事件发生在第几毫秒、发出去的全部声音
    fn run_in_hiss(mut gate: Activity) -> (Vec<String>, Vec<i16>) {
        let mut signal = hiss(32_000, 3_000.);
        let speech = voice(32_000);
        let noise = hiss(32_000, 3_000.);
        signal.extend(speech.iter().zip(&noise).map(|(a, b)| a.saturating_add(*b)));
        signal.extend(hiss(64_000, 3_000.));
        let (mut log, mut sent) = (Vec::new(), Vec::new());
        for (i, chunk) in signal.chunks(1_600).enumerate() {
            for event in gate.push(chunk) {
                match event {
                    ActivityEvent::Started(pcm) => {
                        log.push(format!("start@{}", i * 100));
                        sent.extend(pcm);
                    }
                    ActivityEvent::Audio(pcm) => sent.extend(pcm),
                    ActivityEvent::Ended => log.push(format!("end@{}", i * 100)),
                    ActivityEvent::Voice(_) => {}
                }
            }
        }
        (log, sent)
    }

    #[test]
    fn steady_loud_noise_holds_a_turn_open_unless_denoised() {
        let _serial = crate::denoise::serial();
        let gate = || Activity::with_silence_seconds(1).unwrap();
        let (plain, _) = run_in_hiss(gate());
        assert!(!plain.iter().any(|e| e.starts_with("end")), "{plain:?}");
        let (denoised, _) = run_in_hiss(gate().denoised());
        assert!(
            denoised.iter().any(|e| e.starts_with("end")),
            "说完 4 秒内要能收句：{denoised:?}"
        );
    }

    #[test]
    fn with_denoising_the_sound_sent_is_the_original_only_later() {
        let _serial = crate::denoise::serial();
        let mut gate = Activity::with_silence_seconds(1).unwrap().denoised();
        let delay = Denoiser::new().delay();
        let mut input = vec![0; 16_000];
        input.extend(voice(16_000));
        input.extend(vec![0; 40_000]);
        let mut sent = Vec::new();
        for chunk in input.chunks(1_000) {
            for event in gate.push(chunk) {
                if let ActivityEvent::Started(pcm) | ActivityEvent::Audio(pcm) = event {
                    sent.extend(pcm);
                }
            }
        }
        assert!(!sent.is_empty());
        let delayed: Vec<i16> = std::iter::repeat_n(0, delay).chain(input).collect();
        assert!(
            delayed.windows(sent.len()).any(|w| w == sent.as_slice()),
            "发出去的必须是原声的一段，不能是降噪后的"
        );
    }

    #[test]
    fn finishing_a_denoised_turn_sends_the_queued_tail() {
        let _serial = crate::denoise::serial();
        let mut gate = Activity::default().denoised();
        let speech = voice(16_000);
        let mut sent: Vec<i16> = Vec::new();
        for event in gate.push(&speech).into_iter().chain(gate.finish()) {
            if let ActivityEvent::Started(pcm) | ActivityEvent::Audio(pcm) = event {
                sent.extend(pcm);
            }
        }
        // 开头多出延迟那么多个 0，原声一个不少
        assert!(sent.ends_with(&speech[speech.len() - 1_000..]));
    }
}
