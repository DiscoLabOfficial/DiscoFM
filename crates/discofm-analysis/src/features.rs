use crate::{AnalysisError, MAX_SECONDS, pitch::estimate_pitch, spectrum::spectral_features};
use discofm_audio::AudioBuffer;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ChannelMode {
    #[default]
    Mix,
    Left,
    Right,
}

/// Processing applies to a separate mono analysis buffer, never to the original.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisOptions {
    pub channel: ChannelMode,
    pub remove_dc: bool,
    pub normalize: bool,
    pub trim_silence: bool,
    pub fft_size: usize,
    pub start_seconds: f64,
    pub end_seconds: Option<f64>,
    pub pitch_override_hz: Option<f64>,
}
impl Default for AnalysisOptions {
    fn default() -> Self {
        Self {
            channel: ChannelMode::Mix,
            remove_dc: true,
            normalize: false,
            trim_silence: true,
            fft_size: 2048,
            start_seconds: 0.0,
            end_seconds: None,
            pitch_override_hz: None,
        }
    }
}
impl AnalysisOptions {
    pub fn validate(&self) -> Result<(), AnalysisError> {
        if ![512, 1024, 2048, 4096, 8192].contains(&self.fft_size) {
            return Err(AnalysisError::Settings(
                "FFT size must be 512/1024/2048/4096/8192",
            ));
        }
        if !self.start_seconds.is_finite()
            || self.start_seconds < 0.0
            || self.end_seconds.is_some_and(|end| {
                !end.is_finite()
                    || end <= self.start_seconds
                    || end - self.start_seconds > MAX_SECONDS
            })
            || self
                .pitch_override_hz
                .is_some_and(|hz| !hz.is_finite() || !(20.0..=4000.0).contains(&hz))
        {
            return Err(AnalysisError::Settings(
                "invalid selection or pitch override (20..4000 Hz)",
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct PitchEstimate {
    pub hz: f64,
    pub confidence: f64,
}
impl PitchEstimate {
    pub fn midi(&self) -> f64 {
        69.0 + 12.0 * (self.hz / 440.0).log2()
    }
    pub fn note_name(&self) -> String {
        note_name(self.midi().round() as i32)
    }
    pub fn cents(&self) -> f64 {
        (self.midi() - self.midi().round()) * 100.0
    }
}
pub fn note_name(midi: i32) -> String {
    let names = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!(
        "{}{}",
        names[midi.rem_euclid(12) as usize],
        midi.div_euclid(12) - 1
    )
}
#[derive(Debug, Clone)]
pub struct EnvelopePoint {
    pub seconds: f64,
    pub rms: f32,
    pub peak: f32,
}
#[derive(Debug, Clone)]
pub struct SpectralPoint {
    pub seconds: f64,
    pub centroid_hz: f32,
    pub flux: f32,
}
#[derive(Debug, Clone)]
pub struct Harmonic {
    pub number: usize,
    pub hz: f32,
    pub magnitude: f32,
}
#[derive(Debug, Clone)]
pub struct Spectrum {
    pub frequencies: Vec<f32>,
    pub magnitudes: Vec<f32>,
}
#[derive(Debug, Clone)]
pub struct Spectrogram {
    pub times: Vec<f32>,
    /// Log-spaced frequency rows, low to high; storage is column-major.
    pub frequencies: Vec<f32>,
    pub db: Vec<f32>,
}
#[derive(Debug, Clone)]
pub struct Analysis {
    pub sample_rate: u32,
    pub original_seconds: f64,
    pub region_start_seconds: f64,
    pub region_end_seconds: f64,
    pub dc: [f64; 2],
    pub peak: f32,
    pub rms: f32,
    pub normalization_gain: f32,
    pub clipped_samples: usize,
    pub pitch: Option<PitchEstimate>,
    pub used_pitch_hz: Option<f64>,
    pub envelope: Vec<EnvelopePoint>,
    pub transients: Vec<f64>,
    pub spectral: Vec<SpectralPoint>,
    pub spectrum: Spectrum,
    pub spectrogram: Spectrogram,
    pub harmonics: Vec<Harmonic>,
}
pub(crate) fn validate_audio(audio: &AudioBuffer) -> Result<(), AnalysisError> {
    if audio.sample_rate == 0 || audio.frames.is_empty() {
        return Err(AnalysisError::InvalidAudio);
    }
    Ok(())
}
pub fn analyze(audio: &AudioBuffer, options: &AnalysisOptions) -> Result<Analysis, AnalysisError> {
    analyze_cancellable(audio, options, || false)
}
pub fn analyze_cancellable(
    audio: &AudioBuffer,
    options: &AnalysisOptions,
    cancel: impl Fn() -> bool,
) -> Result<Analysis, AnalysisError> {
    validate_audio(audio)?;
    options.validate()?;
    let rate = f64::from(audio.sample_rate);
    let duration = audio.frames.len() as f64 / rate;
    if options.start_seconds >= duration
        || options
            .end_seconds
            .is_some_and(|end| end > duration + 0.5 / rate)
    {
        return Err(AnalysisError::Settings("selection is outside this audio"));
    }
    let begin = (options.start_seconds * rate).round() as usize;
    let end = (options
        .end_seconds
        .unwrap_or((options.start_seconds + MAX_SECONDS).min(duration))
        * rate)
        .round() as usize;
    let end = end.min(audio.frames.len());
    if end <= begin {
        return Err(AnalysisError::Settings(
            "selection must contain at least one frame",
        ));
    }
    let mut sums = [0.0; 2];
    let mut clipped_samples = 0;
    for (index, frame) in audio.frames.iter().enumerate() {
        if index.is_multiple_of(16384) && cancel() {
            return Err(AnalysisError::Cancelled);
        }
        for channel in 0..2 {
            if !frame[channel].is_finite() {
                return Err(AnalysisError::InvalidAudio);
            }
            sums[channel] += f64::from(frame[channel]);
            clipped_samples += usize::from(frame[channel].abs() >= 1.0);
        }
    }
    let dc = sums.map(|sum| sum / audio.frames.len() as f64);
    let mut mono = Vec::with_capacity(end - begin);
    for (index, frame) in audio.frames[begin..end].iter().enumerate() {
        if index.is_multiple_of(16384) && cancel() {
            return Err(AnalysisError::Cancelled);
        }
        if frame.iter().any(|v| !v.is_finite()) {
            return Err(AnalysisError::InvalidAudio);
        }
        let value = match options.channel {
            ChannelMode::Mix => (f64::from(frame[0]) + f64::from(frame[1])) * 0.5,
            ChannelMode::Left => f64::from(frame[0]),
            ChannelMode::Right => f64::from(frame[1]),
        };
        mono.push(value as f32);
    }
    if options.remove_dc {
        let selected_dc = mono.iter().map(|v| f64::from(*v)).sum::<f64>() / mono.len() as f64;
        for value in &mut mono {
            *value = (f64::from(*value) - selected_dc) as f32;
        }
    }
    let peak = mono.iter().map(|v| v.abs()).fold(0.0_f32, f32::max);
    if !peak.is_finite() || peak > 1e10 {
        return Err(AnalysisError::InvalidAudio);
    }
    let rms = (mono.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / mono.len().max(1) as f64)
        .sqrt() as f32;
    let gain = if options.normalize && peak > 1e-12 {
        1.0 / peak
    } else {
        1.0
    };
    let threshold = (peak * 0.001).max(1e-7);
    let first = if options.trim_silence {
        mono.iter().position(|v| v.abs() > threshold).unwrap_or(0)
    } else {
        0
    };
    let last = if options.trim_silence {
        mono.iter()
            .rposition(|v| v.abs() > threshold)
            .map_or(mono.len(), |i| i + 1)
    } else {
        mono.len()
    };
    let offset = (begin + first) as f64 / rate;
    let region_end = (begin + last) as f64 / rate;
    let mono = &mut mono[first..last];
    for value in mono.iter_mut() {
        *value *= gain;
    }
    let hop = (audio.sample_rate as usize / 200).max(1);
    let window = (audio.sample_rate as usize / 100).max(1);
    let mut envelope = Vec::new();
    for index in (0..mono.len()).step_by(hop) {
        let chunk = &mono[index..(index + window).min(mono.len())];
        let rms = (chunk.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / chunk.len() as f64)
            .sqrt() as f32;
        envelope.push(EnvelopePoint {
            seconds: offset + index as f64 / rate,
            rms,
            peak: chunk.iter().map(|v| v.abs()).fold(0.0, f32::max),
        });
    }
    let pitch = estimate_pitch(mono, audio.sample_rate, &cancel)?;
    let used_pitch_hz = options
        .pitch_override_hz
        .or_else(|| pitch.as_ref().map(|p| p.hz));
    let (spectrum, spectrogram, spectral) =
        spectral_features(mono, audio.sample_rate, options.fft_size, offset, &cancel)?;
    let mut transients = Vec::new();
    let max_rms = envelope.iter().map(|p| p.rms).fold(0.0_f32, f32::max);
    let mut previous = 0.0;
    for point in &envelope {
        if point.rms > max_rms * 0.1
            && point.rms > previous * 1.7 + max_rms * 0.04
            && transients
                .last()
                .is_none_or(|last| point.seconds - last > 0.05)
        {
            transients.push(point.seconds);
        }
        previous = point.rms;
    }
    let flux_mean = spectral.iter().map(|p| p.flux).sum::<f32>() / spectral.len().max(1) as f32;
    for point in &spectral {
        if point.flux > (flux_mean * 3.0).max(0.2)
            && transients
                .iter()
                .all(|time| (point.seconds - time).abs() > 0.05)
        {
            transients.push(point.seconds);
        }
    }
    transients.sort_by(f64::total_cmp);
    let mut harmonics = Vec::new();
    if let Some(f0) = used_pitch_hz {
        let bin = rate / options.fft_size as f64;
        for number in 1..=32 {
            let hz = f0 * number as f64;
            if hz >= rate * 0.5 {
                break;
            }
            let center = (hz / bin).round() as usize;
            let from = center.saturating_sub(1);
            let to = (center + 2).min(spectrum.magnitudes.len());
            harmonics.push(Harmonic {
                number,
                hz: hz as f32,
                magnitude: spectrum.magnitudes[from..to]
                    .iter()
                    .copied()
                    .fold(0.0, f32::max),
            });
        }
    }
    Ok(Analysis {
        sample_rate: audio.sample_rate,
        original_seconds: duration,
        region_start_seconds: offset,
        region_end_seconds: region_end,
        dc,
        peak,
        rms,
        normalization_gain: gain,
        clipped_samples,
        pitch,
        used_pitch_hz,
        envelope,
        transients,
        spectral,
        spectrum,
        spectrogram,
        harmonics,
    })
}
