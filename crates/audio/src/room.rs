//! 屋里吵不吵：说话的声音比底噪响多少倍。
//!
//! 主人 2026-11-24 开风扇念 8 句的真实录音（按听写时一样自动放大）：豆包错字 6% → 14～17%，
//! 微信 5% → 7%。那段录音里说话只比底噪响 5 倍左右，关风扇时一百多倍。
//! 所以上一轮吵（不到 [`NOISY_BELOW`] 倍）时，下一轮让微信为主（快捷键听写里定）。
//!
//! 按 20 ms 一帧的音量记两张直方图：说话的帧、不说话的帧。底噪取不说话帧的第 10 百分位，
//! 说话音量取说话帧的第 90 百分位，两边都至少听够 1 秒才算数。

use std::sync::atomic::{AtomicU8, Ordering};

/// 说话比底噪响不到这么多倍就算吵
pub const NOISY_BELOW: f64 = 20.;
/// 说话、不说话各至少这么多帧（1 秒）才下结论
const ENOUGH_FRAMES: u32 = 50;
/// 直方图：每倍频 4 格，从音量 1/4 起
const BINS_PER_OCTAVE: f64 = 4.;
const LOWEST: f64 = 0.25;
const BINS: usize = 76;

#[derive(Debug, Clone)]
pub struct Room {
    voice: [u32; BINS],
    quiet: [u32; BINS],
}

impl Default for Room {
    fn default() -> Self {
        Self {
            voice: [0; BINS],
            quiet: [0; BINS],
        }
    }
}

impl Room {
    /// 记一帧：这一帧的音量（去掉直流后的均方根），以及断句判断它是不是在说话
    pub fn frame(&mut self, rms: f64, voice: bool) {
        let bin = ((rms.max(LOWEST) / LOWEST).log2() * BINS_PER_OCTAVE) as usize;
        let bin = bin.min(BINS - 1);
        if voice {
            self.voice[bin] += 1;
        } else {
            self.quiet[bin] += 1;
        }
    }

    /// 说话比底噪响多少倍；说话或不说话的时间不到 1 秒时 None
    pub fn speech_over_noise(&self) -> Option<f64> {
        let speech = percentile(&self.voice, 0.9)?;
        let noise = percentile(&self.quiet, 0.1)?;
        Some(speech / noise)
    }

    /// 吵不吵；听得不够时 None
    pub fn noisy(&self) -> Option<bool> {
        self.speech_over_noise().map(|ratio| ratio < NOISY_BELOW)
    }
}

/// 直方图的第 `q` 分位（格子的中间值）；总数不够时 None
fn percentile(counts: &[u32; BINS], q: f64) -> Option<f64> {
    let total: u32 = counts.iter().sum();
    if total < ENOUGH_FRAMES {
        return None;
    }
    let wanted = (f64::from(total) * q).ceil().max(1.) as u32;
    let mut seen = 0;
    for (bin, &count) in counts.iter().enumerate() {
        seen += count;
        if seen >= wanted {
            return Some(LOWEST * ((bin as f64 + 0.5) / BINS_PER_OCTAVE).exp2());
        }
    }
    None
}

/// 上一轮听写的结论：0 不知道、1 安静、2 吵
static LAST_ROUND: AtomicU8 = AtomicU8::new(0);

/// 一轮结束时记下这轮吵不吵（听得不够就不改上一次的结论）
pub fn remember(noisy: Option<bool>) {
    if let Some(noisy) = noisy {
        LAST_ROUND.store(if noisy { 2 } else { 1 }, Ordering::Relaxed);
    }
}

/// 上一轮吵不吵；还没听够过是 None
pub fn last_round_noisy() -> Option<bool> {
    match LAST_ROUND.load(Ordering::Relaxed) {
        1 => Some(false),
        2 => Some(true),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(speech: f64, noise: f64) -> Room {
        let mut room = Room::default();
        for i in 0..100 {
            // 说话的音量有大有小，底噪也有起伏
            room.frame(speech * (0.5 + f64::from(i % 10) / 10.), true);
            room.frame(noise * (0.8 + f64::from(i % 5) / 10.), false);
        }
        room
    }

    // 数字取自主人 2026-11-24 的录音：说话的帧音量约 70～110；关风扇时底噪 0.5 左右，开风扇约 19
    #[test]
    fn a_fan_makes_the_room_noisy_and_a_quiet_room_is_not() {
        assert_eq!(room(100., 19.).noisy(), Some(true));
        assert_eq!(room(70., 0.5).noisy(), Some(false));
        let ratio = room(100., 19.).speech_over_noise().unwrap();
        assert!((3.0..10.0).contains(&ratio), "{ratio}");
    }

    #[test]
    fn too_little_speech_or_silence_gives_no_verdict() {
        let mut room = Room::default();
        for _ in 0..200 {
            room.frame(0.5, false);
        }
        assert_eq!(room.noisy(), None, "没说话");
        for _ in 0..20 {
            room.frame(80., true);
        }
        assert_eq!(room.noisy(), None, "说话不到 1 秒");
        for _ in 0..40 {
            room.frame(80., true);
        }
        assert_eq!(room.noisy(), Some(false));
    }

    #[test]
    fn the_verdict_carries_over_until_a_round_hears_enough() {
        remember(Some(true));
        assert_eq!(last_round_noisy(), Some(true));
        remember(None);
        assert_eq!(last_round_noisy(), Some(true));
        remember(Some(false));
        assert_eq!(last_round_noisy(), Some(false));
    }
}
