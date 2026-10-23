use crate::AudioError;
use std::{
    collections::VecDeque,
    f64::consts::PI,
    sync::{Arc, Mutex, OnceLock},
};

const PHASES: usize = 2048;
type KernelCache = Mutex<Vec<((u64, u32), Arc<Vec<f64>>)>>;
static KERNELS: OnceLock<KernelCache> = OnceLock::new();

pub(crate) struct Kernel {
    cutoff: f64,
    radius: f64,
    pub support: i64,
    coefficients: Option<Arc<Vec<f64>>>,
}
impl Kernel {
    pub fn new(source: f64, target: u32) -> Self {
        let cutoff = (f64::from(target) / source).min(1.0) * 0.94;
        let radius = 32.0 / cutoff;
        let mut kernel = Self {
            cutoff,
            radius,
            support: radius.ceil() as i64,
            coefficients: None,
        };
        // Bounded, shared polyphase tables: interpolation removes trigonometry
        // from the audio loop without changing cutoff, support, or time origin.
        // Unusual extreme conversion ratios retain the analytic implementation.
        if kernel.support <= 512 {
            let identity = (source.to_bits(), target);
            let cache = KERNELS.get_or_init(|| Mutex::new(Vec::new()));
            if let Some(table) = cache
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .iter()
                .find(|(key, _)| *key == identity)
                .map(|(_, v)| Arc::clone(v))
            {
                kernel.coefficients = Some(table);
                return kernel;
            }
            let mut table = Vec::with_capacity((PHASES + 1) * (2 * kernel.support as usize + 1));
            for phase in 0..=PHASES {
                let begin = table.len();
                for tap in -kernel.support..=kernel.support {
                    table.push(kernel.weight(phase as f64 / PHASES as f64 - tap as f64));
                }
                let sum = table[begin..].iter().sum::<f64>();
                for weight in &mut table[begin..] {
                    *weight /= sum;
                }
            }
            let table = Arc::new(table);
            let mut cache = cache.lock().unwrap_or_else(|p| p.into_inner());
            if cache.len() == 4 {
                cache.remove(0);
            }
            cache.push((identity, Arc::clone(&table)));
            kernel.coefficients = Some(table);
        }
        kernel
    }
    fn weight(&self, distance: f64) -> f64 {
        if distance.abs() >= self.radius {
            return 0.0;
        }
        let x = PI * distance * self.cutoff;
        let sinc = if x.abs() < 1e-12 { 1.0 } else { x.sin() / x };
        let window = 0.42
            + 0.5 * (PI * distance / self.radius).cos()
            + 0.08 * (2.0 * PI * distance / self.radius).cos();
        self.cutoff * sinc * window
    }
    /// One channel of `sample`, with the same weights and summation order, so
    /// channel 0 of a duplicated stereo signal is reproduced bit for bit.
    pub fn sample_mono(&self, position: f64, samples: &[f32]) -> f32 {
        let center = position.floor() as i64;
        let last = samples.len() as i64 - 1;
        let at = |index: i64| f64::from(samples[index.clamp(0, last) as usize]);
        if let Some(table) = &self.coefficients {
            let phase = (position - center as f64) * PHASES as f64;
            let index = (phase.floor() as usize).min(PHASES - 1);
            let fraction = phase - index as f64;
            let width = 2 * self.support as usize + 1;
            let a = &table[index * width..(index + 1) * width];
            let b = &table[(index + 1) * width..(index + 2) * width];
            let first = center - self.support;
            let mut sum = 0.0;
            if first >= 0 && first + width as i64 <= samples.len() as i64 {
                let taps = &samples[first as usize..first as usize + width];
                for ((a, b), value) in a.iter().zip(b).zip(taps) {
                    sum += f64::from(*value) * (a + (b - a) * fraction);
                }
            } else {
                for (i, (a, b)) in a.iter().zip(b).enumerate() {
                    sum += at(first + i as i64) * (a + (b - a) * fraction);
                }
            }
            return sum as f32;
        }
        let mut sum = 0.0;
        let mut weights = 0.0;
        for tap in -self.support..=self.support {
            let distance = position - (center + tap) as f64;
            if distance.abs() >= self.radius {
                continue;
            }
            let weight = self.weight(distance);
            sum += at(center + tap) * weight;
            weights += weight;
        }
        (sum / weights) as f32
    }
    pub fn sample(&self, position: f64, mut source: impl FnMut(i64) -> [f32; 2]) -> [f32; 2] {
        let center = position.floor() as i64;
        if let Some(table) = &self.coefficients {
            let phase = (position - center as f64) * PHASES as f64;
            let index = (phase.floor() as usize).min(PHASES - 1);
            let fraction = phase - index as f64;
            let width = 2 * self.support as usize + 1;
            let a = &table[index * width..(index + 1) * width];
            let b = &table[(index + 1) * width..(index + 2) * width];
            let mut sum = [0.0; 2];
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                let weight = a + (b - a) * fraction;
                let frame = source(center + i as i64 - self.support);
                sum[0] += f64::from(frame[0]) * weight;
                sum[1] += f64::from(frame[1]) * weight;
            }
            return [sum[0] as f32, sum[1] as f32];
        }
        let mut sum = [0.0; 2];
        let mut weights = 0.0;
        for tap in -self.support..=self.support {
            let distance = position - (center + tap) as f64;
            if distance.abs() >= self.radius {
                continue;
            }
            let weight = self.weight(distance);
            let frame = source(center + tap);
            sum[0] += f64::from(frame[0]) * weight;
            sum[1] += f64::from(frame[1]) * weight;
            weights += weight;
        }
        [(sum[0] / weights) as f32, (sum[1] / weights) as f32]
    }
}

/// Stateful version of the offline sinc converter. Retains filter history and
/// lookahead; only `finish` extends the true final endpoint. Runs on the worker.
pub struct StreamingResampler {
    source_hz: f64,
    target_hz: u32,
    kernel: Kernel,
    frames: VecDeque<[f32; 2]>,
    start: u64,
    total: u64,
    produced: u64,
    finished: bool,
    capacity: usize,
}
impl StreamingResampler {
    pub fn new(source_hz: f64, target_hz: u32) -> Result<Self, AudioError> {
        if !source_hz.is_finite()
            || !(1.0..=f64::from(u32::MAX)).contains(&source_hz)
            || !(8000..=192000).contains(&target_hz)
        {
            return Err(AudioError::SampleRate);
        }
        let kernel = Kernel::new(source_hz, target_hz);
        let capacity = 4096 + 2 * kernel.support as usize;
        Ok(Self {
            source_hz,
            target_hz,
            kernel,
            frames: VecDeque::with_capacity(capacity),
            start: 0,
            total: 0,
            produced: 0,
            finished: false,
            capacity,
        })
    }
    pub fn reset(&mut self) {
        self.frames.clear();
        self.start = 0;
        self.total = 0;
        self.produced = 0;
        self.finished = false;
    }
    pub fn pending_frames(&self) -> usize {
        self.frames.len()
    }
    pub fn push(&mut self, frames: &[[f32; 2]]) -> Result<(), AudioError> {
        if self.finished || frames.len() > self.capacity - self.frames.len() {
            return Err(AudioError::StreamBuffer);
        }
        if frames.iter().flatten().any(|value| !value.is_finite()) {
            return Err(AudioError::InvalidBuffer);
        }
        self.frames.extend(frames);
        self.total += frames.len() as u64;
        Ok(())
    }
    pub fn finish(&mut self) {
        self.finished = true;
    }
    pub fn read_into(&mut self, output: &mut [[f32; 2]]) -> usize {
        let mut written = 0;
        while written < output.len() && self.total != 0 {
            let position = self.produced as f64 * self.source_hz / f64::from(self.target_hz);
            let center = position.floor() as i64;
            if self.finished {
                let count =
                    (self.total as f64 * f64::from(self.target_hz) / self.source_hz).round() as u64;
                if self.produced >= count {
                    break;
                }
            } else if center + self.kernel.support >= self.total as i64 {
                break;
            }
            output[written] = self.kernel.sample(position, |index| {
                let index = index.clamp(0, self.total as i64 - 1) as u64;
                self.frames[(index - self.start) as usize]
            });
            written += 1;
            self.produced += 1;
            let next =
                (self.produced as f64 * self.source_hz / f64::from(self.target_hz)).floor() as i64;
            let keep = (next - self.kernel.support).max(0) as u64;
            while self.start < keep && self.frames.len() > 1 {
                self.frames.pop_front();
                self.start += 1;
            }
        }
        written
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_polyphase_matches_analytic_sinc_across_rates_and_phases() {
        for (source, target) in [
            (53267.03869047619, 24000),
            (44100.0, 48000),
            (192000.0, 8000),
            (8000.0, 192000),
        ] {
            let cached = Kernel::new(source, target);
            let analytic = Kernel {
                cutoff: cached.cutoff,
                radius: cached.radius,
                support: cached.support,
                coefficients: None,
            };
            let wave = |i: i64| {
                let value = (i as f64 * 0.713).sin() as f32;
                [value, if i == 4 { 1.0 } else { value * 0.2 }]
            };
            let mut maximum = 0.0_f32;
            for i in 0..1000 {
                let position = 0.017 + i as f64 * 0.15937;
                let a = cached.sample(position, wave);
                let b = analytic.sample(position, wave);
                for c in 0..2 {
                    maximum = maximum.max((a[c] - b[c]).abs());
                }
            }
            assert!(maximum < 5e-7, "{source}->{target}: {maximum}");
        }
    }
}
