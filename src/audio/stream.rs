//! Decoding a [`Song`] into device-ready PCM for the micro:bit v2 speaker.
//!
//! The v2 plays raw 8-bit unsigned PCM (128 = silence), mono, at a low sample
//! rate (~7.8 kHz), so [`Song::stream`] downmixes, resamples, and quantises the
//! WAV to that form. The bytes are then sent verbatim over serial (see
//! [`crate::app`]).

use anyhow::{Context, Result};

use super::Song;

/// Device-ready audio decoded from a song: 8-bit unsigned mono PCM at `rate` Hz.
pub struct SongStream {
    pub rate: u32,
    pub samples: Vec<u8>,
}

impl Song {
    /// Decode this song's WAV into mono 8-bit unsigned PCM at `rate` Hz.
    pub fn stream(&self, rate: u32) -> Result<SongStream> {
        let mut reader = hound::WavReader::open(&self.path)
            .with_context(|| format!("opening WAV file {}", self.path.display()))?;
        let spec = reader.spec();
        let channels = spec.channels.max(1) as usize;

        // Normalise every sample to f32 in [-1.0, 1.0], whatever the source format.
        let interleaved: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader
                .samples::<f32>()
                .collect::<Result<_, _>>()
                .context("reading float WAV samples")?,
            hound::SampleFormat::Int => {
                let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
                reader
                    .samples::<i32>()
                    .map(|s| s.map(|v| v as f32 / scale))
                    .collect::<Result<_, _>>()
                    .context("reading integer WAV samples")?
            }
        };

        let mono = downmix(&interleaved, channels);
        let resampled = resample(&mono, spec.sample_rate, rate);
        Ok(SongStream {
            rate,
            samples: to_u8(&resampled),
        })
    }
}

/// Average interleaved channels down to a single mono track.
fn downmix(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Linear-interpolating resampler from `src_rate` to `dst_rate`.
fn resample(input: &[f32], src_rate: u32, dst_rate: u32) -> Vec<f32> {
    if input.is_empty() || src_rate == dst_rate {
        return input.to_vec();
    }
    let ratio = dst_rate as f64 / src_rate as f64;
    let out_len = ((input.len() as f64) * ratio).round() as usize;
    let last = input.len() - 1;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 / ratio;
            let idx = pos.floor() as usize;
            let frac = (pos - idx as f64) as f32;
            let a = input[idx.min(last)];
            let b = input[(idx + 1).min(last)];
            a + (b - a) * frac
        })
        .collect()
}

/// Quantise [-1.0, 1.0] f32 samples to 8-bit unsigned (128 = silence).
fn to_u8(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .map(|&s| {
            (s.clamp(-1.0, 1.0) * 127.0 + 128.0)
                .round()
                .clamp(0.0, 255.0) as u8
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn silence_maps_to_midpoint() {
        assert_eq!(to_u8(&[0.0]), [128]);
    }

    #[test]
    fn extremes_clamp_to_full_range() {
        assert_eq!(to_u8(&[-1.0, 1.0, -2.0, 2.0]), [1, 255, 1, 255]);
    }

    #[test]
    fn downmix_averages_channels() {
        // Two stereo frames: (0.0,1.0) -> 0.5, (-1.0,1.0) -> 0.0
        assert_eq!(downmix(&[0.0, 1.0, -1.0, 1.0], 2), [0.5, 0.0]);
    }

    #[test]
    fn resample_is_identity_at_same_rate() {
        let input = [0.1, 0.2, 0.3];
        assert_eq!(resample(&input, 8000, 8000), input);
    }

    #[test]
    fn resample_halving_rate_roughly_halves_length() {
        let input: Vec<f32> = (0..100).map(|i| i as f32 / 100.0).collect();
        let out = resample(&input, 16000, 8000);
        assert_eq!(out.len(), 50);
    }

    #[test]
    fn song_stream_decodes_resamples_and_quantises() {
        // Write a tiny 16-bit mono WAV at 16 kHz, then decode it at 8 kHz.
        let path = std::env::temp_dir().join(format!("mb_pcm_{}.wav", std::process::id()));
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        {
            let mut writer = hound::WavWriter::create(&path, spec).unwrap();
            for _ in 0..8 {
                writer.write_sample(0i16).unwrap(); // silence
            }
            writer.finalize().unwrap();
        }

        let song = Song {
            name: "silence".to_string(),
            path: PathBuf::from(&path),
        };
        let stream = song.stream(8_000).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(stream.rate, 8_000);
        assert_eq!(stream.samples.len(), 4); // 8 samples halved by the 2:1 resample
        assert!(stream.samples.iter().all(|&b| b == 128)); // silence -> midpoint
    }
}
