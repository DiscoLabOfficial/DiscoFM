//! Stereo conversion, diagnostic WAVs and buffered CPAL device playback.
#![forbid(unsafe_code)]
mod playback;
mod streaming;
use discofm_engine::NativeAudio;
pub use playback::{
    AuditionControls, DeviceInfo, DeviceOptions, OutputInfo, Playback, PlaybackError,
    PlaybackStats, VoiceId, octave_note, output_devices,
};
use std::{fs::File, path::Path};
pub use streaming::StreamingResampler;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("source and target sample rates must be finite and within 8000..192000 Hz")]
    SampleRate,
    #[error("audio contains non-finite values or exceeds the 30 second offline limit")]
    InvalidBuffer,
    #[error("streaming resampler input is full or already finished")]
    StreamBuffer,
    #[error(transparent)]
    Wav(#[from] hound::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioBuffer {
    pub frames: Vec<[f32; 2]>,
    pub sample_rate: u32,
}

/// Blackman-windowed sinc with support equivalent to 64 taps at the lower rate.
/// Source-domain support grows when downsampling, preserving stopband rejection.
/// Zero time is preserved; boundary taps are extended using the endpoint value.
/// Fixed gain, no normalization, dither, analog filtering or DC removal.
pub fn resample(
    frames: &[[f32; 2]],
    source_hz: f64,
    target_hz: u32,
) -> Result<AudioBuffer, AudioError> {
    if !source_hz.is_finite()
        || !(8000.0..=192000.0).contains(&source_hz)
        || !(8000..=192000).contains(&target_hz)
    {
        return Err(AudioError::SampleRate);
    }
    if frames.len() as f64 / source_hz > 30.01 || frames.iter().flatten().any(|v| !v.is_finite()) {
        return Err(AudioError::InvalidBuffer);
    }
    if frames.is_empty() {
        return Ok(AudioBuffer {
            frames: Vec::new(),
            sample_rate: target_hz,
        });
    }
    let count = (frames.len() as f64 * f64::from(target_hz) / source_hz).round() as usize;
    let kernel = streaming::Kernel::new(source_hz, target_hz);
    let mut result = Vec::with_capacity(count);
    for index in 0..count {
        let position = index as f64 * source_hz / f64::from(target_hz);
        result.push(kernel.sample(position, |index| {
            frames[index.clamp(0, frames.len() as i64 - 1) as usize]
        }));
    }
    Ok(AudioBuffer {
        frames: result,
        sample_rate: target_hz,
    })
}

/// Single-channel `resample`: identical kernel, length and endpoint handling,
/// at half the arithmetic and without per-tap stereo frame lookups.
pub fn resample_mono(
    samples: &[f32],
    source_hz: f64,
    target_hz: u32,
) -> Result<Vec<f32>, AudioError> {
    if !source_hz.is_finite()
        || !(8000.0..=192000.0).contains(&source_hz)
        || !(8000..=192000).contains(&target_hz)
    {
        return Err(AudioError::SampleRate);
    }
    if samples.len() as f64 / source_hz > 30.01 || samples.iter().any(|v| !v.is_finite()) {
        return Err(AudioError::InvalidBuffer);
    }
    if samples.is_empty() {
        return Ok(Vec::new());
    }
    let count = (samples.len() as f64 * f64::from(target_hz) / source_hz).round() as usize;
    let kernel = streaming::Kernel::new(source_hz, target_hz);
    Ok((0..count)
        .map(|index| kernel.sample_mono(index as f64 * source_hz / f64::from(target_hz), samples))
        .collect())
}

pub fn from_native(audio: &NativeAudio, target_hz: u32) -> Result<AudioBuffer, AudioError> {
    resample(&audio.frames, audio.sample_rate, target_hz)
}

/// IEEE float WAV preserves the exact audio gain without silent clipping.
/// Requires a new destination; never truncates existing files or linked files.
pub fn write_wav(path: impl AsRef<Path>, audio: &AudioBuffer) -> Result<(), AudioError> {
    if !(8000..=192000).contains(&audio.sample_rate) {
        return Err(AudioError::SampleRate);
    }
    if audio.frames.iter().flatten().any(|v| !v.is_finite()) {
        return Err(AudioError::InvalidBuffer);
    }
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: audio.sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::new(File::create_new(path)?, spec)?;
    for frame in &audio.frames {
        writer.write_sample(frame[0])?;
        writer.write_sample(frame[1])?;
    }
    writer.finalize()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;
    fn rms(frames: &[[f32; 2]]) -> f64 {
        (frames.iter().map(|p| f64::from(p[0]).powi(2)).sum::<f64>() / frames.len() as f64).sqrt()
    }
    #[test]
    fn boundary_lengths_dc_gain_and_empty_buffers() {
        for rate in [8000, 44100, 48000, 96000, 192000] {
            let out = resample(&[[0.25, -0.25]; 5327], 53270.0, rate).unwrap();
            assert_eq!(out.frames.len(), (f64::from(rate) * 0.1).round() as usize);
            assert!(
                out.frames
                    .iter()
                    .all(|p| (p[0] - 0.25).abs() < 1e-6 && (p[1] + 0.25).abs() < 1e-6)
            );
        }
        assert!(resample(&[], 53270.0, 48000).unwrap().frames.is_empty());
        assert!(resample(&[[f32::NAN, 0.0]], 53270.0, 48000).is_err());
        assert!(resample(&[], 0.0, 48000).is_err());
    }
    #[test]
    fn mono_resampling_is_bit_identical_to_stereo_channel_zero() {
        let signal: Vec<f32> = (0..9000)
            .map(|i| ((i as f64 * 0.0371).sin() * 0.6 + if i == 3 { 0.9 } else { 0.0 }) as f32)
            .collect();
        let stereo: Vec<_> = signal.iter().map(|v| [*v, -*v]).collect();
        // Polyphase table and analytic (support > 512) kernels, up and down.
        for (source, target) in [
            (53267.03869047619, 24000),
            (44100.0, 48000),
            (192000.0, 8000),
        ] {
            let expected = resample(&stereo, source, target).unwrap().frames;
            let actual = resample_mono(&signal, source, target).unwrap();
            assert_eq!(actual.len(), expected.len());
            assert!(
                actual
                    .iter()
                    .zip(&expected)
                    .all(|(a, e)| a.to_bits() == e[0].to_bits()),
                "{source}->{target}"
            );
        }
        assert!(resample_mono(&[], 53270.0, 24000).unwrap().is_empty());
        assert!(resample_mono(&[f32::NAN], 53270.0, 24000).is_err());
    }
    #[test]
    fn rejects_out_of_band_energy_when_downsampling() {
        let wave = |hz: f64| {
            (0..5327)
                .map(|i| {
                    let value = (2.0 * PI * hz * i as f64 / 53270.0).sin() as f32;
                    [value, value]
                })
                .collect::<Vec<_>>()
        };
        let low = resample(&wave(440.0), 53270.0, 16000).unwrap();
        let high = resample(&wave(18000.0), 53270.0, 16000).unwrap();
        assert!(rms(&low.frames[64..1536]) > 0.69);
        assert!(rms(&high.frames[64..1536]) < 0.001);
        for frequency in [4080.0, 4200.0, 6000.0] {
            let out = resample(&wave(frequency), 53270.0, 8000).unwrap();
            assert!(
                rms(&out.frames[64..736]) < 0.001,
                "out-of-band input {frequency} Hz must be suppressed below -57 dBFS RMS"
            );
        }
    }
}
