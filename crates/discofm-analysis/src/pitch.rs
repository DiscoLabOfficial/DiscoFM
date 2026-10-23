use crate::{AnalysisError, PitchEstimate};
use rustfft::{FftPlanner, num_complex::Complex};

// YIN supplies the octave. A nearby resolved fundamental supplies fine tuning;
// do not replace a weak/missing fundamental with a stronger harmonic.
fn refine_fundamental(
    samples: &[f64],
    estimate: f64,
    cancel: &impl Fn() -> bool,
) -> Result<f64, AnalysisError> {
    if estimate * samples.len() as f64 / 24000.0 < 4.0 {
        return Ok(estimate);
    }
    if cancel() {
        return Err(AnalysisError::Cancelled);
    }
    let size = (samples.len() * 8).next_power_of_two();
    let mut buffer = vec![Complex::default(); size];
    for (i, sample) in samples.iter().enumerate() {
        buffer[i].re =
            sample * (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / samples.len() as f64).cos());
    }
    FftPlanner::<f64>::new()
        .plan_fft_forward(size)
        .process(&mut buffer);
    if cancel() {
        return Err(AnalysisError::Cancelled);
    }
    let bin_hz = 24000.0 / size as f64;
    let ratio = 2.0_f64.powf(25.0 / 1200.0);
    let from = ((estimate / ratio / bin_hz).floor() as usize)
        .saturating_sub(1)
        .max(1);
    let to = ((estimate * ratio / bin_hz).ceil() as usize + 1).min(size / 2 - 1);
    let Some(peak) =
        (from..=to).max_by(|a, b| buffer[*a].norm_sqr().total_cmp(&buffer[*b].norm_sqr()))
    else {
        return Ok(estimate);
    };
    let power = buffer[peak].norm_sqr();
    let strongest = buffer[1..=size / 2]
        .iter()
        .map(|v| v.norm_sqr())
        .fold(0.0, f64::max);
    if power < strongest * 0.0001
        || power <= buffer[peak - 1].norm_sqr()
        || power <= buffer[peak + 1].norm_sqr()
    {
        return Ok(estimate);
    }
    let left = buffer[peak - 1].norm_sqr().max(1e-30).ln();
    let center = power.max(1e-30).ln();
    let right = buffer[peak + 1].norm_sqr().max(1e-30).ln();
    let denominator = left - 2.0 * center + right;
    let fraction = if denominator.abs() > 1e-12 {
        ((left - right) / (2.0 * denominator)).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    let refined = (peak as f64 + fraction) * bin_hz;
    Ok(
        if (20.0..=4000.0).contains(&refined) && (refined / estimate).log2().abs() * 1200.0 <= 25.0
        {
            refined
        } else {
            estimate
        },
    )
}

/// YIN cumulative-mean normalized difference, with parabolic lag interpolation.
/// Bounded 24 kHz analysis avoids a sample-rate-dependent quadratic cost.
pub(crate) fn estimate_pitch(
    mono: &[f32],
    rate: u32,
    cancel: &impl Fn() -> bool,
) -> Result<Option<PitchEstimate>, AnalysisError> {
    if rate < 40 || mono.len() < rate as usize / 40 {
        return Ok(None);
    }
    // Choose the strongest 0.4-second region, avoiding long leading silence.
    let length = (rate as usize * 2 / 5).min(mono.len());
    let hop = (rate as usize / 20).max(1);
    let mut best = 0;
    let mut energy = 0.0_f64;
    for start in (0..=mono.len() - length).step_by(hop) {
        if cancel() {
            return Err(AnalysisError::Cancelled);
        }
        let sum = mono[start..start + length]
            .iter()
            .map(|v| f64::from(*v).powi(2))
            .sum::<f64>();
        if sum > energy {
            energy = sum;
            best = start;
        }
    }
    if energy / (length as f64) < 1e-12 {
        return Ok(None);
    }
    let source = mono[best..best + length]
        .iter()
        .map(|v| [*v; 2])
        .collect::<Vec<_>>();
    let mut converter = discofm_audio::StreamingResampler::new(f64::from(rate), 24000)?;
    let mut samples =
        Vec::with_capacity((source.len() as f64 * 24000.0 / f64::from(rate)).round() as usize);
    let mut output = [[0.0; 2]; 512];
    for block in source.chunks(512) {
        if cancel() {
            return Err(AnalysisError::Cancelled);
        }
        converter.push(block)?;
        loop {
            let count = converter.read_into(&mut output);
            samples.extend(output[..count].iter().map(|f| f64::from(f[0])));
            if count == 0 {
                break;
            }
        }
    }
    converter.finish();
    loop {
        if cancel() {
            return Err(AnalysisError::Cancelled);
        }
        let count = converter.read_into(&mut output);
        samples.extend(output[..count].iter().map(|f| f64::from(f[0])));
        if count == 0 {
            break;
        }
    }
    let maximum = 1202.min(samples.len() / 3);
    let minimum = 6;
    if maximum <= minimum + 1 {
        return Ok(None);
    }
    let count = samples.len() - maximum;
    let mut difference = vec![0.0; maximum + 1];
    let mut raw = vec![0.0; maximum + 1];
    let mut selected = None;
    let mut running = 0.0;
    for lag in 1..=maximum {
        if lag.is_multiple_of(32) && cancel() {
            return Err(AnalysisError::Cancelled);
        }
        let sum = (0..count)
            .map(|i| (samples[i] - samples[i + lag]).powi(2))
            .sum::<f64>();
        running += sum;
        raw[lag] = sum;
        difference[lag] = if running > 1e-20 {
            sum * lag as f64 / running
        } else {
            1.0
        };
        let candidate = lag - 1;
        if candidate >= minimum
            && difference[candidate] < 0.15
            && difference[candidate] <= difference[candidate - 1]
            && difference[candidate] < difference[lag]
        {
            selected = Some(candidate);
            break;
        }
    }
    let Some(lag) = selected else {
        return Ok(None);
    };
    let left = raw[lag - 1];
    let center = raw[lag];
    let right = raw[lag + 1];
    let denominator = left - 2.0 * center + right;
    let fraction = if denominator.abs() > 1e-12 {
        ((left - right) / (2.0 * denominator)).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    let hz = 24000.0 / (lag as f64 + fraction);
    let hz = refine_fundamental(&samples, hz, cancel)?;
    Ok((19.99..=4000.1).contains(&hz).then_some(PitchEstimate {
        hz: hz.clamp(20.0, 4000.0),
        confidence: (1.0 - difference[lag]).clamp(0.0, 1.0),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spectral_fine_tuning_corrects_decay_bias_without_switching_octaves_or_weak_fundamentals() {
        let hz = 247.091;
        let signal = (0..9600)
            .map(|i| {
                let t = i as f64 / 24000.0;
                let phase = std::f64::consts::TAU * hz * t;
                (0.08 * phase.sin() + 0.7 * (phase * 2.0).sin() + 0.2 * (phase * 3.0).sin())
                    * (-6.0 * t).exp()
            })
            .collect::<Vec<_>>();
        assert!((refine_fundamental(&signal, 245.71, &|| false).unwrap() - hz).abs() < 0.05);
        assert_eq!(
            refine_fundamental(&signal, 220.0, &|| false).unwrap(),
            220.0
        );
        let missing = (0..9600)
            .map(|i| (std::f64::consts::TAU * hz * 2.0 * i as f64 / 24000.0).sin())
            .collect::<Vec<_>>();
        assert_eq!(
            refine_fundamental(&missing, 245.71, &|| false).unwrap(),
            245.71
        );
        assert!(matches!(
            refine_fundamental(&signal, 245.71, &|| true),
            Err(AnalysisError::Cancelled)
        ));
    }
}
