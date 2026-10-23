//! Bounded peak trajectories and phase-independent modulation evidence.
//! Uses the existing STFT; no per-candidate pitch detector or extra FFT plans.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Peak {
    pub hz: f64,
    pub amplitude: f64,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Tracks {
    pub partials: Vec<Vec<Peak>>,
    pub pitch: Vec<Peak>,
    pub periodicity: Vec<f64>,
}
fn peaks(row: &[f32], rate: f64, size: usize) -> Vec<Peak> {
    let maximum = row.iter().copied().fold(0.0, f32::max) as f64;
    let mut result = Vec::new();
    for i in 2..row.len().saturating_sub(1) {
        let value = row[i] as f64;
        if value < maximum * 0.003 || row[i] <= row[i - 1] || row[i] < row[i + 1] {
            continue;
        }
        let a = (row[i - 1] as f64).max(1e-20).ln();
        let b = value.max(1e-20).ln();
        let c = (row[i + 1] as f64).max(1e-20).ln();
        let denominator = a - 2.0 * b + c;
        let delta = if denominator.abs() > 1e-12 {
            (0.5 * (a - c) / denominator).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        let hz = (i as f64 + delta) * rate / size as f64;
        if hz >= 30.0 {
            result.push(Peak {
                hz,
                amplitude: value,
            });
        }
    }
    result.sort_by(|a, b| b.amplitude.total_cmp(&a.amplitude));
    result.truncate(32);
    result
}
pub(crate) fn anchor_peaks(spectra: &[f32], size: usize, rate: f64) -> Vec<Peak> {
    let bins = size / 2 + 1;
    let mut average = vec![0.0; bins];
    for row in spectra.chunks_exact(bins) {
        for (a, b) in average.iter_mut().zip(row) {
            *a += *b;
        }
    }
    let mut p = peaks(&average, rate, size);
    if let Some(first) = p.first() {
        let threshold = first.amplitude * 0.01;
        p.retain(|p| p.amplitude >= threshold);
    }
    p.truncate(12);
    p
}
fn nearest(peaks: &[Peak], hz: f64, tolerance: f64) -> Peak {
    peaks
        .iter()
        .copied()
        .filter(|p| (1200.0 * (p.hz / hz).log2()).abs() <= tolerance)
        .min_by(|a, b| (a.hz / hz).ln().abs().total_cmp(&(b.hz / hz).ln().abs()))
        .unwrap_or_default()
}
/// Autocorrelation of a detrended log curve: insensitive to oscillator start phase.
/// Short/quiet curves deliberately supply no periodicity evidence.
fn periodicity(curve: &[f64], hop_seconds: f64) -> Vec<f64> {
    if curve.len() < 24 || curve.iter().copied().fold(0.0, f64::max) < 1e-9 {
        return Vec::new();
    }
    let floor = curve.iter().copied().fold(0.0, f64::max) * 0.01;
    let log: Vec<_> = curve.iter().map(|v| v.max(floor).ln()).collect();
    let radius = ((0.12 / hop_seconds).round() as usize).max(2);
    if log.len() <= radius * 2 + 8 {
        return Vec::new();
    }
    let residual: Vec<_> = (radius..log.len() - radius)
        .map(|i| {
            log[i] - log[i - radius..=i + radius].iter().sum::<f64>() / (2 * radius + 1) as f64
        })
        .collect();
    let energy = residual.iter().map(|v| v * v).sum::<f64>() / residual.len() as f64;
    let mut result = vec![energy.sqrt()];
    for hz in [
        3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 10.0, 12.0, 16.0, 24.0, 48.0, 72.0,
    ] {
        let lag = (1.0 / (hz * hop_seconds)).round().max(1.0) as usize;
        if residual.len() < lag * 2 || energy < 1e-8 {
            result.push(0.0);
            continue;
        }
        let a = &residual[..residual.len() - lag];
        let b = &residual[lag..];
        let cross = a.iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
        let norm =
            (a.iter().map(|v| v * v).sum::<f64>() * b.iter().map(|v| v * v).sum::<f64>()).sqrt();
        result.push(cross / norm.max(1e-20) * energy.sqrt().min(1.0));
    }
    result
}
pub(crate) fn extract(
    spectra: &[f32],
    size: usize,
    rate: f64,
    pitch: f64,
    anchors: &[f64],
    rms: &[f64],
) -> Tracks {
    let bins = size / 2 + 1;
    let mut partials = vec![Vec::new(); anchors.len()];
    let mut pitch_curve = Vec::new();
    for row in spectra.chunks_exact(bins) {
        let p = peaks(row, rate, size);
        let mut used = vec![false; p.len()];
        for (curve, hz) in partials.iter_mut().zip(anchors) {
            let found = p
                .iter()
                .enumerate()
                .filter(|(i, p)| !used[*i] && (1200.0 * (p.hz / hz).log2()).abs() <= 250.0)
                .min_by(|(_, a), (_, b)| (a.hz / hz).ln().abs().total_cmp(&(b.hz / hz).ln().abs()))
                .map(|(i, _)| i);
            if let Some(i) = found {
                used[i] = true;
                curve.push(p[i]);
            } else {
                curve.push(Peak::default());
            }
        }
        let mut root = nearest(&p, pitch, 180.0);
        if root.amplitude == 0.0 {
            // Missing fundamentals can still be supported by a harmonic. Mark
            // that weaker evidence rather than inventing a perfectly matched 0Hz.
            for harmonic in 2..=4 {
                let mut alternative = nearest(&p, pitch * harmonic as f64, 100.0);
                alternative.hz /= harmonic as f64;
                alternative.amplitude *= 0.25;
                if alternative.amplitude > root.amplitude {
                    root = alternative;
                }
            }
        }
        pitch_curve.push(root);
    }
    Tracks {
        partials,
        pitch: pitch_curve,
        periodicity: periodicity(rms, 0.005),
    }
}
fn curve_error(target: &[Peak], candidate: &[Peak], frequency_only: bool) -> f64 {
    let maximum = target
        .iter()
        .map(|p| p.amplitude)
        .fold(0.0, f64::max)
        .max(1e-20);
    let mut error = 0.0;
    let mut weight = 0.0;
    for (a, b) in target.iter().zip(candidate) {
        let w = a.amplitude / maximum;
        if w < 0.01 {
            continue;
        }
        let frequency = if b.hz > 0.0 && b.amplitude > maximum * 0.003 {
            (1200.0 * (b.hz / a.hz).log2()).abs().min(600.0) / 100.0
        } else {
            6.0
        };
        let amplitude = if frequency_only {
            0.0
        } else {
            (a.amplitude - b.amplitude).abs() / maximum
        };
        error += w * (frequency + amplitude);
        weight += w;
    }
    error / weight.max(1e-20)
}
pub(crate) fn compare(target: &Tracks, candidate: &Tracks) -> (f64, f64, f64) {
    let pitch = curve_error(&target.pitch, &candidate.pitch, true);
    let mut partial_error = 0.0;
    let mut partial_weight = 0.0;
    for (a, b) in target.partials.iter().zip(&candidate.partials) {
        let weight = a.iter().map(|p| p.amplitude).fold(0.0, f64::max);
        partial_error += weight * curve_error(a, b, false);
        partial_weight += weight;
    }
    let strength = |tracks: &Tracks| {
        tracks
            .partials
            .iter()
            .map(|curve| curve.iter().map(|p| p.amplitude.powi(2)).sum::<f64>())
            .collect::<Vec<_>>()
    };
    let a = strength(target);
    let b = strength(candidate);
    let aa = a.iter().sum::<f64>().max(1e-20);
    let bb = b.iter().sum::<f64>().max(1e-20);
    // Integrated relative energy prevents a correct root from displacing a
    // stronger upper partial. It also retains evidence across different decays.
    let balance = a
        .iter()
        .zip(&b)
        .map(|(a, b)| (a / aa - b / bb).abs())
        .sum::<f64>();
    let partials = partial_error / partial_weight.max(1e-20) + balance;
    let periodicity = if target.periodicity.is_empty() {
        0.0
    } else {
        target
            .periodicity
            .iter()
            .enumerate()
            .map(|(i, a)| (a - candidate.periodicity.get(i).copied().unwrap_or(0.0)).abs())
            .sum::<f64>()
            / target.periodicity.len() as f64
    };
    (pitch, partials, periodicity)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn amplitude_periodicity_distinguishes_tremolo_from_decay_and_is_phase_tolerant() {
        let curve = |phase: f64, depth: f64| {
            (0..400)
                .map(|i| {
                    let t = i as f64 * 0.005;
                    (-t).exp() * (1.0 + depth * (std::f64::consts::TAU * 6.0 * t + phase).sin())
                })
                .collect::<Vec<_>>()
        };
        let p = periodicity(&curve(0.0, 0.4), 0.005);
        let q = periodicity(&curve(1.2, 0.4), 0.005);
        let plain = periodicity(&curve(0.0, 0.0), 0.005);
        assert!(p[0] > 0.2);
        assert!(plain[0] < 1e-10);
        assert!((p[0] - q[0]).abs() < 0.02);
        assert!(periodicity(&[0.0; 100], 0.005).is_empty());
    }
    #[test]
    fn resolved_partials_cannot_share_a_single_candidate_peak() {
        let mut a = vec![0.0; 201];
        a[40] = 1.0;
        a[43] = 0.7;
        let mut b = a.clone();
        b[43] = 0.0;
        let target = extract(&a, 400, 1000.0, 100.0, &[100.0, 107.5], &[1.0]);
        let candidate = extract(&b, 400, 1000.0, 100.0, &[100.0, 107.5], &[1.0]);
        assert!(compare(&target, &candidate).1 > 2.5);
    }
    #[test]
    fn relative_partial_energy_distinguishes_inverted_dominance_and_is_gain_invariant() {
        let tracks = |amplitudes: [f64; 2]| Tracks {
            partials: amplitudes
                .into_iter()
                .enumerate()
                .map(|(i, amplitude)| {
                    vec![
                        Peak {
                            hz: 528.0 * (i + 1) as f64,
                            amplitude
                        };
                        8
                    ]
                })
                .collect(),
            ..Default::default()
        };
        let reference = tracks([0.5, 1.0]);
        let inverted = tracks([1.0, 0.5]);
        // Frequency-only terms are identical, but energy belongs to the wrong partial.
        assert_eq!(compare(&reference, &reference).1, 0.0);
        assert!(compare(&reference, &inverted).1 > 1.2);
        let scaled = tracks([5.0, 10.0]);
        let energy_balance = |a: &Tracks, b: &Tracks| {
            compare(a, b).1
                - a.partials
                    .iter()
                    .zip(&b.partials)
                    .map(|(a, b)| a[0].amplitude * curve_error(a, b, false))
                    .sum::<f64>()
                    / 1.5
        };
        assert!(energy_balance(&reference, &scaled).abs() < 1e-12);
    }
    #[test]
    fn missing_partials_are_penalized_and_silent_frames_are_ignored() {
        let target = vec![
            Peak {
                hz: 528.0,
                amplitude: 1.0,
            },
            Peak::default(),
        ];
        let same = target.clone();
        let shifted = vec![
            Peak {
                hz: 540.0,
                amplitude: 1.0
            };
            2
        ];
        assert_eq!(curve_error(&target, &same, true), 0.0);
        assert!(curve_error(&target, &shifted, true) > 0.3);
        assert_eq!(curve_error(&target, &[Peak::default(); 2], true), 6.0);
    }
}
