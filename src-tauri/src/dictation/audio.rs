//! Microphone capture. The cpal stream is not `Send`, so a `Recorder` lives on
//! the dictation worker thread for its whole lifetime.

use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};

pub const WHISPER_RATE: u32 = 16_000;
/// Hard cap so a stuck key cannot grow memory without bound (~5 min at 48 kHz).
const MAX_SAMPLES: usize = 48_000 * 300;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("audio.noDevice")]
    NoDevice,
    #[error("audio.format")]
    Format,
    #[error("audio.stream")]
    Stream,
}

pub struct Recorder {
    stream: Stream,
    buffer: Arc<Mutex<Vec<f32>>>,
    rate: u32,
}

fn push_mono<T: Copy>(
    buffer: &Mutex<Vec<f32>>,
    data: &[T],
    channels: usize,
    convert: impl Fn(T) -> f32,
) {
    let Ok(mut buf) = buffer.lock() else { return };
    if buf.len() >= MAX_SAMPLES || channels == 0 {
        return;
    }
    for frame in data.chunks(channels) {
        let sum: f32 = frame.iter().map(|s| convert(*s)).sum();
        buf.push(sum / frame.len() as f32);
    }
}

impl Recorder {
    pub fn start() -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or(AudioError::NoDevice)?;
        let supported = device
            .default_input_config()
            .map_err(|_| AudioError::Format)?;
        let format = supported.sample_format();
        let config = supported.config();
        let channels = config.channels as usize;
        let rate = config.sample_rate;
        let buffer = Arc::new(Mutex::new(Vec::with_capacity(rate as usize * 10)));
        let sink = buffer.clone();
        let on_error = |_| {};
        let stream = match format {
            SampleFormat::F32 => device.build_input_stream(
                config,
                move |data: &[f32], _: &_| push_mono(&sink, data, channels, |s| s),
                on_error,
                None,
            ),
            SampleFormat::I16 => device.build_input_stream(
                config,
                move |data: &[i16], _: &_| {
                    push_mono(&sink, data, channels, |s| s as f32 / 32_768.0)
                },
                on_error,
                None,
            ),
            SampleFormat::U16 => device.build_input_stream(
                config,
                move |data: &[u16], _: &_| {
                    push_mono(&sink, data, channels, |s| (s as f32 - 32_768.0) / 32_768.0)
                },
                on_error,
                None,
            ),
            SampleFormat::I32 => device.build_input_stream(
                config,
                move |data: &[i32], _: &_| {
                    push_mono(&sink, data, channels, |s| s as f32 / 2_147_483_648.0)
                },
                on_error,
                None,
            ),
            _ => return Err(AudioError::Format),
        }
        .map_err(|_| AudioError::Stream)?;
        stream.play().map_err(|_| AudioError::Stream)?;
        Ok(Self {
            stream,
            buffer,
            rate,
        })
    }

    /// Stops capture and returns mono samples at 16 kHz.
    pub fn finish(self) -> Vec<f32> {
        let _ = self.stream.pause();
        drop(self.stream);
        let samples = self
            .buffer
            .lock()
            .map(|mut b| std::mem::take(&mut *b))
            .unwrap_or_default();
        resample(&samples, self.rate, WHISPER_RATE)
    }
}

/// Linear resampling. Adequate for speech recognition input.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() || from == 0 || to == 0 {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let out_len = ((input.len() as f64) / ratio).floor() as usize;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let idx = pos.floor() as usize;
            let frac = (pos - idx as f64) as f32;
            let a = input[idx.min(input.len() - 1)];
            let b = input[(idx + 1).min(input.len() - 1)];
            a + (b - a) * frac
        })
        .collect()
}

pub fn duration_seconds(samples: &[f32]) -> f64 {
    samples.len() as f64 / WHISPER_RATE as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_48k_to_16k_keeps_duration() {
        let input: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.01).sin()).collect();
        let out = resample(&input, 48_000, 16_000);
        assert_eq!(out.len(), 16_000);
        assert!((duration_seconds(&out) - 1.0).abs() < 1e-9);
        assert!((out[100] - input[300]).abs() < 1e-6);
    }

    #[test]
    fn resample_44k1_handles_fractional_ratio() {
        let input = vec![0.5f32; 44_100];
        let out = resample(&input, 44_100, 16_000);
        assert_eq!(out.len(), 16_000);
        assert!(out.iter().all(|s| (*s - 0.5).abs() < 1e-6));
    }

    #[test]
    fn stereo_is_downmixed() {
        let buffer = Mutex::new(Vec::new());
        push_mono(&buffer, &[1.0f32, 0.0, 0.5, 0.5], 2, |s| s);
        assert_eq!(*buffer.lock().unwrap(), vec![0.5, 0.5]);
    }
}
