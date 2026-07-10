//! Microphone capture with downmixing and resampling to 16 kHz mono i16 PCM.
//!
//! The library exposes the capture pipeline so it can be used both by the
//! standalone binary (framed stdout protocol, used by the Electron app) and
//! in-process by other crates (e.g. the `ito-tray` app) through an
//! [`AudioSink`] implementation.

use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{Sample, SampleFormat, StreamConfig};
use dasp_sample::FromSample;
use rubato::{FftFixedIn, Resampler};
use std::sync::Arc;

use anyhow::{anyhow, Result};

/// All audio leaves the pipeline at this rate, 16-bit mono.
pub const TARGET_SAMPLE_RATE: u32 = 16000;

/// Effective input/output configuration reported when capture starts.
#[derive(Debug, Clone, Copy)]
pub struct AudioConfigInfo {
    pub input_sample_rate: u32,
    pub output_sample_rate: u32,
    pub channels: u8,
}

/// Receives pipeline output. Implementations must be cheap and non-blocking;
/// `on_chunk` is called from a dedicated writer thread with 16 kHz mono i16
/// samples. `on_drain_complete` fires once after `CaptureSession::stop`, when
/// all buffered audio has been flushed.
pub trait AudioSink: Send + Sync + 'static {
    fn on_config(&self, config: AudioConfigInfo);
    fn on_chunk(&self, pcm: &[i16]);
    fn on_drain_complete(&self);
}

/// Creates the preferred cpal host for the current platform.
/// On Windows prefers WASAPI directly for best performance (10-30 ms latency
/// vs DirectSound's 50-80 ms).
pub fn create_host() -> cpal::Host {
    #[cfg(target_os = "windows")]
    {
        match cpal::host_from_id(cpal::platform::HostId::Wasapi) {
            Ok(wasapi_host) => {
                eprintln!("[audio-recorder] Using WASAPI host (optimal for Windows)");
                wasapi_host
            }
            Err(e) => {
                eprintln!(
                    "[audio-recorder] WASAPI unavailable ({}), falling back to default",
                    e
                );
                cpal::default_host()
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        cpal::default_host()
    }
}

/// Lists input device names for the host.
pub fn list_input_devices(host: &cpal::Host) -> Vec<String> {
    match host.input_devices() {
        Ok(devices) => devices
            .map(|d| d.name().unwrap_or_else(|_| "Unknown Device".to_string()))
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Finds an input device by name; `None`, `""` or `"default"` selects the
/// system default input device.
pub fn find_input_device(host: &cpal::Host, device_name: Option<&str>) -> Option<cpal::Device> {
    match device_name {
        Some(name) if !name.is_empty() && name.to_lowercase() != "default" => host
            .input_devices()
            .ok()
            .and_then(|mut it| it.find(|d| d.name().unwrap_or_default() == name)),
        _ => host.default_input_device(),
    }
}

/// Reports the device's maximum supported input sample rate (used by the
/// binary's `get-device-config` command).
pub fn device_input_rate(host: &cpal::Host, device_name: Option<&str>) -> u32 {
    find_input_device(host, device_name)
        .and_then(|d| d.supported_input_configs().ok())
        .and_then(|mut cfgs| cfgs.find(|r| r.channels() > 0))
        .map(|cfg| cfg.with_max_sample_rate().sample_rate().0)
        .unwrap_or(TARGET_SAMPLE_RATE)
}

/// Selects the dominant channel to avoid amplitude loss when one channel is
/// near-silent.
pub fn downmix_to_mono_vec<T>(data: &[T], num_channels: usize) -> Vec<f32>
where
    T: Sample,
    f32: FromSample<T>,
{
    if num_channels <= 1 {
        return data.iter().map(|s| s.to_sample::<f32>()).collect();
    }
    let frames = data.len() / num_channels;
    if frames == 0 {
        return Vec::new();
    }

    let mut energy_per_channel: Vec<f32> = vec![0.0; num_channels];
    for frame_idx in 0..frames {
        let base = frame_idx * num_channels;
        for c in 0..num_channels {
            let v = data[base + c].to_sample::<f32>();
            energy_per_channel[c] += v * v;
        }
    }
    let mut best_channel = 0usize;
    let mut best_energy = energy_per_channel[0];
    #[allow(clippy::needless_range_loop)]
    for c in 1..num_channels {
        if energy_per_channel[c] > best_energy {
            best_energy = energy_per_channel[c];
            best_channel = c;
        }
    }

    let mut out: Vec<f32> = Vec::with_capacity(frames);
    for frame_idx in 0..frames {
        let base = frame_idx * num_channels;
        out.push(data[base + best_channel].to_sample::<f32>());
    }
    out
}

/// Converts clamped f32 samples to i16.
pub fn f32_to_i16(data: &[f32]) -> Vec<i16> {
    data.iter()
        .map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
        .collect()
}

// Linear resampler fallback for mono when FFT resampler isn't available
fn linear_resample_mono(input: &[f32], in_rate: u32, out_rate: u32) -> Vec<f32> {
    if input.is_empty() || in_rate == 0 || in_rate == out_rate {
        return input.to_vec();
    }
    let in_len = input.len();
    let ratio = out_rate as f32 / in_rate as f32;
    let out_len = ((in_len as f32) * ratio).round().max(0.0) as usize;
    if out_len <= 1 {
        return Vec::new();
    }
    let step = in_rate as f32 / out_rate as f32;
    let mut out = Vec::with_capacity(out_len);
    let mut pos: f32 = 0.0;
    for _ in 0..out_len {
        let idx = pos.floor() as usize;
        if idx >= in_len - 1 {
            out.push(input[in_len - 1]);
        } else {
            let frac = pos - (idx as f32);
            let a = input[idx];
            let b = input[idx + 1];
            out.push(a + (b - a) * frac);
        }
        pos += step;
    }
    out
}

fn writer_loop(
    audio_rx: crossbeam_channel::Receiver<Vec<f32>>,
    sink: Arc<dyn AudioSink>,
    input_sample_rate: u32,
) {
    const RESAMPLER_CHUNK_SIZE_DEFAULT: usize = 1024;
    const RESAMPLER_CHUNK_SIZE_FALLBACK: usize = 512;

    let emit = |data: &[f32]| sink.on_chunk(&f32_to_i16(data));

    // Try FFT resampler with default size, then fallback chunk size
    let mut chosen_chunk_size: usize = RESAMPLER_CHUNK_SIZE_DEFAULT;
    let mut resampler_opt = if input_sample_rate != TARGET_SAMPLE_RATE {
        match FftFixedIn::new(
            input_sample_rate as usize,
            TARGET_SAMPLE_RATE as usize,
            chosen_chunk_size,
            1,
            1,
        ) {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!(
                    "[audio-recorder] CRITICAL: Failed to create resampler ({}), trying fallback chunk size",
                    e
                );
                chosen_chunk_size = RESAMPLER_CHUNK_SIZE_FALLBACK;
                match FftFixedIn::new(
                    input_sample_rate as usize,
                    TARGET_SAMPLE_RATE as usize,
                    chosen_chunk_size,
                    1,
                    1,
                ) {
                    Ok(r2) => Some(r2),
                    Err(e2) => {
                        eprintln!(
                            "[audio-recorder] CRITICAL: Fallback resampler creation failed ({}), using linear fallback",
                            e2
                        );
                        None
                    }
                }
            }
        }
    } else {
        None
    };

    let mut in_buffer: Vec<f32> = Vec::new();

    while let Ok(frame) = audio_rx.recv() {
        if let Some(resampler) = resampler_opt.as_mut() {
            in_buffer.extend_from_slice(&frame);
            while in_buffer.len() >= chosen_chunk_size {
                let chunk_to_process: Vec<f32> =
                    in_buffer.drain(..chosen_chunk_size).collect::<Vec<_>>();
                match resampler.process(&[chunk_to_process], None) {
                    Ok(mut resampled) => {
                        if !resampled.is_empty() {
                            emit(&resampled.remove(0));
                        }
                    }
                    Err(e) => eprintln!(
                        "[audio-recorder] CRITICAL: Resampling failed in writer: {}",
                        e
                    ),
                }
            }
        } else if input_sample_rate != TARGET_SAMPLE_RATE {
            let resampled = linear_resample_mono(&frame, input_sample_rate, TARGET_SAMPLE_RATE);
            if !resampled.is_empty() {
                emit(&resampled);
            }
        } else {
            emit(&frame);
        }
    }

    // Channel closed; flush any remaining buffered samples through resampler
    if let Some(mut resampler) = resampler_opt.take() {
        while !in_buffer.is_empty() {
            let take = if in_buffer.len() >= chosen_chunk_size {
                chosen_chunk_size
            } else {
                in_buffer.len()
            };
            let mut chunk = in_buffer.drain(..take).collect::<Vec<_>>();
            if chunk.len() < chosen_chunk_size {
                // zero-pad final chunk to meet resampler size
                chunk.resize(chosen_chunk_size, 0.0);
            }
            if let Ok(mut resampled) = resampler.process(&[chunk], None) {
                if !resampled.is_empty() {
                    emit(&resampled.remove(0));
                }
            }
        }
    } else if !in_buffer.is_empty() {
        if input_sample_rate != TARGET_SAMPLE_RATE {
            let resampled = linear_resample_mono(&in_buffer, input_sample_rate, TARGET_SAMPLE_RATE);
            if !resampled.is_empty() {
                emit(&resampled);
            }
        } else {
            emit(&in_buffer);
        }
    }

    sink.on_drain_complete();
}

/// A running capture. Dropping it stops the stream, but call [`stop`] to also
/// flush the pipeline and wait for `on_drain_complete`.
///
/// Note: `cpal::Stream` is not `Send`; keep the session on the thread that
/// created it.
///
/// [`stop`]: CaptureSession::stop
pub struct CaptureSession {
    stream: cpal::Stream,
    audio_tx: crossbeam_channel::Sender<Vec<f32>>,
    writer_handle: std::thread::JoinHandle<()>,
}

impl CaptureSession {
    pub fn stream(&self) -> &cpal::Stream {
        &self.stream
    }

    /// Stops capture, flushes buffered audio through the resampler, and waits
    /// for the writer thread to finish (the sink receives
    /// `on_drain_complete`).
    pub fn stop(self) {
        use cpal::traits::StreamTrait;
        let _ = self.stream.pause();
        drop(self.stream);
        // Close audio channel to signal writer thread to exit
        drop(self.audio_tx);
        let _ = self.writer_handle.join();
    }
}

/// Starts capturing from the given device (see [`find_input_device`] for name
/// semantics) and streams 16 kHz mono i16 chunks into `sink`. The returned
/// session's stream is already playing.
pub fn start_capture(
    host: &cpal::Host,
    device_name: Option<&str>,
    sink: Arc<dyn AudioSink>,
) -> Result<CaptureSession> {
    use cpal::traits::StreamTrait;
    const QUEUE_CAPACITY: usize = 512;

    let device = find_input_device(host, device_name)
        .ok_or_else(|| anyhow!("[audio-recorder] Failed to find input device"))?;

    // Prefer the device's default input configuration instead of max rate to
    // better align with other apps (e.g., Zoom) and reduce host resampling.
    let default_config = device
        .default_input_config()
        .map_err(|_| anyhow!("[audio-recorder] No default input config found"))?;

    let input_sample_rate = default_config.sample_rate().0;
    let input_sample_format = default_config.sample_format();
    let channels_count: usize = default_config.channels() as usize;

    let err_fn = |err| eprintln!("[audio-recorder] Stream error: {}", err);
    let stream_config: StreamConfig = default_config.clone().into();

    // Writer thread and queue
    let (audio_tx, audio_rx) = crossbeam_channel::bounded::<Vec<f32>>(QUEUE_CAPACITY);
    let sink_for_writer = Arc::clone(&sink);
    let writer_handle = std::thread::spawn(move || {
        writer_loop(audio_rx, sink_for_writer, input_sample_rate);
    });

    // Notify the sink about input and effective output audio configuration
    sink.on_config(AudioConfigInfo {
        input_sample_rate,
        output_sample_rate: TARGET_SAMPLE_RATE,
        channels: 1,
    });

    macro_rules! build_stream {
        ($sample_ty:ty) => {{
            let tx = audio_tx.clone();
            device.build_input_stream(
                &stream_config,
                move |data: &[$sample_ty], _| {
                    let mono = downmix_to_mono_vec(data, channels_count);
                    let _ = tx.try_send(mono);
                },
                err_fn,
                None,
            )?
        }};
    }

    let stream = match input_sample_format {
        SampleFormat::F32 => build_stream!(f32),
        SampleFormat::I16 => build_stream!(i16),
        SampleFormat::U16 => build_stream!(u16),
        SampleFormat::U8 => build_stream!(u8),
        SampleFormat::I32 => build_stream!(i32),
        SampleFormat::F64 => build_stream!(f64),
        SampleFormat::U32 => build_stream!(u32),
        format => {
            return Err(anyhow!(
                "[audio-recorder] Unsupported sample format {}",
                format
            ))
        }
    };

    stream.play()?;

    Ok(CaptureSession {
        stream,
        audio_tx,
        writer_handle,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_downmix_to_mono_single_channel() {
        let mono_samples: Vec<f32> = vec![0.5, -0.5, 1.0, -1.0];
        let result = downmix_to_mono_vec(&mono_samples, 1);

        assert_eq!(result.len(), 4);
        assert_eq!(result, vec![0.5, -0.5, 1.0, -1.0]);
    }

    #[test]
    fn test_downmix_to_mono_stereo() {
        // Stereo: L,R,L,R pattern
        let stereo_samples: Vec<f32> = vec![0.8, 0.2, -0.6, -0.4];
        let result = downmix_to_mono_vec(&stereo_samples, 2);

        assert_eq!(result.len(), 2);
        assert_eq!(result[0], 0.8); // Left channel sample 1
        assert_eq!(result[1], -0.6); // Left channel sample 2
    }

    #[test]
    fn test_downmix_to_mono_quad() {
        // 4 channels: one frame with values [1.0, 0.5, 0.25, 0.25]
        let quad_samples: Vec<f32> = vec![1.0, 0.5, 0.25, 0.25]; // One frame
        let result = downmix_to_mono_vec(&quad_samples, 4);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0], 1.0); // Channel 0 sample
    }

    #[test]
    fn test_downmix_partial_frame() {
        // 5 samples with 2 channels - last sample incomplete, should be ignored
        let samples: Vec<f32> = vec![0.8, 0.2, -0.6, -0.4, 1.0];
        let result = downmix_to_mono_vec(&samples, 2);

        assert_eq!(result.len(), 2); // Only 2 complete frames
        assert_eq!(result[0], 0.8); // Left channel sample 1
        assert_eq!(result[1], -0.6); // Left channel sample 2
    }

    #[test]
    fn test_f32_to_i16_clamps_and_scales() {
        let out = f32_to_i16(&[0.0, 1.0, -1.0, 2.0, -2.0]);
        assert_eq!(out, vec![0, 32767, -32767, 32767, -32767]);
    }
}
