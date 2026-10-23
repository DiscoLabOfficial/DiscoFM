use discofm_audio::{AudioBuffer, AudioError, from_native, resample};
use discofm_chip::Instrument;
use discofm_engine::{RenderRequest, render_note_cancellable, render_ymfm_reference};
use rustfft::{FftPlanner, num_complex::Complex};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct AudioComparison {
    pub sample_rate: u32,
    pub frames: usize,
    /// Positive means the actual signal lags the reference.
    pub alignment_samples: i32,
    pub alignment_seconds: f64,
    pub recording_gain: f64,
    pub rms_actual: f64,
    pub rms_reference: f64,
    pub relative_waveform_error: f64,
    pub spectral_cosine_low: f64,
    pub spectral_cosine_full: f64,
    pub envelope_relative_error: f64,
    pub dominant_actual_hz: f64,
    pub dominant_reference_hz: f64,
}
fn mean_removed(audio: &AudioBuffer) -> Vec<f64> {
    let mut signal = audio
        .frames
        .iter()
        .map(|f| f64::from(f[0] + f[1]) * 0.5)
        .collect::<Vec<_>>();
    let mean = signal.iter().sum::<f64>() / signal.len().max(1) as f64;
    for v in &mut signal {
        *v -= mean;
    }
    signal
}
fn rms(signal: &[f64]) -> f64 {
    (signal.iter().map(|v| v * v).sum::<f64>() / signal.len().max(1) as f64).sqrt()
}
fn aligned<'a>(a: &'a [f64], b: &'a [f64], lag: i32) -> (&'a [f64], &'a [f64]) {
    let a_start = lag.max(0) as usize;
    let b_start = (-lag).max(0) as usize;
    let length = (a.len() - a_start).min(b.len() - b_start);
    (&a[a_start..a_start + length], &b[b_start..b_start + length])
}
fn magnitude(signal: &[f64], size: usize) -> Vec<f64> {
    let mut data = vec![Complex::default(); size];
    for (i, v) in signal.iter().enumerate() {
        data[i].re =
            *v * (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / signal.len() as f64).cos());
    }
    FftPlanner::new().plan_fft_forward(size).process(&mut data);
    data[..=size / 2].iter().map(|v| v.norm()).collect()
}
fn cosine(a: &[f64], b: &[f64]) -> f64 {
    let dot = a.iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
    let norm = (a.iter().map(|v| v * v).sum::<f64>() * b.iter().map(|v| v * v).sum::<f64>()).sqrt();
    if norm < 1e-20 {
        if a == b { 1.0 } else { 0.0 }
    } else {
        (dot / norm).clamp(0.0, 1.0)
    }
}
fn envelope(signal: &[f64]) -> Vec<f64> {
    // 40 ms windows reduce carrier-phase ripples in low bass, 5 ms hop.
    signal
        .chunks(120)
        .enumerate()
        .map(|(i, _)| rms(&signal[i * 120..(i * 120 + 960).min(signal.len())]))
        .collect()
}
/// Compare an independently recorded note against a render. The caller selects
/// its region; alignment is explicitly bounded to 20 ms and gain is optional.
/// Whole-note FFT/cosine and 40 ms envelopes are independent of the fit STFT.
pub fn compare_audio(
    actual: &AudioBuffer,
    reference: &AudioBuffer,
    max_alignment_seconds: f64,
    match_level: bool,
) -> Result<AudioComparison, AudioError> {
    if !max_alignment_seconds.is_finite()
        || !(0.0..=0.02).contains(&max_alignment_seconds)
        || actual.frames.is_empty()
        || reference.frames.is_empty()
    {
        return Err(AudioError::InvalidBuffer);
    }
    let a = mean_removed(&resample(&actual.frames, actual.sample_rate.into(), 24000)?);
    let b = mean_removed(&resample(
        &reference.frames,
        reference.sample_rate.into(),
        24000,
    )?);
    if a.len().min(b.len()) < 600 {
        return Err(AudioError::InvalidBuffer);
    }
    let maximum = ((max_alignment_seconds * 24000.0).round() as i32)
        .min(a.len().min(b.len()).saturating_sub(2) as i32);
    let correlation = |lag| {
        let (a, b) = aligned(&a, &b, lag);
        let (dot, aa, bb) = a
            .iter()
            .zip(b)
            .step_by(4)
            .fold((0.0, 0.0, 0.0), |(dot, aa, bb), (a, b)| {
                (dot + a * b, aa + a * a, bb + b * b)
            });
        dot / (aa * bb).sqrt().max(1e-20)
    };
    let coarse = (-maximum..=maximum)
        .step_by(4)
        .max_by(|a, b| correlation(*a).total_cmp(&correlation(*b)))
        .unwrap_or(0);
    let lag = ((coarse - 3).max(-maximum)..=(coarse + 3).min(maximum))
        .max_by(|a, b| correlation(*a).total_cmp(&correlation(*b)))
        .unwrap_or(0);
    let (a, b) = aligned(&a, &b, lag);
    // Recenter the overlap: removed recording padding must not bias metrics.
    let center = |s: &[f64]| {
        let mean = s.iter().sum::<f64>() / s.len() as f64;
        s.iter().map(|v| v - mean).collect::<Vec<_>>()
    };
    let a = center(a);
    let b = center(b);
    let ra = rms(&a);
    let rb = rms(&b);
    let gain = if match_level && ra > 1e-12 {
        (rb / ra).clamp(0.0001, 10000.0)
    } else {
        1.0
    };
    let scaled = a.iter().map(|v| v * gain).collect::<Vec<_>>();
    let size = (a.len() * 4).next_power_of_two();
    let sa = magnitude(&scaled, size);
    let sb = magnitude(&b, size);
    let from = (30.0 * size as f64 / 24000.0).ceil() as usize;
    let low = (1000.0 * size as f64 / 24000.0).floor() as usize;
    let full = (10000.0 * size as f64 / 24000.0).floor() as usize;
    let peak = |s: &[f64]| {
        let index = (from.max(1)..full.min(s.len() - 1))
            .max_by(|a, b| s[*a].total_cmp(&s[*b]))
            .unwrap_or(1);
        let denominator = s[index - 1] - 2.0 * s[index] + s[index + 1];
        let offset = if denominator.abs() > 1e-20 {
            (0.5 * (s[index - 1] - s[index + 1]) / denominator).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        (index as f64 + offset) * 24000.0 / size as f64
    };
    let ea = envelope(&scaled);
    let eb = envelope(&b);
    let error = |a: &[f64], b: &[f64]| {
        (a.iter().zip(b).map(|(a, b)| (a - b).powi(2)).sum::<f64>()
            / b.iter().map(|v| v * v).sum::<f64>().max(1e-20))
        .sqrt()
    };
    Ok(AudioComparison {
        sample_rate: 24000,
        frames: a.len(),
        alignment_samples: lag,
        alignment_seconds: lag as f64 / 24000.0,
        recording_gain: gain,
        rms_actual: ra,
        rms_reference: rb,
        relative_waveform_error: error(&scaled, &b),
        spectral_cosine_low: cosine(&sa[from..=low], &sb[from..=low]),
        spectral_cosine_full: cosine(&sa[from..=full], &sb[from..=full]),
        envelope_relative_error: error(&ea, &eb),
        dominant_actual_hz: peak(&sa),
        dominant_reference_hz: peak(&sb),
    })
}
pub fn compare_ymfm(
    patch: &Instrument,
    request: &RenderRequest,
) -> Result<AudioComparison, Box<dyn std::error::Error>> {
    let nuked = from_native(&render_note_cancellable(patch, request, || false)?, 24000)?;
    let ymfm = from_native(&render_ymfm_reference(patch, request)?, 24000)?;
    Ok(compare_audio(
        &nuked,
        &ymfm,
        2.0 / request.clock.native_hz(),
        false,
    )?)
}
