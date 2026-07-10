//! Audio preparation ported from the Ito server (server/src/utils/audio.ts
//! and audioProcessing.ts): light enhancement of 16-bit PCM plus WAV header
//! creation, and a minimal WAV reader for the CLI test mode.

pub const SAMPLE_RATE: u32 = 16000;

/// Light audio enhancement for 16-bit PCM mono at a given sample rate.
/// - Removes DC offset
/// - Applies a gentle high-pass filter (~80 Hz)
/// - Peak normalizes to ~-3 dBFS with a capped gain (only when gain > 1.05)
pub fn enhance_pcm16(samples: &[i16], sample_rate: u32) -> Vec<i16> {
    let sample_count = samples.len();
    if sample_count == 0 {
        return Vec::new();
    }

    // DC offset removal (integer mean, like the TS implementation)
    let sum: i64 = samples.iter().map(|&s| s as i64).sum();
    let mean = sum / sample_count as i64;
    let dc_removed: Vec<f32> = samples.iter().map(|&s| (s as i64 - mean) as f32).collect();

    // Gentle high-pass filter (~80 Hz)
    let fc = 80.0_f32;
    let a = (-2.0 * std::f32::consts::PI * fc / sample_rate as f32).exp();
    let mut prev_x = 0.0_f32;
    let mut prev_y = 0.0_f32;
    let mut filtered = Vec::with_capacity(sample_count);
    for &x in &dc_removed {
        let y = a * (prev_y + x - prev_x);
        filtered.push(y);
        prev_x = x;
        prev_y = y;
    }

    // Peak normalize to ~-3 dBFS, cap max gain to ~+12 dB
    let mut peak = 1.0_f32;
    for &v in &filtered {
        let v = v.abs();
        if v > peak {
            peak = v;
        }
    }
    let target = 0.707 * 32767.0;
    let gain = (target / peak).min(4.0);

    let effective_gain = if gain > 1.05 { gain } else { 1.0 };
    filtered
        .iter()
        .map(|&v| (v * effective_gain).round().clamp(-32768.0, 32767.0) as i16)
        .collect()
}

/// Standard 44-byte PCM WAV header.
#[allow(dead_code)] // kept for symmetry with the server pipeline; used in tests
pub fn create_wav_header(
    data_length: u32,
    sample_rate: u32,
    channel_count: u16,
    bit_depth: u16,
) -> [u8; 44] {
    let byte_rate = sample_rate * channel_count as u32 * (bit_depth as u32 / 8);
    let block_align = channel_count * (bit_depth / 8);

    let mut header = [0u8; 44];
    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&(36 + data_length).to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(b"fmt ");
    header[16..20].copy_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    header[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    header[22..24].copy_from_slice(&channel_count.to_le_bytes());
    header[24..28].copy_from_slice(&sample_rate.to_le_bytes());
    header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    header[32..34].copy_from_slice(&block_align.to_le_bytes());
    header[34..36].copy_from_slice(&bit_depth.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&data_length.to_le_bytes());
    header
}

pub fn duration_ms(sample_count: usize, sample_rate: u32) -> u64 {
    (sample_count as u64 * 1000) / sample_rate as u64
}

/// Minimal WAV reader for the CLI test mode: expects 16-bit PCM. Returns
/// (samples of the first channel, sample_rate).
pub fn read_wav_pcm16(bytes: &[u8]) -> Result<(Vec<i16>, u32), String> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("Not a RIFF/WAVE file".to_string());
    }

    let mut pos = 12usize;
    let mut sample_rate = 0u32;
    let mut channels = 1u16;
    let mut bit_depth = 16u16;
    let mut data: Option<&[u8]> = None;

    while pos + 8 <= bytes.len() {
        let chunk_id = &bytes[pos..pos + 4];
        let chunk_len = u32::from_le_bytes([
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ]) as usize;
        let body_start = pos + 8;
        let body_end = (body_start + chunk_len).min(bytes.len());

        match chunk_id {
            b"fmt " if chunk_len >= 16 => {
                channels = u16::from_le_bytes([bytes[body_start + 2], bytes[body_start + 3]]);
                sample_rate = u32::from_le_bytes([
                    bytes[body_start + 4],
                    bytes[body_start + 5],
                    bytes[body_start + 6],
                    bytes[body_start + 7],
                ]);
                bit_depth = u16::from_le_bytes([bytes[body_start + 14], bytes[body_start + 15]]);
            }
            b"data" => data = Some(&bytes[body_start..body_end]),
            _ => {}
        }
        // Chunks are word-aligned
        pos = body_start + chunk_len + (chunk_len & 1);
    }

    if bit_depth != 16 {
        return Err(format!(
            "Only 16-bit PCM WAV is supported, got {bit_depth}-bit"
        ));
    }
    let data = data.ok_or("No data chunk found")?;

    let mut samples = Vec::with_capacity(data.len() / 2 / channels as usize);
    let frame_bytes = 2 * channels as usize;
    let mut i = 0;
    while i + 1 < data.len() {
        samples.push(i16::from_le_bytes([data[i], data[i + 1]]));
        i += frame_bytes;
    }
    Ok((samples, sample_rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_layout() {
        let header = create_wav_header(1000, 16000, 1, 16);
        assert_eq!(&header[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(header[4..8].try_into().unwrap()), 1036);
        assert_eq!(&header[8..12], b"WAVE");
        assert_eq!(
            u32::from_le_bytes(header[24..28].try_into().unwrap()),
            16000
        );
        assert_eq!(u16::from_le_bytes(header[22..24].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(header[34..36].try_into().unwrap()), 16);
        assert_eq!(u32::from_le_bytes(header[40..44].try_into().unwrap()), 1000);
    }

    #[test]
    fn wav_roundtrip() {
        let samples: Vec<i16> = (0..1600)
            .map(|i| ((i % 100) * 300 - 15000) as i16)
            .collect();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&create_wav_header(samples.len() as u32 * 2, 16000, 1, 16));
        for s in &samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        let (parsed, rate) = read_wav_pcm16(&bytes).unwrap();
        assert_eq!(rate, 16000);
        assert_eq!(parsed, samples);
    }

    #[test]
    fn enhance_removes_dc_offset() {
        // Constant signal = pure DC; after enhancement it should be ~zero
        let samples = vec![1000i16; 16000];
        let out = enhance_pcm16(&samples, 16000);
        let max = out.iter().map(|s| s.abs()).max().unwrap();
        assert!(max < 100, "residual DC too high: {max}");
    }

    #[test]
    fn enhance_boosts_quiet_speech_band_signal() {
        // Quiet 440 Hz tone (in the passband) should get gain applied
        let samples: Vec<i16> = (0..16000)
            .map(|i| {
                let t = i as f32 / 16000.0;
                ((t * 440.0 * 2.0 * std::f32::consts::PI).sin() * 1000.0) as i16
            })
            .collect();
        let out = enhance_pcm16(&samples, 16000);
        let peak_in = samples.iter().map(|s| s.abs()).max().unwrap() as f32;
        let peak_out = out.iter().map(|s| s.abs()).max().unwrap() as f32;
        assert!(
            peak_out > peak_in * 2.0,
            "expected gain: in={peak_in} out={peak_out}"
        );
        // Gain is capped at 4x
        assert!(peak_out <= peak_in * 4.2);
    }

    #[test]
    fn duration_calc() {
        assert_eq!(duration_ms(16000, 16000), 1000);
        assert_eq!(duration_ms(1600, 16000), 100);
        assert_eq!(duration_ms(0, 16000), 0);
    }
}
