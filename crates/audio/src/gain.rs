//! 自动增益：麦克风离嘴远、声音太小时，送去识别前把它放大到正常音量。
//! 主人 2026-11-16：「收音确实不是太好」。实测主人说话时峰值中位只有满格的 0.5%（约 150），
//! 同一句话压到这个音量云端直接返回「没识别出字」，放大到 3000 左右就全对。
//!
//! 做法：按 20 ms 一帧看峰值，只拿「像说话」的帧（比底噪明显高）来估计说话音量，
//! 增益 = 目标峰值 / 最近说话峰值，限制在 1～MAX_GAIN 倍；增益变化要平滑，免得忽大忽小。
//! 底噪不单独估增益，跟着说话的增益一起放大；放大后超过满格的样本硬限幅。
use crate::encoder::FRAME_SAMPLES;

/// 放大后希望说话峰值落在这里（满格的约 30%，留足余量防爆音）
const TARGET_PEAK: f32 = 10_000.;
/// 最多放大这么多倍（约 +30 dB）；再大底噪会盖过说话
pub const MAX_GAIN: f32 = 32.;
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
            self.speech = Some(match self.speech {
                None => peak,
                Some(level) if peak > level => level + (peak - level) * 0.5,
                Some(level) => level + (peak - level) * 0.02,
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
        out.extend(frame.iter().map(|&s| amplify(s, self.gain)));
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
        assert!(peak(tail) > 3000, "放大后最后半秒峰值 {}", peak(tail));
        assert!(gain.current() > 15. && gain.current() <= MAX_GAIN);
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
}
