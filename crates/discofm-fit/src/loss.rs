use crate::{FitContext, FitError, FitInput, LOSS_VERSION};
use discofm_analysis::{ChannelMode, analyze_cancellable};
use discofm_audio::StreamingResampler;
use discofm_engine::render_note_cancellable;
use realfft::{RealFftPlanner, RealToComplex, num_complex::Complex};
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

const RATE: u32 = 24000;
const BANDS: usize = 24;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LossWeights {
    #[serde(default)]
    pub pitch: f64,
    #[serde(default)]
    pub partials: f64,
    #[serde(default)]
    pub periodicity: f64,
    pub log_spectral: f64,
    pub convergence: f64,
    pub envelope: f64,
    pub harmonic: f64,
    pub transient: f64,
    pub modulation: f64,
    pub tail: f64,
    /// Version 5: smoothed band-energy trajectories; absent in older archives.
    #[serde(default)]
    pub bands: f64,
    /// Version 6: energy off the reference's harmonic grid. `None` (archives
    /// and configs saved before v6) scores 0; `fit` resolves it for new runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inharmonic: Option<f64>,
}
pub const DEFAULT_INHARMONIC_WEIGHT: f64 = 0.5;
impl Default for LossWeights {
    fn default() -> Self {
        Self {
            inharmonic: Some(DEFAULT_INHARMONIC_WEIGHT),
            bands: 0.5,
            pitch: 0.5,
            partials: 0.75,
            periodicity: 0.5,
            log_spectral: 1.0,
            convergence: 1.0,
            envelope: 2.5,
            harmonic: 0.25,
            transient: 0.35,
            modulation: 0.25,
            tail: 0.5,
        }
    }
}
impl LossWeights {
    /// Percussion preset: pitch-anchored terms compare against an arbitrary
    /// playback note, so they are dropped. Per-bin spectral terms keep full
    /// weight: lowering them let noise seeds win early and stall the search.
    pub fn unpitched() -> Self {
        Self {
            pitch: 0.0,
            partials: 0.0,
            periodicity: 0.0,
            harmonic: 0.0,
            inharmonic: Some(0.0),
            bands: 1.0,
            ..Self::default()
        }
    }
    /// Fills a weight missing from an older save with its current default.
    pub fn resolved(&self) -> Self {
        Self {
            inharmonic: Some(self.inharmonic.unwrap_or(DEFAULT_INHARMONIC_WEIGHT)),
            ..self.clone()
        }
    }
    pub(crate) fn values(&self) -> [f64; 12] {
        [
            self.pitch,
            self.partials,
            self.periodicity,
            self.log_spectral,
            self.convergence,
            self.envelope,
            self.harmonic,
            self.transient,
            self.modulation,
            self.tail,
            self.bands,
            self.inharmonic.unwrap_or(0.0),
        ]
    }
    pub fn validate(&self) -> Result<(), FitError> {
        let values = self.values();
        if values
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=10000.0).contains(v))
            || values.iter().sum::<f64>() <= 0.0
        {
            return Err(FitError::Settings(
                "loss weights must be finite, nonnegative, <=10000 and not all zero",
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LossBreakdown {
    #[serde(default)]
    pub pitch: f64,
    #[serde(default)]
    pub partials: f64,
    #[serde(default)]
    pub periodicity: f64,
    #[serde(default = "unity")]
    pub recording_gain: f64,
    pub total: f64,
    pub log_spectral: f64,
    pub convergence: f64,
    pub envelope: f64,
    pub harmonic: f64,
    pub transient: f64,
    pub modulation: f64,
    pub tail: f64,
    #[serde(default)]
    pub bands: f64,
    #[serde(default)]
    pub inharmonic: f64,
}
fn unity() -> f64 {
    1.0
}
impl LossBreakdown {
    fn values(self) -> [f64; 12] {
        [
            self.pitch,
            self.partials,
            self.periodicity,
            self.log_spectral,
            self.convergence,
            self.envelope,
            self.harmonic,
            self.transient,
            self.modulation,
            self.tail,
            self.bands,
            self.inharmonic,
        ]
    }
    fn weighted(&self, weights: &LossWeights) -> f64 {
        self.values()
            .iter()
            .zip(weights.values())
            .map(|(v, w)| v * w)
            .sum::<f64>()
            / weights.values().iter().sum::<f64>()
    }
    pub fn validate(&self, weights: &LossWeights) -> Result<(), FitError> {
        if !self.total.is_finite()
            || !self.recording_gain.is_finite()
            || !(0.0001..=10000.0).contains(&self.recording_gain)
            || self.total < 0.0
            || self.values().iter().any(|v| !v.is_finite() || *v < 0.0)
            || (self.weighted(weights) - self.total).abs() > 1e-9 * (1.0 + self.total)
        {
            return Err(FitError::Settings(
                "invalid loss components or weighted score",
            ));
        }
        Ok(())
    }
}

/// Real-input FFT: half the work of a complex transform of the same frames.
struct Resolution {
    size: usize,
    centers: Vec<usize>,
    window: Vec<f32>,
    fft: Arc<dyn RealToComplex<f32>>,
    scratch: Vec<Complex<f32>>,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
}
impl Resolution {
    fn new(size: usize, frames: usize, planner: &mut RealFftPlanner<f32>) -> Self {
        // Continuous 75% overlap, including boundaries. No unobserved intervals
        // in long references; storage remains bounded by the 30-second limit.
        let mut centers = (0..frames).step_by(size / 4).collect::<Vec<_>>();
        if centers.last().copied() != Some(frames - 1) {
            centers.push(frames - 1);
        }
        let fft = planner.plan_fft_forward(size);
        Self {
            size,
            centers,
            window: (0..size)
                .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / size as f32).cos())
                .collect(),
            scratch: fft.make_scratch_vec(),
            input: fft.make_input_vec(),
            spectrum: fft.make_output_vec(),
            fft,
        }
    }
    /// Same layout and shared plan, with private scratch for another worker.
    fn fork(&self) -> Self {
        Self {
            size: self.size,
            centers: self.centers.clone(),
            window: self.window.clone(),
            fft: Arc::clone(&self.fft),
            scratch: self.fft.make_scratch_vec(),
            input: self.fft.make_input_vec(),
            spectrum: self.fft.make_output_vec(),
        }
    }
    fn extract(
        &mut self,
        signal: &[f32],
        cancel: &impl Fn() -> bool,
    ) -> Result<Vec<f32>, FitError> {
        let mut output = Vec::with_capacity(self.centers.len() * (self.size / 2 + 1));
        for center in &self.centers {
            check(cancel)?;
            for (i, value) in self.input.iter_mut().enumerate() {
                let source = (*center as isize + i as isize - self.size as isize / 2) as usize;
                *value = signal.get(source).copied().unwrap_or(0.0) * self.window[i];
            }
            self.fft
                .process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch)
                .map_err(|e| FitError::Internal(format!("FFT: {e}")))?;
            output.extend(
                self.spectrum
                    .iter()
                    .map(|v| v.norm() * (4.0 / self.size as f32)),
            );
        }
        Ok(output)
    }
}
struct Features {
    tracks: crate::tracking::Tracks,
    spectra: Vec<Vec<f32>>,
    rms: Vec<f64>,
    smooth_rms: Vec<f64>,
    harmonic: Vec<f64>,
    off_grid: f64,
    centroid: Vec<f64>,
    flux: Vec<f64>,
    roughness: f64,
    bands: Vec<f64>,
}
pub(crate) struct Objective {
    pub context: FitContext,
    target: Features,
    plans: Vec<Resolution>,
    floor: f64,
    band_floor: f64,
    level: f64,
    channel: ChannelMode,
    remove_dc: bool,
    weights: LossWeights,
    crop: usize,
    tail_begin: usize,
    match_level: bool,
    pub performance: crate::FitPerformance,
    /// GPU slots from calibration, handed to workers as they are created.
    gpu_slots: Vec<crate::gpu::GpuScore>,
    gpu: bool,
    pool: Vec<Worker>,
    /// Threads live for the whole fit: the coordinate stages send thousands
    /// of small batches, and spawning threads for each one cost more than
    /// scoring a short candidate.
    threads: rayon::ThreadPool,
    target_logs: Vec<Vec<f64>>,
    target_energy: Vec<f64>,
    anchors: Vec<f64>,
    pub partial_seeds: Vec<crate::tracking::Peak>,
    workers: usize,
}
pub(crate) fn check(cancel: &impl Fn() -> bool) -> Result<(), FitError> {
    if cancel() {
        Err(FitError::Cancelled)
    } else {
        Ok(())
    }
}
fn mono(
    frames: &[[f32; 2]],
    rate: f64,
    channel: ChannelMode,
    remove_dc: bool,
    gain: f64,
    count: Option<usize>,
    cancel: &impl Fn() -> bool,
) -> Result<Vec<f32>, FitError> {
    let mean = if remove_dc {
        frames
            .iter()
            .map(|f| channel_sample(*f, channel))
            .sum::<f64>()
            / frames.len().max(1) as f64
    } else {
        0.0
    };
    let mut resampler = StreamingResampler::new(rate, RATE)?;
    let mut result =
        Vec::with_capacity((frames.len() as f64 * f64::from(RATE) / rate).round() as usize);
    let mut output = [[0.0; 2]; 512];
    for block in frames.chunks(512) {
        check(cancel)?;
        let input = block
            .iter()
            .map(|f| [((channel_sample(*f, channel) - mean) * gain) as f32; 2])
            .collect::<Vec<_>>();
        resampler.push(&input)?;
        loop {
            let n = resampler.read_into(&mut output);
            result.extend(output[..n].iter().map(|f| f[0]));
            if n == 0 {
                break;
            }
        }
    }
    resampler.finish();
    loop {
        check(cancel)?;
        let n = resampler.read_into(&mut output);
        result.extend(output[..n].iter().map(|f| f[0]));
        if n == 0 {
            break;
        }
    }
    if let Some(count) = count {
        result.resize(count, 0.0);
        result.truncate(count);
    }
    Ok(result)
}
fn channel_sample(frame: [f32; 2], channel: ChannelMode) -> f64 {
    match channel {
        ChannelMode::Mix => (f64::from(frame[0]) + f64::from(frame[1])) * 0.5,
        ChannelMode::Left => frame[0].into(),
        ChannelMode::Right => frame[1].into(),
    }
}
fn condition(signal: &mut [f32], remove_dc: bool, gain: f64) {
    let mean = if remove_dc {
        signal.iter().map(|v| f64::from(*v)).sum::<f64>() / signal.len().max(1) as f64
    } else {
        0.0
    };
    for v in signal {
        *v = ((f64::from(*v) - mean) * gain) as f32;
    }
}
fn features(
    signal: &[f32],
    plans: &mut [Resolution],
    pitch: f64,
    tail_begin: usize,
    cancel: &impl Fn() -> bool,
    supplied_spectra: Option<Vec<Vec<f32>>>,
    anchors: &[f64],
) -> Result<Features, FitError> {
    check(cancel)?;
    let spectra = if let Some(spectra) = supplied_spectra {
        spectra
    } else {
        plans
            .iter_mut()
            .map(|p| p.extract(signal, cancel))
            .collect::<Result<Vec<_>, _>>()?
    };
    let rms_window = (2.0 * f64::from(RATE) / pitch).round().clamp(240.0, 960.0) as usize;
    check(cancel)?;
    let energy = squared_prefix(signal);
    let rms = rms_envelope(&energy, rms_window);
    // A second, longer window distinguishes the musical decay from beating
    // and phase-dependent fluctuations in the pitch-adaptive envelope.
    let smooth_rms = rms_envelope(&energy, 960);
    let largest = plans.last().expect("three resolutions");
    let bins = largest.size / 2 + 1;
    let mut average = vec![0.0_f64; bins];
    let mut centroid = Vec::new();
    let mut flux = Vec::new();
    let mut previous = vec![0.0_f64; bins];
    for window in spectra.last().expect("three spectra").chunks_exact(bins) {
        let energy = window.iter().map(|v| f64::from(*v)).sum::<f64>().max(1e-12);
        centroid.push(
            window
                .iter()
                .enumerate()
                .map(|(i, v)| i as f64 * f64::from(*v))
                .sum::<f64>()
                / energy
                / (bins - 1) as f64,
        );
        let mut change = 0.0;
        for (i, v) in window.iter().enumerate() {
            let value = f64::from(*v);
            average[i] += value / largest.centers.len() as f64;
            let normalized = value / energy;
            change += (normalized - previous[i]).max(0.0);
            previous[i] = normalized;
        }
        flux.push(change);
    }
    let mut harmonic = Vec::new();
    for k in 1..=32 {
        let center = (pitch * k as f64 * largest.size as f64 / f64::from(RATE)).round() as usize;
        if center >= bins - 1 {
            break;
        }
        harmonic.push(
            average[center.saturating_sub(1)..(center + 2).min(bins)]
                .iter()
                .copied()
                .fold(0.0, f64::max),
        );
    }
    let sum = harmonic.iter().sum::<f64>().max(1e-12);
    for v in &mut harmonic {
        *v /= sum;
    }
    let off_grid = off_grid_share(&average, largest.size, pitch);
    let tail = &signal[tail_begin.min(signal.len())..];
    let energy = tail
        .iter()
        .map(|v| f64::from(*v).powi(2))
        .sum::<f64>()
        .max(1e-20);
    let roughness = (tail
        .windows(2)
        .map(|w| (f64::from(w[1]) - f64::from(w[0])).powi(2))
        .sum::<f64>()
        / energy)
        .sqrt();
    let bands = band_energies(spectra.last().expect("largest spectrum"), largest.size);
    Ok(Features {
        bands,
        tracks: crate::tracking::extract(
            spectra.last().expect("largest spectrum"),
            largest.size,
            RATE.into(),
            pitch,
            anchors,
            &rms,
        ),
        spectra,
        rms,
        smooth_rms,
        harmonic,
        off_grid,
        centroid,
        flux,
        roughness,
    })
}
/// Share of average power between harmonics, from f0/4 to 11 kHz. Harmonic
/// peaks alone cannot see a MUL ×½ patch that plays an octave low: its even
/// partials land on every reference harmonic while half of its energy sits at
/// (k + ½)·f0, which sounds like a square/buzz an octave down.
fn off_grid_share(average: &[f64], size: usize, pitch: f64) -> f64 {
    let hz = f64::from(RATE) / size as f64;
    let first = (0.25 * pitch / hz).ceil() as usize;
    let last = ((32.25 * pitch).min(11000.0) / hz).floor() as usize;
    // At least the Hann main lobe, so leakage of a low f0 stays on the grid.
    let tolerance = (0.25 * pitch).max(2.0 * hz);
    let (mut on, mut off) = (0.0, 0.0);
    for (i, value) in average
        .iter()
        .enumerate()
        .take(last.min(average.len().saturating_sub(1)) + 1)
        .skip(first)
    {
        let f = i as f64 * hz;
        let nearest = (f / pitch).round().max(1.0) * pitch;
        let power = value * value;
        if (f - nearest).abs() <= tolerance {
            on += power;
        } else {
            off += power;
        }
    }
    off / (on + off).max(1e-30)
}
/// Third-octave-like bands from 40 Hz to 11 kHz over the largest STFT, which
/// both CPU and GPU paths provide. Summing bins averages the realization
/// variance of chaotic feedback-7 "noise", which per-bin errors cannot.
fn band_energies(spectrum: &[f32], size: usize) -> Vec<f64> {
    let bins = size / 2 + 1;
    let edges: Vec<usize> = (0..=BANDS)
        .map(|i| {
            let hz = 40.0 * (11000.0_f64 / 40.0).powf(i as f64 / BANDS as f64);
            ((hz * size as f64 / f64::from(RATE)).round() as usize).min(bins - 1)
        })
        .collect();
    let mut result = Vec::with_capacity(spectrum.len() / bins * BANDS);
    for window in spectrum.chunks_exact(bins) {
        for band in edges.windows(2) {
            result.push(
                window[band[0]..band[1].max(band[0] + 1)]
                    .iter()
                    .map(|v| f64::from(*v).powi(2))
                    .sum(),
            );
        }
    }
    result
}
/// Running sum of squares with a leading zero, so any window sum is one
/// subtraction instead of a pass over the window.
fn squared_prefix(signal: &[f32]) -> Vec<f64> {
    std::iter::once(0.0)
        .chain(signal.iter().scan(0.0, |sum, v| {
            *sum += f64::from(*v).powi(2);
            Some(*sum)
        }))
        .collect()
}
/// RMS every 120 samples over `width`; windows shorten at the end. The clamp
/// absorbs rounding where a silent window follows a loud passage.
fn rms_envelope(energy: &[f64], width: usize) -> Vec<f64> {
    let frames = energy.len() - 1;
    (0..frames)
        .step_by(120)
        .map(|start| {
            let end = (start + width).min(frames);
            ((energy[end] - energy[start]).max(0.0) / (end - start) as f64).sqrt()
        })
        .collect()
}
fn mean_difference(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(a, b)| (a - b).abs()).sum::<f64>() / a.len().max(1) as f64
}
fn log_envelope_difference(a: &[f64], b: &[f64], level: f64) -> f64 {
    let floor = (level * 0.01).max(1e-12);
    a.iter()
        .zip(b)
        .map(|(a, b)| (a.max(floor).ln() - b.max(floor).ln()).abs())
        .sum::<f64>()
        / (a.len().max(1) as f64 * 100.0_f64.ln())
}
fn equivalent_spectra(cpu: &[Vec<f32>], gpu: &[Vec<f32>]) -> bool {
    if cpu.len() != gpu.len() {
        return false;
    }
    cpu.iter().zip(gpu).all(|(a, b)| {
        if a.len() != b.len() || b.iter().any(|v| !v.is_finite() || *v < 0.0) {
            return false;
        }
        let energy = a.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>();
        let error = a
            .iter()
            .zip(b)
            .map(|(a, b)| f64::from(a - b).powi(2))
            .sum::<f64>();
        let peak = a.iter().copied().fold(0.0, f32::max);
        let maximum = a
            .iter()
            .zip(b)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        maximum <= peak * 0.0001 + 1e-12 && error <= energy * 1e-8 + 1e-24
    })
}
fn calibrate_gpu(
    signal: &[f32],
    plans: &mut [Resolution],
    target: &Features,
    mode: crate::ComputeMode,
    performance: &mut crate::FitPerformance,
    cancel: &impl Fn() -> bool,
) -> Result<Option<Vec<crate::gpu::GpuScore>>, FitError> {
    let gpu_error = |error| {
        if cancel() {
            FitError::Cancelled
        } else {
            FitError::Compute(error)
        }
    };
    let layouts = plans
        .iter()
        .map(|p| (p.centers.clone(), p.window.clone()))
        .collect::<Vec<_>>();
    let compute = crate::gpu::GpuFft::new(signal.len(), &layouts).map_err(gpu_error)?;
    performance.adapter = Some(compute.name().to_owned());
    let extract = || {
        compute.extract(signal, cancel).map_err(|e| {
            if cancel() {
                FitError::Cancelled
            } else {
                FitError::Compute(e)
            }
        })
    };
    let spectra = extract()?;
    performance.gpu_dispatches += 3;
    if !equivalent_spectra(&target.spectra, &spectra) {
        return Err(FitError::Compute("FFT disagrees with CPU".into()));
    }
    let peak = signal
        .iter()
        .map(|v| f64::from(v.abs()))
        .fold(0.0, f64::max);
    let floor = (peak * 1e-4).max(1e-8);
    // Search scores one candidate per worker slot as soon as it is rendered;
    // renders of a batch finish together, so measure that many concurrent
    // candidates on each backend rather than a single idealized one.
    let workers = performance.workers.max(1);
    let first = crate::gpu::GpuScore::new(signal.len(), &layouts, &target.spectra, floor, 1)
        .map_err(gpu_error)?;
    let mut slots = Vec::with_capacity(workers);
    for _ in 1..workers {
        slots.push(first.slot().map_err(gpu_error)?);
    }
    slots.insert(0, first);
    let mut cpu_plans: Vec<Vec<Resolution>> = (0..workers)
        .map(|_| plans.iter().map(Resolution::fork).collect())
        .collect();
    let cached_logs: Vec<Vec<f64>> = target
        .spectra
        .iter()
        .map(|s| s.iter().map(|v| f64::from(*v).max(floor).ln()).collect())
        .collect();
    let cpu_round = |plans: &mut Vec<Vec<Resolution>>| {
        std::thread::scope(|scope| {
            for plans in plans.iter_mut() {
                let cached_logs = &cached_logs;
                scope.spawn(move || {
                    for (index, (p, target)) in plans.iter_mut().zip(&target.spectra).enumerate() {
                        let candidate = p.extract(signal, &|| false).unwrap_or_default();
                        let log_distance: f64 = cached_logs[index]
                            .iter()
                            .zip(&candidate)
                            .map(|(a, b)| (*a - f64::from(*b).max(floor).ln()).abs())
                            .sum();
                        let squared_distance: f64 = target
                            .iter()
                            .zip(&candidate)
                            .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
                            .sum();
                        std::hint::black_box(log_distance + squared_distance);
                    }
                });
            }
        })
    };
    let gpu_round = |slots: &[crate::gpu::GpuScore]| {
        std::thread::scope(|scope| {
            let handles: Vec<_> = slots
                .iter()
                .map(|slot| scope.spawn(move || slot.score(&[signal], &|| false)))
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join()
                        .unwrap_or_else(|_| Err("GPU worker panicked".into()))
                })
                .collect::<Result<Vec<_>, String>>()
        })
    };
    gpu_round(&slots).map_err(gpu_error)?;
    performance.gpu_dispatches += 3 * workers;
    let mut cpu_times = Vec::new();
    let mut gpu_times = Vec::new();
    for _ in 0..3 {
        check(cancel)?;
        let start = std::time::Instant::now();
        cpu_round(&mut cpu_plans);
        cpu_times.push(start.elapsed().as_secs_f64() * 1000.0 / workers as f64);
        check(cancel)?;
        let start = std::time::Instant::now();
        let scores = gpu_round(&slots).map_err(gpu_error)?;
        performance.gpu_dispatches += 3 * workers;
        gpu_times.push(start.elapsed().as_secs_f64() * 1000.0 / workers as f64);
        for score in scores.into_iter().flatten() {
            if !equivalent_spectra(&[target.spectra[2].clone()], &[score.largest])
                || score
                    .log_sum
                    .iter()
                    .zip(&target.spectra)
                    .any(|(s, t)| s / t.len() as f64 > 1e-4)
                || score
                    .squared_error
                    .iter()
                    .zip(&target.spectra)
                    .any(|(e, t)| {
                        *e > t.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() * 1e-8 + 1e-20
                    })
            {
                return Err(FitError::Compute(
                    "GPU spectral reductions disagree with CPU".into(),
                ));
            }
        }
    }
    cpu_times.sort_by(f64::total_cmp);
    gpu_times.sort_by(f64::total_cmp);
    performance.cpu_fft_ms = cpu_times[1];
    performance.gpu_fft_ms = Some(gpu_times[1]);
    if mode == crate::ComputeMode::Gpu || gpu_times[1] < cpu_times[1] * 0.9 {
        performance.backend = "GPU · per-worker STFT and spectral loss".into();
        Ok(Some(slots))
    } else {
        performance.note = "CPU chosen by measurement: concurrent GPU transfers and \
             synchronisation cost more than parallel CPU spectra for this region"
            .into();
        Ok(None)
    }
}
#[derive(Clone)]
struct RenderSettings {
    context: FitContext,
    channel: ChannelMode,
    crop: usize,
    remove_dc: bool,
    match_level: bool,
    level: f64,
}
struct Worker {
    plans: Vec<Resolution>,
    gpu: Option<crate::gpu::GpuScore>,
}
struct Evaluated {
    loss: LossBreakdown,
    render_seconds: f64,
    resample_seconds: f64,
    features_seconds: f64,
    dispatches: usize,
}
struct PreparedSignal {
    signal: Vec<f32>,
    gain: f64,
    render_seconds: f64,
    resample_seconds: f64,
}
fn render_signal(
    settings: &RenderSettings,
    patch: &discofm_chip::Instrument,
    cancel: &impl Fn() -> bool,
) -> Result<PreparedSignal, FitError> {
    let started = std::time::Instant::now();
    let native = render_note_cancellable(patch, &settings.context.audition, cancel).map_err(
        |e| match e {
            discofm_engine::EngineError::Cancelled => FitError::Cancelled,
            other => FitError::Engine(other),
        },
    )?;
    prepare_native(settings, native, started, cancel)
}
fn prepare_native(
    settings: &RenderSettings,
    native: discofm_engine::NativeAudio,
    started: std::time::Instant,
    cancel: &impl Fn() -> bool,
) -> Result<PreparedSignal, FitError> {
    let render_seconds = started.elapsed().as_secs_f64();
    let started = std::time::Instant::now();
    check(cancel)?;
    // Same values as the streaming `mono` path, which duplicated one channel
    // through a stereo converter; this is half the work and bit-identical.
    let downmix: Vec<f32> = native
        .frames
        .iter()
        .map(|f| channel_sample(*f, settings.channel) as f32)
        .collect();
    let mut signal = discofm_audio::resample_mono(&downmix, native.sample_rate, RATE)?;
    signal.drain(..settings.crop.min(signal.len()));
    signal.resize(settings.context.frames, 0.0);
    signal.truncate(settings.context.frames);
    condition(
        &mut signal,
        settings.remove_dc,
        settings.context.reference_gain,
    );
    let rms =
        (signal.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / signal.len() as f64).sqrt();
    let recording_gain = if settings.match_level && rms > 1e-12 {
        (settings.level / rms).clamp(0.0001, 10000.0)
    } else {
        1.0
    };
    if settings.match_level {
        for v in &mut signal {
            *v *= recording_gain as f32;
        }
    }
    let resample_seconds = started.elapsed().as_secs_f64();
    Ok(PreparedSignal {
        signal,
        gain: recording_gain,
        render_seconds,
        resample_seconds,
    })
}
/// The note an unpitched (drum) patch is triggered at. Drum patches on the
/// YM2612 are keyed like instruments, so any fixed note is a valid choice;
/// MUL/DT/FB then shape the spectrum relative to it. `prepare` rejects results
/// outside 20..4000 Hz.
fn unpitched_hz(
    audition: &discofm_engine::RenderRequest,
    analysis: &discofm_analysis::Analysis,
) -> Result<f64, FitError> {
    let spectrum = &analysis.spectrum;
    match body_hz(
        &spectrum.frequencies,
        &spectrum.magnitudes,
        analysis.pitch.as_ref().map(|p| p.hz),
    ) {
        Some(hz) => Ok(hz),
        None => Ok(audition.pitch_hz()?),
    }
}
/// The lowest spectral peak within 6 dB of the strongest one in 20..4000 Hz.
/// Keying the note on the body mode lets a MUL 1 carrier land on it and MUL
/// 2..15 reach the brighter partials; keying on the loudest partial can leave
/// the body below MUL 0.5. Spectrum bins are coarse at kick frequencies, so a
/// low-confidence YIN estimate within a semitone refines the peak.
fn body_hz(frequencies: &[f32], magnitudes: &[f32], yin: Option<f64>) -> Option<f64> {
    let n = frequencies.len().min(magnitudes.len());
    let peaks: Vec<usize> = (1..n.saturating_sub(1))
        .filter(|&i| {
            (20.0..=4000.0).contains(&frequencies[i])
                && magnitudes[i] > magnitudes[i - 1]
                && magnitudes[i] >= magnitudes[i + 1]
        })
        .collect();
    let strongest = peaks.iter().map(|&i| magnitudes[i]).fold(0.0, f32::max);
    if strongest <= 0.0 || !strongest.is_finite() {
        return None;
    }
    let i = *peaks.iter().find(|&&i| magnitudes[i] >= strongest * 0.5)?;
    let log = |j: usize| f64::from(magnitudes[j].max(1e-12)).ln();
    let (a, b, c) = (log(i - 1), log(i), log(i + 1));
    let curvature = a - 2.0 * b + c;
    let offset = if curvature < 0.0 {
        (0.5 * (a - c) / curvature).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    let step = f64::from(frequencies[i + 1] - frequencies[i]);
    let peak = f64::from(frequencies[i]) + offset * step;
    let hz = match yin {
        Some(yin) if yin.is_finite() && yin > 0.0 && (yin / peak).log2().abs() <= 1.0 / 12.0 => yin,
        _ => peak,
    };
    Some(hz.clamp(20.0, 4000.0))
}
impl Objective {
    pub fn prepare(input: &FitInput, cancel: &impl Fn() -> bool) -> Result<Self, FitError> {
        check(cancel)?;
        let analysis =
            analyze_cancellable(&input.reference, &input.options, cancel).map_err(|error| {
                match error {
                    discofm_analysis::AnalysisError::Cancelled => FitError::Cancelled,
                    other => FitError::Analysis(other),
                }
            })?;
        if analysis.rms < 1e-7 {
            return Err(FitError::Silent);
        }
        let pitch = match input.options.pitch_override_hz {
            Some(hz) => hz,
            None if input.config.unpitched => unpitched_hz(&input.audition, &analysis)?,
            None => analysis
                .pitch
                .as_ref()
                .filter(|p| p.confidence >= 0.5)
                .map(|p| p.hz)
                .ok_or(FitError::Pitch)?,
        };
        if !(20.0..=4000.0).contains(&pitch) {
            return Err(FitError::Settings("fitting pitch must be 20..4000 Hz"));
        }
        // An amplitude threshold is not the trigger time: trimming must never
        // restart a slow envelope halfway through its attack.
        let onset = input.note_on_seconds;
        let scoring_start = onset.max(input.options.start_seconds);
        let end = input
            .options
            .end_seconds
            .unwrap_or(
                (input.options.start_seconds + discofm_analysis::MAX_SECONDS)
                    .min(analysis.original_seconds),
            )
            .min(analysis.original_seconds);
        let seconds = end - onset;
        if seconds > input.config.max_seconds + 1.0 / f64::from(input.reference.sample_rate) {
            return Err(FitError::Duration);
        }
        if end - scoring_start < 0.025
            || input.key_off_seconds <= onset
            || input.key_off_seconds > end + 0.5 / f64::from(input.reference.sample_rate)
        {
            return Err(FitError::Settings(
                "require at least 25 ms and onset < key-off <= selected region end",
            ));
        }
        let mut audition = input.effective_audition();
        audition.note = (69.0 + 12.0 * (pitch / 440.0).log2())
            .round()
            .clamp(0.0, 127.0) as u8;
        audition.tuning_cents = 1200.0 * (pitch / discofm_engine::midi_hz(audition.note)?).log2();
        audition.total_seconds = seconds.min(30.0);
        audition.key_on_seconds = (input.key_off_seconds - onset).min(audition.total_seconds);
        audition.validate()?;
        let from = (scoring_start * f64::from(input.reference.sample_rate)).round() as usize;
        let to = ((end * f64::from(input.reference.sample_rate)).round() as usize)
            .min(input.reference.frames.len());
        let gain = f64::from(analysis.normalization_gain);
        let mut signal = mono(
            &input.reference.frames[from..to],
            input.reference.sample_rate.into(),
            input.options.channel,
            false,
            1.0,
            None,
            cancel,
        )?;
        condition(&mut signal, input.options.remove_dc, gain);
        let peak = signal
            .iter()
            .map(|v| f64::from(v.abs()))
            .fold(0.0, f64::max);
        let level = (signal.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>()
            / signal.len().max(1) as f64)
            .sqrt();
        if level < 1e-7 {
            return Err(FitError::Silent);
        }
        let mut planner = RealFftPlanner::new();
        let mut plans = [512, 1024, 4096]
            .into_iter()
            .map(|n| Resolution::new(n, signal.len(), &mut planner))
            .collect::<Vec<_>>();
        let crop = ((scoring_start - onset) * f64::from(RATE)).round() as usize;
        // One-shot notes can decay before key-off. Always inspect at least the
        // final quarter instead of silently assigning an empty tail zero loss.
        let tail_begin = (((input.key_off_seconds - scoring_start).max(0.0) * f64::from(RATE))
            .round() as usize)
            .min(signal.len() * 3 / 4);
        let mut performance = crate::FitPerformance {
            backend: "CPU · realfft".into(),
            ..Default::default()
        };
        let mut target = features(&signal, &mut plans, pitch, tail_begin, cancel, None, &[])?;
        let partial_seeds = crate::tracking::anchor_peaks(
            target.spectra.last().expect("largest spectrum"),
            4096,
            RATE.into(),
        );
        let anchors: Vec<_> = partial_seeds.iter().map(|p| p.hz).collect();
        target.tracks = crate::tracking::extract(
            target.spectra.last().expect("largest spectrum"),
            4096,
            RATE.into(),
            pitch,
            &anchors,
            &target.rms,
        );
        let workers = if input.config.workers == 0 {
            crate::default_workers()
        } else {
            input.config.workers
        };
        performance.workers = workers;
        let mut gpu = None;
        if input.config.compute != crate::ComputeMode::Cpu {
            check(cancel)?;
            match calibrate_gpu(
                &signal,
                &mut plans,
                &target,
                input.config.compute,
                &mut performance,
                cancel,
            ) {
                Ok(compute) => gpu = compute,
                Err(FitError::Cancelled) => return Err(FitError::Cancelled),
                Err(error) if input.config.compute == crate::ComputeMode::Auto => {
                    performance.note = format!("GPU unavailable; using CPU: {error}")
                }
                Err(error) => return Err(error),
            }
        }
        let floor = (peak * 1e-4).max(1e-8);
        // -60 dB below the loudest reference band, as for per-bin log errors.
        let band_floor = (target.bands.iter().copied().fold(0.0, f64::max) * 1e-6).max(1e-20);
        let target_logs = target
            .spectra
            .iter()
            .map(|s| s.iter().map(|v| f64::from(*v).max(floor).ln()).collect())
            .collect();
        let target_energy = target
            .spectra
            .iter()
            .map(|s| s.iter().map(|v| f64::from(*v).powi(2)).sum())
            .collect();
        let mut hash = 0xcbf29ce484222325_u64;
        let settings = format!(
            "{LOSS_VERSION}:{audition:?}:{:?}:{:?}:{gain:?}:{}:{}",
            input.options, input.config.weights, input.config.match_level, performance.backend
        );
        for byte in settings
            .bytes()
            .chain(signal.iter().flat_map(|v| v.to_bits().to_le_bytes()))
        {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
        let context = FitContext {
            loss_version: LOSS_VERSION,
            fingerprint: format!("{hash:016x}"),
            sample_rate: RATE,
            frames: signal.len(),
            reference_onset_seconds: onset,
            reference_start_seconds: scoring_start,
            reference_end_seconds: end,
            reference_gain: gain,
            pitch_hz: pitch,
            audition,
        };
        Ok(Self {
            context,
            target,
            plans,
            floor,
            band_floor,
            level,
            channel: input.options.channel,
            remove_dc: input.options.remove_dc,
            weights: input.config.weights.clone(),
            crop,
            tail_begin,
            match_level: input.config.match_level,
            performance,
            gpu: gpu.is_some(),
            gpu_slots: gpu.unwrap_or_default(),
            pool: Vec::new(),
            threads: rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .thread_name(|i| format!("discofm-fit-{i}"))
                .build()
                .map_err(|error| FitError::Internal(format!("worker threads: {error}")))?,
            target_logs,
            target_energy,
            anchors,
            partial_seeds,
            workers,
        })
    }
    /// Scores every patch on a pool of workers that each render, convert and
    /// score whole candidates; no serial scoring stage idles the other workers.
    /// Scores are pure functions of the patch, so results and their order do
    /// not depend on the worker count. On cancellation the completed prefix is
    /// returned; unfinished or out-of-order work is discarded.
    pub fn evaluate_batch(
        &mut self,
        patches: &[discofm_chip::Instrument],
        cancel: &impl Fn() -> bool,
    ) -> Result<Vec<LossBreakdown>, FitError> {
        self.run_batch(patches, cancel, false)
    }
    /// Scores rendered through ymfm instead of Nuked, for ranking proposals
    /// before the Nuked render. Never recorded, cached or ranked.
    pub fn screen_batch(
        &mut self,
        patches: &[discofm_chip::Instrument],
        cancel: &impl Fn() -> bool,
    ) -> Result<Vec<f64>, FitError> {
        let scores = self.run_batch(patches, cancel, true)?;
        self.performance.screened += scores.len();
        Ok(scores.into_iter().map(|loss| loss.total).collect())
    }
    fn run_batch(
        &mut self,
        patches: &[discofm_chip::Instrument],
        cancel: &impl Fn() -> bool,
        screen: bool,
    ) -> Result<Vec<LossBreakdown>, FitError> {
        check(cancel)?;
        if patches.is_empty() {
            return Ok(Vec::new());
        }
        let settings = RenderSettings {
            context: self.context.clone(),
            channel: self.channel,
            crop: self.crop,
            remove_dc: self.remove_dc,
            match_level: self.match_level,
            level: self.level,
        };
        let workers = self.workers.clamp(1, patches.len());
        while self.pool.len() < workers {
            let gpu = if self.gpu {
                Some(match self.gpu_slots.pop() {
                    Some(slot) => slot,
                    None => self
                        .pool
                        .iter()
                        .find_map(|worker| worker.gpu.as_ref())
                        .ok_or_else(|| FitError::Internal("no calibrated GPU slot".into()))?
                        .slot()
                        .map_err(FitError::Compute)?,
                })
            } else {
                None
            };
            let plans = self.plans.iter().map(Resolution::fork).collect();
            self.pool.push(Worker { plans, gpu });
        }
        let mut pool = std::mem::take(&mut self.pool);
        let next = AtomicUsize::new(0);
        let stop = AtomicBool::new(false);
        let mut cancelled = false;
        let mut results: Vec<Option<Result<Evaluated, FitError>>> =
            (0..patches.len()).map(|_| None).collect();
        let this = &*self;
        // `in_place_scope` runs this closure on the calling thread, so the user
        // callback stays on the coordinator; workers observe an atomic flag,
        // preserving support for non-Sync cancellation APIs.
        self.threads.in_place_scope(|scope| {
            let (tx, rx) = std::sync::mpsc::channel();
            for worker in pool.iter_mut().take(workers) {
                let tx = tx.clone();
                let (next, stop, settings) = (&next, &stop, &settings);
                scope.spawn(move |_| {
                    let stopped = || stop.load(Ordering::Acquire);
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        if index >= patches.len() || stopped() {
                            break;
                        }
                        let result =
                            this.evaluate_one(settings, &patches[index], worker, &stopped, screen);
                        if result.is_err() {
                            stop.store(true, Ordering::Release);
                        }
                        if tx.send((index, result)).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(tx);
            loop {
                if !cancelled && cancel() {
                    cancelled = true;
                    stop.store(true, Ordering::Release);
                }
                match rx.recv_timeout(std::time::Duration::from_millis(2)) {
                    Ok((index, result)) => results[index] = Some(result),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        self.pool = pool;
        cancelled |= cancel();
        let mut scores = Vec::with_capacity(patches.len());
        let mut complete = true;
        for result in results {
            match result {
                Some(Ok(evaluated)) => {
                    if !screen {
                        self.performance.render_seconds += evaluated.render_seconds;
                        self.performance.resample_seconds += evaluated.resample_seconds;
                        self.performance.features_seconds += evaluated.features_seconds;
                        self.performance.gpu_dispatches += evaluated.dispatches;
                    }
                    if complete {
                        scores.push(evaluated.loss);
                    }
                }
                Some(Err(FitError::Cancelled)) | None => complete = false,
                Some(Err(error)) if !cancelled => return Err(error),
                Some(Err(_)) => complete = false,
            }
        }
        if scores.is_empty() || (scores.len() < patches.len() && !cancelled) {
            return Err(FitError::Cancelled);
        }
        Ok(scores)
    }
    fn evaluate_one(
        &self,
        settings: &RenderSettings,
        patch: &discofm_chip::Instrument,
        worker: &mut Worker,
        stop: &impl Fn() -> bool,
        screen: bool,
    ) -> Result<Evaluated, FitError> {
        let prepared = if screen {
            let started = std::time::Instant::now();
            let native = discofm_engine::render_ymfm_reference(patch, &settings.context.audition)
                .map_err(FitError::Engine)?;
            prepare_native(settings, native, started, stop)?
        } else {
            render_signal(settings, patch, stop)?
        };
        let mut gpu_seconds = 0.0;
        let mut dispatches = 0;
        let spectral = if !screen && let Some(gpu) = &worker.gpu {
            let started = std::time::Instant::now();
            let score = gpu
                .score(&[prepared.signal.as_slice()], stop)
                .map_err(|error| {
                    if stop() {
                        FitError::Cancelled
                    } else {
                        FitError::Compute(error)
                    }
                })?;
            gpu_seconds = started.elapsed().as_secs_f64();
            dispatches = 3;
            score.into_iter().next()
        } else {
            None
        };
        let started = std::time::Instant::now();
        let loss = self.score_signal(
            &mut worker.plans,
            prepared.signal,
            prepared.gain,
            spectral,
            stop,
        )?;
        Ok(Evaluated {
            loss,
            render_seconds: prepared.render_seconds,
            resample_seconds: prepared.resample_seconds,
            features_seconds: gpu_seconds + started.elapsed().as_secs_f64(),
            dispatches,
        })
    }
    fn score_signal(
        &self,
        plans: &mut [Resolution],
        signal: Vec<f32>,
        recording_gain: f64,
        mut spectral: Option<crate::gpu::SpectralScore>,
        cancel: &impl Fn() -> bool,
    ) -> Result<LossBreakdown, FitError> {
        let candidate = features(
            &signal,
            plans,
            self.context.pitch_hz,
            self.tail_begin,
            cancel,
            spectral
                .as_mut()
                .map(|s| vec![Vec::new(), Vec::new(), std::mem::take(&mut s.largest)]),
            &self.anchors,
        )?;
        let mut log_spectral = 0.0;
        let mut convergence = 0.0;
        if let Some(spectral) = spectral {
            for i in 0..3 {
                log_spectral += spectral.log_sum[i]
                    / (self.target.spectra[i].len() as f64 * 3.0 * std::f64::consts::LN_10);
                convergence += (spectral.squared_error[i] / self.target_energy[i].max(1e-20))
                    .sqrt()
                    .min(100.0);
            }
        } else {
            for (index, (a, b)) in self
                .target
                .spectra
                .iter()
                .zip(&candidate.spectra)
                .enumerate()
            {
                check(cancel)?;
                log_spectral += self.target_logs[index]
                    .iter()
                    .zip(b)
                    .map(|(a, b)| (*a - f64::from(*b).max(self.floor).ln()).abs())
                    .sum::<f64>()
                    / (a.len() as f64 * 60.0 / 20.0 * std::f64::consts::LN_10);
                let error = a
                    .iter()
                    .zip(b)
                    .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
                    .sum::<f64>();
                let energy = self.target_energy[index];
                convergence += (error / energy.max(1e-20)).sqrt().min(100.0);
            }
        }
        let envelope_error = |a: &[f64], b: &[f64], log_weight: f64| {
            let convergence = (a.iter().zip(b).map(|(a, b)| (a - b).powi(2)).sum::<f64>()
                / a.iter()
                    .map(|v| v * v)
                    .sum::<f64>()
                    .max(self.level.powi(2) * a.len() as f64)
                    .max(1e-20))
            .sqrt()
            .min(100.0);
            // Relative L2 protects the energetic attack; L1/log terms retain
            // sensitivity to quieter decay instead of averaging the attack away.
            0.5 * (mean_difference(a, b) / self.level + convergence)
                + log_weight * log_envelope_difference(a, b, self.level)
        };
        let envelope = 0.5
            * (envelope_error(&self.target.rms, &candidate.rms, 0.25)
                + envelope_error(&self.target.smooth_rms, &candidate.smooth_rms, 0.25));
        let harmonic = mean_difference(&self.target.harmonic, &candidate.harmonic)
            * self.target.harmonic.len() as f64;
        let derivative = |v: &[f64]| {
            v.windows(2)
                .map(|w| (w[1] - w[0]) / self.level)
                .collect::<Vec<_>>()
        };
        let transient = mean_difference(&derivative(&self.target.rms), &derivative(&candidate.rms));
        let modulation = mean_difference(&self.target.centroid, &candidate.centroid)
            + mean_difference(&self.target.flux, &candidate.flux);
        let begin = self.tail_begin.div_ceil(120).min(self.target.rms.len());
        let tail = 0.5
            * (envelope_error(&self.target.rms[begin..], &candidate.rms[begin..], 0.5)
                + envelope_error(
                    &self.target.smooth_rms[begin..],
                    &candidate.smooth_rms[begin..],
                    0.5,
                ))
            + 0.1 * (self.target.roughness - candidate.roughness).abs();
        let (pitch, partials, periodicity) =
            crate::tracking::compare(&self.target.tracks, &candidate.tracks);
        let bands = self
            .target
            .bands
            .iter()
            .zip(&candidate.bands)
            .map(|(a, b)| (a.max(self.band_floor).ln() - b.max(self.band_floor).ln()).abs())
            .sum::<f64>()
            / (self.target.bands.len().max(1) as f64 * 1e6_f64.ln());
        // One-sided: references with their own inharmonic or sub-harmonic
        // content (bells, detuned layers) allow the candidate as much.
        let inharmonic = ((candidate.off_grid + 0.01) / (self.target.off_grid + 0.01))
            .ln()
            .max(0.0);
        let mut loss = LossBreakdown {
            inharmonic,
            bands,
            pitch,
            partials,
            periodicity,
            recording_gain,
            total: 0.0,
            log_spectral: log_spectral / 3.0,
            convergence: convergence / 3.0,
            envelope,
            harmonic,
            transient,
            modulation,
            tail,
        };
        loss.total = loss.weighted(&self.weights);
        loss.validate(&self.weights)?;
        Ok(loss)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gpu_calibration_rejects_nonfinite_wrong_shapes_and_low_level_zero_outputs() {
        let cpu = vec![vec![1e-6, 2e-6, 3e-7]];
        assert!(equivalent_spectra(&cpu, &cpu));
        for bad in [
            vec![vec![f32::NAN; 3]],
            vec![vec![0.0; 3]],
            vec![vec![1e-6]],
            Vec::new(),
        ] {
            assert!(!equivalent_spectra(&cpu, &bad));
        }
    }
    #[test]
    #[ignore = "requires a physical wgpu compute adapter"]
    fn gpu_fft_agrees_with_cpu_for_silence_impulses_bass_and_mixed_spectra() {
        for frames in [4097, 24000] {
            let mut planner = RealFftPlanner::new();
            let mut plans = [512, 1024, 4096]
                .into_iter()
                .map(|n| Resolution::new(n, frames, &mut planner))
                .collect::<Vec<_>>();
            let layouts = plans
                .iter()
                .map(|p| (p.centers.clone(), p.window.clone()))
                .collect::<Vec<_>>();
            let gpu = crate::gpu::GpuFft::new(frames, &layouts)
                .expect("a real GPU is required for this opt-in test");
            eprintln!("GPU FFT validation: {}", gpu.name());
            for kind in 0..4 {
                let signal = (0..frames)
                    .map(|i| match kind {
                        0 => 0.0,
                        1 => {
                            if i == 0 || i == frames - 1 || i == 123 {
                                0.7
                            } else {
                                0.0
                            }
                        }
                        2 => (std::f32::consts::TAU * 65.583 * i as f32 / 24000.0).sin() * 0.8,
                        _ => {
                            (std::f32::consts::TAU * 440.0 * i as f32 / 24000.0).sin() * 0.3
                                + (std::f32::consts::TAU * 7999.0 * i as f32 / 24000.0).cos() * 0.17
                        }
                    })
                    .collect::<Vec<_>>();
                let expected = plans
                    .iter_mut()
                    .map(|p| p.extract(&signal, &|| false).unwrap())
                    .collect::<Vec<_>>();
                let actual = gpu.extract(&signal, &|| false).unwrap();
                for (cpu, gpu) in expected.iter().zip(&actual) {
                    assert_eq!(cpu.len(), gpu.len());
                    let maximum = cpu
                        .iter()
                        .zip(gpu)
                        .map(|(a, b)| (a - b).abs())
                        .fold(0.0, f32::max);
                    let relative = (cpu
                        .iter()
                        .zip(gpu)
                        .map(|(a, b)| f64::from(a - b).powi(2))
                        .sum::<f64>()
                        / cpu
                            .iter()
                            .map(|v| f64::from(*v).powi(2))
                            .sum::<f64>()
                            .max(1e-20))
                    .sqrt();
                    assert!(
                        maximum < 1e-5 && relative < 1e-5,
                        "{frames} kind{kind} abs{maximum} rel{relative}"
                    );
                }
            }
        }
    }
    #[test]
    #[ignore = "requires a physical wgpu compute adapter"]
    fn gpu_batched_loss_matches_cpu_for_distinct_candidates_and_reused_slots() {
        let frames = 8193;
        let mut planner = RealFftPlanner::new();
        let mut plans: Vec<_> = [512, 1024, 4096]
            .into_iter()
            .map(|n| Resolution::new(n, frames, &mut planner))
            .collect();
        let layouts: Vec<_> = plans
            .iter()
            .map(|p| (p.centers.clone(), p.window.clone()))
            .collect();
        let target: Vec<_> = (0..frames)
            .map(|i| {
                (std::f32::consts::TAU * 528.0 * i as f32 / 24000.0).sin() * 0.4
                    + (std::f32::consts::TAU * 1049.0 * i as f32 / 24000.0).sin() * 0.25
            })
            .collect();
        let spectra: Vec<_> = plans
            .iter_mut()
            .map(|p| p.extract(&target, &|| false).unwrap())
            .collect();
        let floor = 1e-4;
        let gpu = crate::gpu::GpuScore::new(frames, &layouts, &spectra, floor, 4).unwrap();
        let signals = [
            vec![0.0; frames],
            target.clone(),
            (0..frames)
                .map(|i| if i == 511 { 0.8 } else { 0.0 })
                .collect(),
            (0..frames)
                .map(|i| (std::f32::consts::TAU * 540.0 * i as f32 / 24000.0).sin() * 0.15)
                .collect(),
        ];
        for order in [[0, 1, 2, 3], [3, 0, 1, 2]] {
            let refs: Vec<_> = order.iter().map(|i| signals[*i].as_slice()).collect();
            let actual = gpu.score(&refs, &|| false).unwrap();
            for (signal, actual) in refs.iter().zip(actual) {
                for (i, plan) in plans.iter_mut().enumerate() {
                    let expected = plan.extract(signal, &|| false).unwrap();
                    let log: f64 = spectra[i]
                        .iter()
                        .zip(&expected)
                        .map(|(a, b)| {
                            (f64::from(*a).max(floor).ln() - f64::from(*b).max(floor).ln()).abs()
                        })
                        .sum();
                    let error: f64 = spectra[i]
                        .iter()
                        .zip(&expected)
                        .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
                        .sum();
                    assert!(
                        (log - actual.log_sum[i]).abs() / (expected.len() as f64) < 1e-4,
                        "log {i}: {log} vs {}",
                        actual.log_sum[i]
                    );
                    assert!(
                        (error - actual.squared_error[i]).abs() < error * 1e-4 + 1e-10,
                        "energy {i}: {error} vs {}",
                        actual.squared_error[i]
                    );
                    if i == 2 {
                        assert!(equivalent_spectra(
                            &[expected],
                            std::slice::from_ref(&actual.largest)
                        ));
                    }
                }
            }
        }
        assert!(gpu.score(&[&target], &|| true).is_err());
        assert!(gpu.score(&[&target], &|| false).is_ok());
    }
    #[test]
    fn weights_and_score_reject_nonfinite_negative_or_inconsistent_values() {
        for bad in [f64::NAN, f64::INFINITY, -1.0] {
            let weights = LossWeights {
                envelope: bad,
                ..Default::default()
            };
            assert!(weights.validate().is_err());
        }
        let zero = LossWeights {
            pitch: 0.0,
            partials: 0.0,
            periodicity: 0.0,
            log_spectral: 0.0,
            convergence: 0.0,
            envelope: 0.0,
            harmonic: 0.0,
            transient: 0.0,
            modulation: 0.0,
            tail: 0.0,
            bands: 0.0,
            inharmonic: None,
        };
        assert!(zero.validate().is_err());
        let mut score = LossBreakdown {
            pitch: 0.0,
            partials: 0.0,
            periodicity: 0.0,
            recording_gain: 1.0,
            total: 0.0,
            log_spectral: 0.0,
            convergence: 0.0,
            envelope: 0.0,
            harmonic: 0.0,
            transient: 0.0,
            modulation: 0.0,
            tail: 0.0,
            bands: 0.0,
            inharmonic: 0.0,
        };
        assert!(score.validate(&LossWeights::default()).is_ok());
        score.total = 1.0;
        assert!(score.validate(&LossWeights::default()).is_err());
    }
    #[test]
    fn sub_octave_energy_is_off_grid_and_legacy_weights_resolve_the_v6_term() {
        let size = 4096;
        let hz = f64::from(RATE) / size as f64;
        let pitch = 523.25;
        let spectrum = |frequencies: &[f64]| {
            let mut average = vec![0.0; size / 2 + 1];
            for f in frequencies {
                average[(f / hz).round() as usize] = 1.0;
            }
            average
        };
        let harmonics: Vec<f64> = (1..=12).map(|k| k as f64 * pitch).collect();
        assert_eq!(off_grid_share(&spectrum(&harmonics), size, pitch), 0.0);
        // A MUL ×½ carrier: every reference harmonic plus (k + ½)·f0 between them.
        let octave_low: Vec<f64> = (1..=24).map(|k| k as f64 * pitch / 2.0).collect();
        let share = off_grid_share(&spectrum(&octave_low), size, pitch);
        assert!((share - 0.5).abs() < 0.05, "{share}");

        let mut legacy = serde_json::to_value(LossWeights::default()).unwrap();
        legacy.as_object_mut().unwrap().remove("inharmonic");
        let weights: LossWeights = serde_json::from_value(legacy).unwrap();
        assert_eq!(weights.inharmonic, None);
        assert_eq!(weights.values()[11], 0.0);
        assert_eq!(
            weights.resolved().inharmonic,
            Some(DEFAULT_INHARMONIC_WEIGHT)
        );
        let explicit = LossWeights {
            inharmonic: Some(0.0),
            ..LossWeights::default()
        };
        assert_eq!(explicit.resolved().inharmonic, Some(0.0));
    }
    /// Would ymfm (about 10x cheaper to render) rank candidates like Nuked, so
    /// it could screen proposals before the Nuked render? Spearman rank
    /// correlation and how many of Nuked's best tenth ymfm puts in its best
    /// third, over perturbations of chip-generated targets.
    #[test]
    #[ignore = "slow probe; run in release with --nocapture"]
    fn ymfm_screening_rank_agreement() {
        use discofm_chip::{Algorithm, Instrument, Value};
        let mut tone = Instrument::default();
        tone.channel.algorithm = Algorithm::new(4).unwrap();
        tone.channel.feedback = Value::new(5).unwrap();
        for (o, (mul, tl, d1r)) in
            tone.operators
                .iter_mut()
                .zip([(2, 28, 6), (1, 4, 4), (3, 34, 8), (1, 8, 5)])
        {
            o.mul = Value::new(mul).unwrap();
            o.tl = Value::new(tl).unwrap();
            o.d1r = Value::new(d1r).unwrap();
            o.sl = Value::new(4).unwrap();
            o.d2r = Value::new(2).unwrap();
            o.rr = Value::new(6).unwrap();
        }
        let mut snare = Instrument::default();
        snare.channel.algorithm = Algorithm::new(4).unwrap();
        snare.channel.feedback = Value::new(7).unwrap();
        for (o, (mul, tl, d1r)) in
            snare
                .operators
                .iter_mut()
                .zip([(15, 0, 16), (1, 6, 17), (1, 20, 22), (1, 6, 15)])
        {
            o.mul = Value::new(mul).unwrap();
            o.tl = Value::new(tl).unwrap();
            o.d1r = Value::new(d1r).unwrap();
            o.sl = Value::new(15).unwrap();
        }
        let mut state = 0x2612_u64;
        let mut next = |n: u64| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) % n
        };
        for (name, note, target) in [("tone", 57, tone), ("snare", 60, snare)] {
            let audition = discofm_engine::RenderRequest {
                note,
                key_on_seconds: 0.45,
                total_seconds: 0.6,
                ..Default::default()
            };
            let audio = discofm_audio::from_native(
                &discofm_engine::render_note(&target, &audition).unwrap(),
                24000,
            )
            .unwrap();
            let input = crate::FitInput {
                reference: std::sync::Arc::new(audio),
                options: discofm_analysis::AnalysisOptions {
                    trim_silence: false,
                    pitch_override_hz: Some(discofm_engine::midi_hz(note).unwrap()),
                    ..Default::default()
                },
                note_on_seconds: 0.0,
                key_off_seconds: 0.45,
                initial: Instrument::default(),
                audition,
                config: crate::FitConfig::default(),
                locks: Default::default(),
                seeds: Vec::new(),
                refine: false,
            };
            let mut objective = Objective::prepare(&input, &|| false).unwrap();
            // Candidates at increasing distance from the truth, as a search sees them.
            let candidates: Vec<Instrument> = (0..240)
                .map(|i| {
                    let mut p = target.clone();
                    for _ in 0..1 + i / 30 {
                        let op = &mut p.operators[next(4) as usize];
                        match next(6) {
                            0 => op.tl = Value::new(next(128) as u8).unwrap(),
                            1 => op.mul = Value::new(next(16) as u8).unwrap(),
                            2 => op.d1r = Value::new(next(32) as u8).unwrap(),
                            3 => op.ar = Value::new(10 + next(22) as u8).unwrap(),
                            4 => p.channel.feedback = Value::new(next(8) as u8).unwrap(),
                            _ => p.channel.algorithm = Algorithm::new(next(8) as u8).unwrap(),
                        }
                    }
                    p
                })
                .collect();
            let started = std::time::Instant::now();
            let nuked: Vec<f64> = objective
                .evaluate_batch(&candidates, &|| false)
                .unwrap()
                .iter()
                .map(|l| l.total)
                .collect();
            let nuked_seconds = started.elapsed().as_secs_f64();
            let settings = RenderSettings {
                context: objective.context.clone(),
                channel: objective.channel,
                crop: objective.crop,
                remove_dc: objective.remove_dc,
                match_level: objective.match_level,
                level: objective.level,
            };
            let mut plans: Vec<Resolution> = objective.plans.iter().map(Resolution::fork).collect();
            let started = std::time::Instant::now();
            let ymfm: Vec<f64> = candidates
                .iter()
                .map(|p| {
                    let native =
                        discofm_engine::render_ymfm_reference(p, &settings.context.audition)
                            .unwrap();
                    let prepared =
                        prepare_native(&settings, native, std::time::Instant::now(), &|| false)
                            .unwrap();
                    objective
                        .score_signal(&mut plans, prepared.signal, prepared.gain, None, &|| false)
                        .unwrap()
                        .total
                })
                .collect();
            let ymfm_seconds = started.elapsed().as_secs_f64();
            let rank = |v: &[f64]| {
                let mut order: Vec<usize> = (0..v.len()).collect();
                order.sort_by(|a, b| v[*a].total_cmp(&v[*b]));
                let mut r = vec![0.0; v.len()];
                for (position, index) in order.into_iter().enumerate() {
                    r[index] = position as f64;
                }
                r
            };
            let (a, b) = (rank(&nuked), rank(&ymfm));
            let n = a.len() as f64;
            let d2: f64 = a.iter().zip(&b).map(|(x, y)| (x - y).powi(2)).sum();
            let spearman = 1.0 - 6.0 * d2 / (n * (n * n - 1.0));
            let top = a.len() / 10;
            let recall = a
                .iter()
                .zip(&b)
                .filter(|(x, y)| (**x as usize) < top && (**y as usize) < a.len() / 3)
                .count();
            let best_nuked = a.iter().position(|x| *x == 0.0).unwrap();
            println!(
                "{name}: spearman {spearman:.3}; Nuked best-10% in ymfm best-third {recall}/{top}; Nuked's best ranks #{} under ymfm; scoring {nuked_seconds:.2} s Nuked ({} workers) vs {ymfm_seconds:.2} s ymfm (1 thread)",
                b[best_nuked] as usize + 1,
                objective.workers
            );
        }
    }
    #[test]
    fn unpitched_note_keys_on_the_lowest_strong_peak() {
        let step = 10.0_f32;
        let frequencies: Vec<f32> = (0..1000).map(|i| i as f32 * step).collect();
        let mut magnitudes = vec![0.01_f32; 1000];
        let mut bump = |center: usize, height: f32| {
            magnitudes[center - 1] = height * 0.5;
            magnitudes[center] = height;
            magnitudes[center + 1] = height * 0.5;
        };
        bump(6, 0.6); // 60 Hz body, -4.4 dB
        bump(30, 1.0); // 300 Hz loudest partial
        bump(1, 5.0); // 10 Hz rumble, outside 20..4000 Hz
        let hz = body_hz(&frequencies, &magnitudes, None).unwrap();
        assert!((hz - 60.0).abs() < 1e-3, "{hz}");
        assert_eq!(body_hz(&frequencies, &magnitudes, Some(61.5)), Some(61.5));
        assert!((body_hz(&frequencies, &magnitudes, Some(90.0)).unwrap() - 60.0).abs() < 1e-3);
        magnitudes[6] = 0.3;
        let hz = body_hz(&frequencies, &magnitudes, None).unwrap();
        assert!((hz - 300.0).abs() < 1e-3, "{hz}");
        assert_eq!(body_hz(&frequencies, &[0.0; 1000], None), None);
        assert_eq!(body_hz(&[], &[], Some(100.0)), None);
    }
    #[test]
    fn legacy_weights_lack_bands_and_band_energy_averages_noise_bins() {
        let mut legacy = serde_json::to_value(LossWeights::default()).unwrap();
        legacy.as_object_mut().unwrap().remove("bands");
        let weights: LossWeights = serde_json::from_value(legacy).unwrap();
        assert_eq!(weights.bands, 0.0);
        assert!(LossWeights::unpitched().validate().is_ok());
        let size = 4096;
        let bins = size / 2 + 1;
        let mut spectrum = vec![0.0_f32; bins * 2];
        spectrum[bins + 2000] = 1.0; // ~11.7 kHz, above the last band
        spectrum[bins + 18] = 2.0; // ~105 Hz
        let bands = band_energies(&spectrum, size);
        assert_eq!(bands.len(), BANDS * 2);
        assert!(bands[..BANDS].iter().all(|v| *v == 0.0));
        assert_eq!(bands[BANDS..].iter().sum::<f64>(), 4.0);
    }
    #[test]
    fn thirty_second_spectral_layout_has_continuous_coverage_at_every_resolution() {
        let mut planner = RealFftPlanner::new();
        for size in [512, 1024, 4096] {
            let layout = Resolution::new(size, 720000, &mut planner);
            assert_eq!(layout.centers.first(), Some(&0));
            assert_eq!(layout.centers.last(), Some(&719999));
            assert!(layout.centers.windows(2).all(|w| w[1] - w[0] <= size / 4));
        }
    }
}
