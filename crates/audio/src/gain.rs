//! 自动增益：麦克风离嘴远、声音太小时，送去识别前把它放大到正常音量。
//! 主人 2026-11-16：「收音确实不是太好」。实测主人说话时峰值中位只有满格的 0.5%（约 150），
//! 同一句话压到这个音量云端直接返回「没识别出字」，放大到 3000 左右就全对。
//!
//! 做法：按 20 ms 一帧看峰值，只拿「像说话」的帧（比底噪明显高）来估计说话音量。
//! 说话音量取最近几秒里最响的字（上来马上跟、下去很慢），这样一句话里轻声的字不会把增益拉高；
//! 增益 = 目标峰值 / 说话音量，限制在 1～MAX_GAIN 倍。每一帧再按自己的峰值封顶，保证放大后不削顶。
//! 第一版（e9585d7）目标定得太高、放大太猛，正常音量的字被削顶变形，主人说「声音大的话，反而就更差了」。
use crate::encoder::FRAME_SAMPLES;

/// 放大后希望最响的字落在这里（满格的约 12%）。实验里峰值 1000 以上云端就认得稳，不用放太大
const TARGET_PEAK: f32 = 4_000.;
/// 最多放大这么多倍（约 +24 dB）；再大底噪会盖过说话
pub const MAX_GAIN: f32 = 16.;
/// 任何一帧放大后的峰值都不超过这里（离满格留余量，不削顶）
const CEILING: f32 = 30_000.;
/// 峰值至少是底噪的这么多倍才算说话帧
const SPEECH_OVER_NOISE: f32 = 3.;
/// 说话帧峰值的下限：再小当成底噪，不拿来估增益
const MIN_SPEECH_PEAK: f32 = 60.;

pub struct Gain {
    pending: Vec<i16>,
    /// 底噪峰值的慢速估计
    noise: f32,
    /// 最近说话峰值的估计；还没听到说话时为 None
    speech: Option<f32>,
    /// 当前实际用的增益（平滑过）
    gain: f32,
}

impl Default for Gain {
    fn default() -> Self {
        Self {
            pending: Vec::with_capacity(FRAME_SAMPLES),
            noise: 30.,
            speech: None,
            gain: 1.,
        }
    }
}

impl Gain {
    /// 当前增益（倍数），给日志和测试看
    pub fn current(&self) -> f32 {
        self.gain
    }

    /// 处理一段 PCM，返回放大后的同样长度的 PCM。凑不满一帧的尾巴留到下次（所以输出可能比输入短一点）。
    pub fn process(&mut self, pcm: &[i16]) -> Vec<i16> {
        let mut out = Vec::with_capacity(pcm.len() + FRAME_SAMPLES);
        for &sample in pcm {
            self.pending.push(sample);
            if self.pending.len() == FRAME_SAMPLES {
                let frame = std::mem::replace(&mut self.pending, Vec::with_capacity(FRAME_SAMPLES));
                self.frame(&frame, &mut out);
            }
        }
        out
    }

    /// 把凑不满一帧的尾巴按当前增益放大后吐出来（录音结束时调用）
    pub fn flush(&mut self) -> Vec<i16> {
        let frame = std::mem::take(&mut self.pending);
        frame.iter().map(|&s| amplify(s, self.gain)).collect()
    }

    fn frame(&mut self, frame: &[i16], out: &mut Vec<i16>) {
        let peak = frame.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0) as f32;
        let speaking = peak >= MIN_SPEECH_PEAK && peak >= self.noise * SPEECH_OVER_NOISE;
        if speaking {
            // 说话音量：上来快（大声马上压住）、下去慢（一句话里轻声的字不会被猛放大）
            // 说话音量 = 最近最响的字：更响马上跟上；更轻时每帧只降 0.3%（约 4 秒降一半）
            self.speech = Some(match self.speech {
                Some(level) if peak < level => level * 0.997,
                _ => peak,
            });
        } else {
            self.noise += (peak - self.noise) * if peak < self.noise { 0.2 } else { 0.01 };
        }
        let wanted = self
            .speech
            .map_or(1., |level| (TARGET_PEAK / level).clamp(1., MAX_GAIN));
        // 往小调要快（防爆音），往大调要慢（防一下子把底噪放上来）
        let rate = if wanted < self.gain { 0.5 } else { 0.05 };
        self.gain += (wanted - self.gain) * rate;
        // 这一帧放大后不能超过 CEILING
        let frame_gain = if peak > 0. {
            self.gain.min(CEILING / peak).max(1.)
        } else {
            self.gain
        };
        out.extend(frame.iter().map(|&s| amplify(s, frame_gain)));
    }
}

fn amplify(sample: i16, gain: f32) -> i16 {
    (f32::from(sample) * gain).clamp(-32767., 32767.) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一段「像说话」的声音：220 Hz 正弦，给定峰值和时长（毫秒）
    fn tone(peak: f32, ms: usize) -> Vec<i16> {
        (0..16 * ms)
            .map(|i| (peak * (i as f32 * 220. * std::f32::consts::TAU / 16_000.).sin()) as i16)
            .collect()
    }

    fn peak(pcm: &[i16]) -> i16 {
        pcm.iter().map(|s| s.saturating_abs()).max().unwrap_or(0)
    }

    #[test]
    fn quiet_speech_is_brought_up_to_a_normal_level() {
        // 主人实测：说话峰值中位约 150
        let mut gain = Gain::default();
        let out = gain.process(&tone(150., 3000));
        let tail = &out[out.len() - 16 * 500..];
        assert!(peak(tail) > 2000, "放大后最后半秒峰值 {}", peak(tail));
        assert!(gain.current() > 12. && gain.current() <= MAX_GAIN);
    }

    #[test]
    fn normal_speech_is_left_alone() {
        let mut gain = Gain::default();
        let input = tone(12_000., 2000);
        let out = gain.process(&input);
        assert_eq!(gain.current(), 1.);
        assert_eq!(out, input);
    }

    #[test]
    fn silence_and_background_are_not_pumped_up_before_anyone_speaks() {
        let mut gain = Gain::default();
        // 只有底噪：峰值 40 上下
        let noise: Vec<i16> = (0..16_000 * 3)
            .map(|i| ((i * 7919 % 81) as i16) - 40)
            .collect();
        let out = gain.process(&noise);
        assert_eq!(gain.current(), 1.);
        assert!(peak(&out) <= 41);
    }

    #[test]
    fn a_sudden_loud_sound_does_not_clip_for_long() {
        let mut gain = Gain::default();
        gain.process(&tone(150., 3000));
        // 突然大声（比如凑近麦克风说话）：一帧之内超出的部分硬限幅，几帧后增益就降下来
        let out = gain.process(&tone(8000., 500));
        assert!(out.iter().all(|s| s.unsigned_abs() <= 32767));
        let late = &out[out.len() - 16 * 200..];
        assert!(peak(late) < 32767, "200 ms 后不该还在削顶");
        assert!(gain.current() < 3.);
    }

    #[test]
    fn output_keeps_every_sample_in_order() {
        let mut gain = Gain::default();
        let mut out = gain.process(&tone(150., 33));
        out.extend(gain.process(&tone(150., 7)));
        out.extend(gain.flush());
        assert_eq!(out.len(), 16 * 40);
    }

    /// 真人说话：一个个字（音节）忽大忽小，字之间有短停顿。`loudest` 是最响那个字的峰值。
    fn speech(loudest: f32, seconds: usize) -> Vec<i16> {
        let mut out = Vec::new();
        let levels = [0.25, 0.6, 0.35, 1.0, 0.3, 0.5, 0.8, 0.28];
        for i in 0..seconds * 4 {
            out.extend(tone(loudest * levels[i % levels.len()], 180));
            out.extend(std::iter::repeat_n(0_i16, 16 * 70));
        }
        out
    }

    fn clipped(pcm: &[i16]) -> usize {
        pcm.iter().filter(|s| s.unsigned_abs() >= 32_000).count()
    }

    #[test]
    fn normal_speech_with_uneven_syllables_is_not_clipped() {
        // 主人 2026-11-16：「声音大的话，反而就更差了」。日志里最响的字峰值 1100 时放大了 32 倍，削顶变形
        for loudest in [1_100., 3_000., 6_600.] {
            let mut gain = Gain::default();
            let out = gain.process(&speech(loudest, 4));
            assert_eq!(
                clipped(&out),
                0,
                "最响 {loudest} 时有 {} 个样本削顶",
                clipped(&out)
            );
        }
    }

    #[test]
    fn loud_enough_speech_is_barely_touched() {
        // 原始峰值 1000 以上云端本来就认得出，不该大幅放大
        let mut gain = Gain::default();
        gain.process(&speech(3_000., 4));
        assert!(
            gain.current() <= 1.5,
            "峰值 3000 的说话放大了 {:.1} 倍",
            gain.current()
        );
    }

    #[test]
    fn quiet_uneven_speech_is_still_lifted() {
        // 晚上小声：最响的字只有 150～300，要放大到云端认得出的音量（实验里峰值 1000 以上就稳）
        for loudest in [150., 300.] {
            let mut gain = Gain::default();
            let out = gain.process(&speech(loudest, 4));
            let tail = &out[out.len() - 16_000..];
            assert!(
                peak(tail) >= 1_500,
                "最响 {loudest}：放大后最后一秒峰值只有 {}",
                peak(tail)
            );
            assert_eq!(clipped(&out), 0);
        }
    }
}
