use crate::{AnalysisError, SpectralPoint, Spectrogram, Spectrum};
use rustfft::{FftPlanner, num_complex::Complex};

pub(crate) fn spectral_features(
    mono: &[f32],
    rate: u32,
    size: usize,
    offset: f64,
    cancel: &impl Fn() -> bool,
) -> Result<(Spectrum, Spectrogram, Vec<SpectralPoint>), AnalysisError> {
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(size);
    let mut scratch = vec![Complex::default(); fft.get_inplace_scratch_len()];
    let mut buffer = vec![Complex::default(); size];
    let window = (0..size)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / size as f32).cos())
        .collect::<Vec<_>>();
    let gain = window.iter().sum::<f32>();
    let bins = size / 2 + 1;
    let hop = (size / 4).max(mono.len().div_ceil(1024));
    let rows = 256.min(bins);
    let frequencies = (0..rows)
        .map(|i| 20.0_f32 * (rate as f32 * 0.5 / 20.0).powf(i as f32 / (rows - 1) as f32))
        .collect::<Vec<_>>();
    let mut gram = Spectrogram {
        times: Vec::new(),
        frequencies,
        db: Vec::new(),
    };
    let mut points = Vec::new();
    let mut average = vec![0.0; bins];
    let mut previous = vec![0.0; bins];
    let mut averaged = 0;
    let mut has_steady = false;
    for start in (0..mono.len().max(1)).step_by(hop) {
        if cancel() {
            return Err(AnalysisError::Cancelled);
        }
        for i in 0..size {
            buffer[i] = Complex::new(mono.get(start + i).copied().unwrap_or(0.0) * window[i], 0.0);
        }
        fft.process_with_scratch(&mut buffer, &mut scratch);
        let magnitude = (0..bins)
            .map(|i| buffer[i].norm() / gain * if i == 0 || i == bins - 1 { 1.0 } else { 2.0 })
            .collect::<Vec<_>>();
        let total = magnitude.iter().sum::<f32>();
        let centroid = if total > 1e-12 {
            magnitude
                .iter()
                .enumerate()
                .map(|(i, v)| i as f32 * rate as f32 / size as f32 * v)
                .sum::<f32>()
                / total
        } else {
            0.0
        };
        let flux = if total > 1e-12 {
            magnitude
                .iter()
                .zip(&previous)
                .map(|(a, b)| (a - b).max(0.0))
                .sum::<f32>()
                / total
        } else {
            0.0
        };
        let seconds = offset + (start + size / 2).min(mono.len()) as f64 / f64::from(rate);
        points.push(SpectralPoint {
            seconds,
            centroid_hz: centroid,
            flux,
        });
        gram.times.push(seconds as f32);
        for hz in &gram.frequencies {
            let bin = (*hz * size as f32 / rate as f32).round() as usize;
            gram.db
                .push(20.0 * magnitude[bin.min(bins - 1)].max(1e-6).log10());
        }
        // Use complete middle-half windows for a stable spectrum; for short
        // clips keep the first padded window rather than return fabricated zeros.
        let steady = start >= mono.len() / 4 && start + size <= mono.len() * 3 / 4;
        if steady || start == 0 {
            if steady && !has_steady {
                average.fill(0.0);
                averaged = 0;
                has_steady = true;
            }
            for (sum, value) in average.iter_mut().zip(&magnitude) {
                *sum += value;
            }
            averaged += 1;
        }
        previous = magnitude;
    }
    for value in &mut average {
        *value /= averaged.max(1) as f32;
    }
    let spectrum = Spectrum {
        frequencies: (0..bins)
            .map(|i| i as f32 * rate as f32 / size as f32)
            .collect(),
        magnitudes: average,
    };
    Ok((spectrum, gram, points))
}
