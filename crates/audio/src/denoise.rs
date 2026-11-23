//! 降噪：RNNoise（nnnoiseless 移植，BSD-3）。它只认 48 kHz，所以 16 kHz 先升到 48 kHz、降完噪再降回来。
//!
//! 只拿来给断句判断「有没有人在说话」（见 `Activity::denoised`），不改发给云端的声音。
//! 2026-11-24 实验（风扇声，信噪比 0 dB）：不降噪时风扇声让断句一直以为有人在说话，一分钟的话成了一句，
//! 微信那边几乎全丢；降噪后再发给云端，豆包错字多了快一倍；断句看降噪后的、发原声，两家都和安静时差不多。
use crate::encoder::SAMPLE_RATE;
use nnnoiseless::DenoiseState;
use std::sync::Mutex;

/// RNNoise 一帧：48 kHz 下 10 ms
const FRAME_48K: usize = DenoiseState::FRAME_SIZE;
/// 16 kHz ↔ 48 kHz
const FACTOR: usize = 3;
/// 升降采样共用的低通滤波器长度（奇数：群延迟正好 48 个 48 kHz 样本）
const TAPS: usize = 97;
/// 截止频率：16 kHz 的奈奎斯特频率 8 kHz 以下
const CUTOFF_HZ: f32 = 7_600.;

/// 上一轮学到的底噪，下一轮接着用：RNNoise 要听 1～2 秒才学会一种底噪，每轮从头学的话，
/// 一开口那一两秒的风扇声照样会被当成说话，短句要多等一两秒才收句
static CARRY: Mutex<Option<Box<DenoiseState<'static>>>> = Mutex::new(None);

/// 测试里用到 `CARRY` 的（`resume`、`Activity::denoised`）排队跑，免得并行的测试互相拿走上一轮的底噪
#[cfg(test)]
pub(crate) fn serial() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub struct Denoiser {
    state: Option<Box<DenoiseState<'static>>>,
    /// 用完把学到的底噪留给下一轮
    carry: bool,
    taps: [f32; TAPS],
    /// 最近的 16 kHz 输入（升采样用）
    input: Vec<f32>,
    /// 凑一帧送 RNNoise 的 48 kHz 样本
    frame: Vec<f32>,
    /// 最近的降噪后 48 kHz 样本（降采样用）
    output: Vec<f32>,
    /// 48 kHz 降噪输出数到第几个（每 3 个取 1 个）
    phase: usize,
}

impl Default for Denoiser {
    fn default() -> Self {
        Self::new()
    }
}

impl Denoiser {
    pub fn new() -> Self {
        Self {
            state: Some(DenoiseState::new()),
            carry: false,
            taps: lowpass(),
            input: vec![0.; TAPS.div_ceil(FACTOR)],
            frame: Vec::with_capacity(FRAME_48K),
            output: vec![0.; TAPS],
            phase: 0,
        }
    }

    /// 接着上一轮学到的底噪开始（没有就从头学），用完再留给下一轮
    pub fn resume() -> Self {
        let mut denoiser = Self::new();
        if let Some(state) = CARRY.lock().ok().and_then(|mut carry| carry.take()) {
            denoiser.state = Some(state);
        }
        denoiser.carry = true;
        denoiser
    }

    /// 输出比输入晚多少个 16 kHz 样本：RNNoise 一帧，加升、降采样滤波器各半个长度
    pub fn delay(&self) -> usize {
        (FRAME_48K + TAPS - 1) / FACTOR
    }

    /// 处理一段 16 kHz PCM。每凑满 10 ms 吐出 10 ms，所以累计输出 = 累计输入按 160 个样本向下取整；
    /// 输出第 k 个样本对应输入第 k − `delay()` 个。
    pub fn process(&mut self, pcm: &[i16]) -> Vec<i16> {
        let mut out = Vec::with_capacity(pcm.len() + FRAME_48K / FACTOR);
        for &sample in pcm {
            self.input.rotate_left(1);
            *self.input.last_mut().expect("history is never empty") = f32::from(sample);
            for phase in 0..FACTOR {
                // 补零升采样后过低通 = 每个相位只用到 1/3 的系数（多相）；乘 3 补回补零损失的能量
                let value: f32 = self
                    .input
                    .iter()
                    .rev()
                    .zip(self.taps.iter().skip(phase).step_by(FACTOR))
                    .map(|(x, h)| x * h)
                    .sum();
                self.frame.push(value * FACTOR as f32);
            }
            if self.frame.len() == FRAME_48K {
                self.denoise_frame(&mut out);
            }
        }
        out
    }

    fn denoise_frame(&mut self, out: &mut Vec<i16>) {
        let mut clean = [0.; FRAME_48K];
        self.state
            .as_mut()
            .expect("state is only taken on drop")
            .process_frame(&mut clean, &self.frame);
        self.frame.clear();
        for value in clean {
            self.output.rotate_left(1);
            *self.output.last_mut().expect("history is never empty") = value;
            self.phase = (self.phase + 1) % FACTOR;
            if self.phase == 0 {
                let low: f32 = self
                    .output
                    .iter()
                    .rev()
                    .zip(&self.taps)
                    .map(|(x, h)| x * h)
                    .sum();
                out.push(low.round().clamp(-32_768., 32_767.) as i16);
            }
        }
    }
}

impl Drop for Denoiser {
    fn drop(&mut self) {
        if self.carry
            && let Ok(mut carry) = CARRY.lock()
        {
            *carry = self.state.take();
        }
    }
}

/// 加 Blackman 窗的 sinc 低通（48 kHz 下设计），系数和为 1
fn lowpass() -> [f32; TAPS] {
    let rate = (SAMPLE_RATE as usize * FACTOR) as f32;
    let cutoff = CUTOFF_HZ / rate;
    let middle = (TAPS - 1) as f32 / 2.;
    let mut taps = [0.; TAPS];
    for (i, tap) in taps.iter_mut().enumerate() {
        let t = i as f32 - middle;
        let sinc = if t == 0. {
            2. * cutoff
        } else {
            (2. * std::f32::consts::PI * cutoff * t).sin() / (std::f32::consts::PI * t)
        };
        let x = 2. * std::f32::consts::PI * i as f32 / (TAPS - 1) as f32;
        *tap = sinc * (0.42 - 0.5 * x.cos() + 0.08 * (2. * x).cos());
    }
    let sum: f32 = taps.iter().sum();
    taps.map(|tap| tap / sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(pcm: &[i16]) -> f64 {
        (pcm.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / pcm.len().max(1) as f64).sqrt()
    }

    /// 可复现的白噪声
    fn noise(samples: usize, amplitude: f64) -> Vec<i16> {
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

    #[test]
    fn every_ten_milliseconds_in_gives_ten_milliseconds_out() {
        let mut d = Denoiser::new();
        assert_eq!(d.process(&[0; 100]).len(), 0);
        assert_eq!(d.process(&[0; 100]).len(), 160);
        assert_eq!(d.process(&[0; 1_000]).len(), 960);
        assert_eq!(d.delay(), 192);
    }

    #[test]
    fn silence_stays_silent() {
        let mut d = Denoiser::new();
        assert!(d.process(&[0; 16_000]).iter().all(|&s| s == 0));
    }

    #[test]
    fn a_steady_hiss_is_mostly_removed() {
        let mut d = Denoiser::new();
        let input = noise(48_000, 2_000.);
        let output = d.process(&input);
        // 头一秒让它学会底噪，看后两秒
        let (before, after) = (rms(&input[16_000..]), rms(&output[16_000..]));
        assert!(after < before / 4., "降噪前 {before:.0}，降噪后 {after:.0}");
    }

    #[test]
    fn the_resampling_filter_passes_speech_frequencies_and_blocks_aliases() {
        let taps = lowpass();
        let response = |hz: f32| -> f32 {
            let w = 2. * std::f32::consts::PI * hz / 48_000.;
            let (re, im) = taps.iter().enumerate().fold((0., 0.), |(re, im), (n, h)| {
                (re + h * (w * n as f32).cos(), im - h * (w * n as f32).sin())
            });
            (re * re + im * im).sqrt()
        };
        assert!((response(1_000.) - 1.).abs() < 0.01);
        assert!(response(4_000.) > 0.97);
        assert!(response(10_000.) < 0.001);
        assert!(response(16_000.) < 0.001);
    }

    #[test]
    fn a_resumed_denoiser_already_knows_last_rounds_noise() {
        let _serial = serial();
        // 别的测试留下的底噪（嘶声、说话）会让「上一轮」学得慢：先清掉，只看接着学这一件事。
        // 2026-11-24 打包时排在别的降噪测试后面跑，「从头学 771，接着上一轮 736」没过
        *CARRY.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        let input = noise(16_000, 2_000.);
        let first_second = |d: &mut Denoiser| rms(&d.process(&input));
        let fresh = first_second(&mut Denoiser::new());
        // 上一轮听了 3 秒同样的底噪
        let mut last_round = Denoiser::resume();
        last_round.process(&noise(48_000, 2_000.));
        drop(last_round);
        let resumed = first_second(&mut Denoiser::resume());
        assert!(
            resumed < fresh / 2.,
            "从头学 {fresh:.0}，接着上一轮 {resumed:.0}"
        );
    }
}
