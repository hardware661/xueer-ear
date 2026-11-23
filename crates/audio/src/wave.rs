//! Bounded PCM input for reproducible codec/backend verification.
use std::{error::Error, path::Path};

pub fn read(path: &Path) -> Result<Vec<i16>, Box<dyn Error + Send + Sync>> {
    if std::fs::metadata(path)?.len() > 4_000_000 {
        return Err("verification WAV exceeds two minutes".into());
    }
    decode(&std::fs::read(path)?)
}

/// 16 kHz 单声道 16 位 PCM 写成 WAV（`record-wave` 录音用，`decode` 能原样读回）
pub fn encode(samples: &[i16]) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&[1, 0, 1, 0]);
    out.extend_from_slice(&16_000_u32.to_le_bytes());
    out.extend_from_slice(&32_000_u32.to_le_bytes());
    out.extend_from_slice(&[2, 0, 16, 0]);
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

pub fn decode(bytes: &[u8]) -> Result<Vec<i16>, Box<dyn Error + Send + Sync>> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("invalid WAV header".into());
    }
    let end = u32::from_le_bytes(bytes[4..8].try_into()?) as usize + 8;
    if end > bytes.len() {
        return Err("truncated WAV".into());
    }
    let (mut position, mut format, mut data) = (12, false, None);
    while position < end {
        let header = bytes
            .get(position..position + 8)
            .ok_or("truncated WAV chunk")?;
        let size = u32::from_le_bytes(header[4..8].try_into()?) as usize;
        position += 8;
        let next = position
            .checked_add(size)
            .filter(|&n| n <= end)
            .ok_or("truncated WAV data")?;
        let chunk = &bytes[position..next];
        match &header[..4] {
            b"fmt " => {
                format = chunk.len() >= 16
                    && chunk[..2] == [1, 0]
                    && chunk[2..4] == [1, 0]
                    && chunk[4..8] == 16_000_u32.to_le_bytes()
                    && chunk[12..14] == [2, 0]
                    && chunk[14..16] == [16, 0];
            }
            b"data" => {
                data = Some(chunk);
            }
            _ => (),
        }
        position = next + (size & 1);
    }
    if !format {
        return Err("WAV must be 16 kHz, 16-bit mono PCM".into());
    }
    let data = data.ok_or("WAV contains no audio")?;
    if data.is_empty() || data.len() % 2 != 0 {
        return Err("invalid PCM sample count".into());
    }
    Ok(data
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_wav_reads_back_the_same_samples() {
        let samples: Vec<i16> = (0..1600).map(|i| (i * 37 % 2001 - 1000) as i16).collect();
        assert_eq!(decode(&encode(&samples)).unwrap(), samples);
    }
}
