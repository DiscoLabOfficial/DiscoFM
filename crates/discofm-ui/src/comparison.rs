//! One interval, channel, sample rate and gain for comparison plots and A/B.
use discofm_analysis::{Analysis, AnalysisOptions, ChannelMode, analyze_cancellable};
use discofm_audio::{AudioBuffer, StreamingResampler};
use std::sync::Arc;

pub(crate) struct Comparison {
    pub reference: Arc<AudioBuffer>,
    pub generated: Arc<AudioBuffer>,
    pub analysis: Analysis,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub generated_gain: f64,
    pub playback_gain: f64,
}
fn sample(frame: [f32; 2], channel: ChannelMode) -> f64 {
    match channel {
        ChannelMode::Mix => (f64::from(frame[0]) + f64::from(frame[1])) * 0.5,
        ChannelMode::Left => f64::from(frame[0]),
        ChannelMode::Right => f64::from(frame[1]),
    }
}
fn condition(frames: &mut [[f32; 2]], remove_dc: bool, gain: f64) {
    let mean = if remove_dc {
        frames.iter().map(|f| f64::from(f[0])).sum::<f64>() / frames.len().max(1) as f64
    } else {
        0.0
    };
    for frame in frames {
        *frame = [((f64::from(frame[0]) - mean) * gain) as f32; 2];
    }
}
fn rms(audio: &AudioBuffer) -> f64 {
    (audio
        .frames
        .iter()
        .map(|f| f64::from(f[0]).powi(2))
        .sum::<f64>()
        / audio.frames.len().max(1) as f64)
        .sqrt()
}
pub(crate) fn build(
    source: &AudioBuffer,
    generated: &AudioBuffer,
    options: &AnalysisOptions,
    analysis: &Analysis,
    note_on: f64,
    match_level: bool,
    cancel: &impl Fn() -> bool,
) -> Result<(Comparison, Analysis), String> {
    let start_seconds = options.start_seconds.max(note_on);
    let end_seconds = options
        .end_seconds
        .unwrap_or((options.start_seconds + 30.0).min(analysis.original_seconds));
    let rate = f64::from(source.sample_rate);
    let from = (start_seconds * rate).round() as usize;
    let to = ((end_seconds * rate).round() as usize).min(source.frames.len());
    if from >= to {
        return Err("Comparison needs the note-on marker inside the selected region".into());
    }
    let mut converter = StreamingResampler::new(rate, 48000).map_err(|e| e.to_string())?;
    let mut frames = Vec::new();
    let mut output = [[0.0; 2]; 512];
    for block in source.frames[from..to].chunks(512) {
        if cancel() {
            return Err("cancelled".into());
        }
        let mono = block
            .iter()
            .map(|f| [sample(*f, options.channel) as f32; 2])
            .collect::<Vec<_>>();
        if source.sample_rate == 48000 {
            frames.extend_from_slice(&mono);
            continue;
        }
        converter.push(&mono).map_err(|e| e.to_string())?;
        loop {
            let n = converter.read_into(&mut output);
            frames.extend_from_slice(&output[..n]);
            if n == 0 {
                break;
            }
        }
    }
    converter.finish();
    if source.sample_rate != 48000 {
        loop {
            if cancel() {
                return Err("cancelled".into());
            }
            let n = converter.read_into(&mut output);
            frames.extend_from_slice(&output[..n]);
            if n == 0 {
                break;
            }
        }
    }
    let shared_gain = f64::from(analysis.normalization_gain);
    condition(&mut frames, options.remove_dc, shared_gain);
    let reference = Arc::new(AudioBuffer {
        sample_rate: 48000,
        frames,
    });
    let crop = ((start_seconds - note_on).max(0.0) * 48000.0).round() as usize;
    let available = generated
        .frames
        .len()
        .saturating_sub(crop)
        .min(reference.frames.len());
    let mut frames = (0..available)
        .map(|i| {
            [generated
                .frames
                .get(crop + i)
                .map_or(0.0, |f| sample(*f, options.channel)) as f32; 2]
        })
        .collect::<Vec<_>>();
    condition(&mut frames, options.remove_dc, shared_gain);
    frames.resize(reference.frames.len(), [0.0; 2]);
    let mut generated = AudioBuffer {
        sample_rate: 48000,
        frames,
    };
    let level = rms(&generated);
    let generated_gain = if match_level && level > 1e-12 {
        (rms(&reference) / level).clamp(0.0001, 10000.0)
    } else {
        1.0
    };
    for block in generated.frames.chunks_mut(4096) {
        if cancel() {
            return Err("cancelled".into());
        }
        for frame in block {
            for value in frame {
                *value *= generated_gain as f32;
            }
        }
    }
    let peak = reference
        .frames
        .iter()
        .chain(&generated.frames)
        .flat_map(|f| f.iter())
        .map(|v| f64::from(v.abs()))
        .fold(0.0, f64::max);
    let playback_gain = if peak > 0.98 { 0.98 / peak } else { 1.0 };
    let prepared = AnalysisOptions {
        remove_dc: false,
        normalize: false,
        trim_silence: false,
        fft_size: options.fft_size,
        pitch_override_hz: analysis.used_pitch_hz,
        ..Default::default()
    };
    let analysis = analyze_cancellable(&reference, &prepared, cancel).map_err(|e| e.to_string())?;
    let generated_analysis =
        analyze_cancellable(&generated, &prepared, cancel).map_err(|e| e.to_string())?;
    Ok((
        Comparison {
            reference,
            generated: Arc::new(generated),
            analysis,
            start_seconds,
            end_seconds,
            generated_gain: generated_gain * shared_gain,
            playback_gain,
        },
        generated_analysis,
    ))
}

impl Comparison {
    pub fn clip(&self, reference: bool) -> Arc<AudioBuffer> {
        let mut audio = if reference {
            Arc::clone(&self.reference)
        } else {
            Arc::clone(&self.generated)
        };
        if self.playback_gain != 1.0 {
            for frame in &mut Arc::make_mut(&mut audio).frames {
                for value in frame {
                    *value *= self.playback_gain as f32;
                }
            }
        }
        audio
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prepared_ab_shares_interval_gain_and_headroom_without_erasing_missing_tail() {
        let tone = |i: usize| (i as f32 * std::f32::consts::TAU * 250.0 / 48000.0).sin();
        let source = AudioBuffer {
            sample_rate: 48000,
            frames: (0..24000)
                .map(|i| [tone(i) * 0.8 + 0.1, -tone(i) * 0.8])
                .collect(),
        };
        let chip = AudioBuffer {
            sample_rate: 48000,
            frames: (0..24000).map(|i| [tone(i) * 0.08 + 0.01; 2]).collect(),
        };
        let options = AnalysisOptions {
            channel: ChannelMode::Left,
            start_seconds: 0.1,
            end_seconds: Some(0.4),
            normalize: true,
            trim_silence: true,
            fft_size: 8192,
            pitch_override_hz: Some(250.0),
            ..Default::default()
        };
        let original = source.clone();
        let analysis = analyze_cancellable(&source, &options, || false).unwrap();
        let (ab, b) = build(&source, &chip, &options, &analysis, 0.0, true, &|| false).unwrap();
        assert_eq!(source, original);
        assert_eq!(ab.reference.frames.len(), 14400);
        assert_eq!(ab.reference.frames.len(), ab.generated.frames.len());
        assert!((rms(&ab.reference) - rms(&ab.generated)).abs() < 1e-6);
        assert!((ab.generated_gain - 10.0 * f64::from(analysis.normalization_gain)).abs() < 1e-4);
        assert_eq!(ab.analysis.spectrum.frequencies, b.spectrum.frequencies);
        for (a, b) in ab.reference.frames.iter().zip(&ab.generated.frames) {
            assert!((a[0] - b[0]).abs() < 1e-5);
        }
        assert!(
            ab.clip(false)
                .frames
                .iter()
                .flatten()
                .all(|v| v.abs() <= 0.980001)
        );
        let short = AudioBuffer {
            frames: chip.frames[..9600].to_vec(),
            ..chip.clone()
        };
        let (ab, _) = build(&source, &short, &options, &analysis, 0.0, true, &|| false).unwrap();
        assert!(ab.generated.frames[5000..].iter().all(|f| *f == [0.0; 2]));
        assert!(ab.generated.frames[1000][0].abs() > ab.generated.frames[6000][0].abs());
        let (raw, _) = build(&source, &chip, &options, &analysis, 0.0, false, &|| false).unwrap();
        assert!((rms(&raw.generated) / rms(&raw.reference) - 0.1).abs() < 1e-6);
    }
}
