//! Hardware synthesis through unmodified Nuked-OPN2. All DSP comes from the core.
//! One backend tick = 6 master clocks; a native frame integrates 24 pin ticks.
mod diagnostics;
mod live;
mod native;
pub use diagnostics::{
    ChannelSnapshot, DiagnosticRender, EnvelopePoint, EnvelopeStage, OperatorSnapshot,
    inspect_instrument, render_diagnostics, render_note_cancellable,
};
pub use live::{LiveConfig, LiveRenderer};
pub use native::{NukedChip, direct_upstream_pins, render_ymfm_reference};

use discofm_chip::{
    Channel, Frequency, Instrument, ParameterError, RegisterWrite, Value, encode_instrument,
    frequency_writes, key_write,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const BACKEND_COMMIT: &str = "335747d78cb0abbc3b55b004e62dad9763140115";
pub const YMFM_COMMIT: &str = "81aec25ccbb98f4873a255f7551ac4dadac59b4a";
pub const MASTER_CLOCKS_PER_TICK: u32 = 6;
pub const TICKS_PER_FRAME: usize = 24;
pub const REGISTER_TICKS: u64 = 48;
const MAX_TICKS: usize = 40_000_000;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("render cancelled")]
    Cancelled,
    #[error(transparent)]
    Parameter(#[from] ParameterError),
    #[error("Nuked-OPN2 does not expose a RAW_DIAGNOSTIC profile; capture native pins instead")]
    UnsupportedRawProfile,
    #[error("native chip allocation failed")]
    Allocation,
    #[error("native backend lock poisoned")]
    BackendPoisoned,
    #[error("invalid hardware port {0}; expected 0..3")]
    InvalidPort(u8),
    #[error("invalid render request: {0}")]
    InvalidRequest(&'static str),
    #[error("invalid or oversized register trace")]
    InvalidTrace,
    #[error("frequency is outside representable FNUM/BLOCK range")]
    FrequencyRange,
    #[error("native backend returned invalid diagnostic state")]
    BackendState,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ChipProfile {
    #[default]
    Ym2612Discrete,
    Ym3438,
    RawDiagnostic,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClockConfig {
    #[default]
    Ntsc,
    Pal,
}
impl ClockConfig {
    /// Nominal console oscillator divided by seven, kept as a fractional rate.
    pub fn master_hz(self) -> f64 {
        match self {
            Self::Ntsc => 53_693_175.0 / 7.0,
            Self::Pal => 53_203_424.0 / 7.0,
        }
    }
    pub fn tick_hz(self) -> f64 {
        self.master_hz() / f64::from(MASTER_CLOCKS_PER_TICK)
    }
    pub fn native_hz(self) -> f64 {
        self.tick_hz() / TICKS_PER_FRAME as f64
    }
}

/// Replaceable cycle-level backend. Never use a lower-fidelity preview path.
pub trait ChipBackend {
    fn reset(&mut self) -> Result<(), EngineError>;
    fn write_port(&mut self, port: u8, data: u8) -> Result<(), EngineError>;
    fn clock_into(&mut self, output: &mut [[i16; 2]]) -> Result<(), EngineError>;
    fn status(&mut self) -> Result<u8, EngineError>;
}

/// A port write immediately before the specified internal tick (six master clocks).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortEvent {
    pub tick: u64,
    pub port: u32,
    pub data: u8,
}

pub fn validate_trace(events: &[PortEvent], start: u64, ticks: usize) -> Result<(), EngineError> {
    let end = start
        .checked_add(ticks as u64)
        .ok_or(EngineError::InvalidTrace)?;
    if ticks > MAX_TICKS
        || end > MAX_TICKS as u64
        || events.iter().any(|e| e.port > 3 || e.tick >= end)
        || events.windows(2).any(|w| w[0].tick >= w[1].tick)
    {
        return Err(EngineError::InvalidTrace);
    }
    Ok(())
}

/// Replays a trace from reset, capturing exact stereo MOL/MOR pin values.
/// Small clock batches release the global profile lock between native calls.
pub fn render_trace(
    backend: &mut impl ChipBackend,
    events: &[PortEvent],
    start: u64,
    ticks: usize,
) -> Result<Vec<[i16; 2]>, EngineError> {
    validate_trace(events, start, ticks)?;
    backend.reset()?;
    let end = start + ticks as u64;
    let mut output = vec![[0; 2]; ticks];
    let mut scratch = [[0; 2]; 4096];
    let mut tick = 0;
    let mut event = 0;
    while tick < end {
        if event < events.len() && events[event].tick == tick {
            backend.write_port(events[event].port as u8, events[event].data)?;
            event += 1;
        }
        let next = events.get(event).map_or(end, |e| e.tick).min(end);
        let boundary = if tick < start { next.min(start) } else { next };
        let count = (boundary - tick).min(scratch.len() as u64) as usize;
        if tick < start {
            backend.clock_into(&mut scratch[..count])?;
        } else {
            let offset = (tick - start) as usize;
            backend.clock_into(&mut output[offset..offset + count])?;
        }
        tick += count as u64;
    }
    Ok(output)
}

/// Frequency for MUL=1 and DT=0, derived from the hardware phase accumulator.
pub fn frequency_hz(frequency: Frequency, clock: ClockConfig) -> f64 {
    let increment = (u32::from(frequency.fnum()) << frequency.block()) >> 1;
    f64::from(increment) * clock.native_hz() / 1_048_576.0
}

pub fn frequency_for_hz(hz: f64, clock: ClockConfig) -> Result<Frequency, EngineError> {
    if !hz.is_finite() || hz <= 0.0 {
        return Err(EngineError::FrequencyRange);
    }
    // Lowest block with a fitting F-number maximizes register resolution.
    for block in 0..=7 {
        let raw = hz * 1_048_576.0 / (clock.native_hz() * 2.0_f64.powi(block - 1));
        let rounded = if block == 0 {
            (raw / 2.0).round() * 2.0
        } else {
            raw.round()
        };
        if (1.0..=2047.0).contains(&rounded) {
            return Ok(Frequency::new(rounded as u16, block as u8)?);
        }
    }
    Err(EngineError::FrequencyRange)
}

pub fn midi_hz(note: u8) -> Result<f64, EngineError> {
    if note > 127 {
        return Err(EngineError::InvalidRequest("MIDI note must be 0..127"));
    }
    Ok(440.0 * 2.0_f64.powf((f64::from(note) - 69.0) / 12.0))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderRequest {
    pub note: u8,
    #[serde(default, skip_serializing_if = "zero_tuning")]
    pub tuning_cents: f64,
    pub velocity: Value<127>,
    pub key_on_seconds: f64,
    pub total_seconds: f64,
    pub channel: Channel,
    pub profile: ChipProfile,
    pub clock: ClockConfig,
}
fn zero_tuning(value: &f64) -> bool {
    *value == 0.0
}
impl Default for RenderRequest {
    fn default() -> Self {
        Self {
            note: 69,
            tuning_cents: 0.0,
            velocity: Value::new(127).expect("valid constant"),
            key_on_seconds: 0.5,
            total_seconds: 1.0,
            channel: Channel::ONE,
            profile: ChipProfile::default(),
            clock: ClockConfig::default(),
        }
    }
}
impl RenderRequest {
    pub fn pitch_hz(&self) -> Result<f64, EngineError> {
        if !self.tuning_cents.is_finite() || self.tuning_cents.abs() > 100.0 {
            return Err(EngineError::InvalidRequest(
                "tuning must be finite and within ±100 cents",
            ));
        }
        Ok(midi_hz(self.note)? * 2.0_f64.powf(self.tuning_cents / 1200.0))
    }
    pub fn validate(&self) -> Result<(), EngineError> {
        frequency_for_hz(self.pitch_hz()?, self.clock)?;
        if !self.total_seconds.is_finite()
            || !(0.0..=30.0).contains(&self.total_seconds)
            || self.total_seconds == 0.0
            || !self.key_on_seconds.is_finite()
            || self.key_on_seconds < REGISTER_TICKS as f64 / self.clock.tick_hz()
            || self.key_on_seconds > self.total_seconds
        {
            return Err(EngineError::InvalidRequest(
                "require 0 < key-on <= total <= 30 seconds, with key-on at least 48 chip ticks",
            ));
        }
        if self.profile == ChipProfile::RawDiagnostic {
            return Err(EngineError::UnsupportedRawProfile);
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct NoteSchedule {
    pub events: Vec<PortEvent>,
    pub capture_start: u64,
    pub frames: usize,
    pub key_off_tick: u64,
    pub frequency: Frequency,
}
fn schedule_write(events: &mut Vec<PortEvent>, tick: u64, write: RegisterWrite) {
    events.push(PortEvent {
        tick,
        port: u32::from(write.bank) * 2,
        data: write.address,
    });
    events.push(PortEvent {
        tick: tick + 2,
        port: u32::from(write.bank) * 2 + 1,
        data: write.data,
    });
}
pub fn schedule_note(
    patch: &Instrument,
    request: &RenderRequest,
) -> Result<NoteSchedule, EngineError> {
    request.validate()?;
    let frequency = frequency_for_hz(request.pitch_hz()?, request.clock)?;
    let mut writes = Vec::new();
    for number in 1..=6 {
        let channel = Channel::new(number)?;
        writes.push(key_write(channel, Value::new(0)?));
        writes.push(RegisterWrite::new(
            channel.bank(),
            0xb4 + channel.offset(),
            0,
        ));
    }
    writes.extend(encode_instrument(patch, request.channel, request.velocity)?);
    writes.extend(frequency_writes(request.channel, frequency));
    let mut events = Vec::new();
    let mut tick = 96;
    for write in writes {
        schedule_write(&mut events, tick, write);
        tick += REGISTER_TICKS;
    }
    tick += 96;
    let capture_start = tick;
    schedule_write(
        &mut events,
        tick,
        key_write(request.channel, Value::new(15)?),
    );
    let frames = (request.total_seconds * request.clock.native_hz()).ceil() as usize;
    let key_off_tick = tick + (request.key_on_seconds * request.clock.tick_hz()).round() as u64;
    let end = tick + (frames * TICKS_PER_FRAME) as u64;
    // A key-off at/after the end has no captured release; omit out-of-range writes.
    if key_off_tick + 2 < end {
        schedule_write(
            &mut events,
            key_off_tick,
            key_write(request.channel, Value::new(0)?),
        );
    }
    Ok(NoteSchedule {
        events,
        capture_start,
        frames,
        key_off_tick,
        frequency,
    })
}

#[derive(Debug, Clone)]
pub struct NativeAudio {
    pub frames: Vec<[f32; 2]>,
    pub sample_rate: f64,
    pub schedule: NoteSchedule,
}
/// Integrate the time-multiplexed pins over one complete operator frame.
/// Fixed gain 1/256 preserves profile differences and the backend's residual DC.
pub fn integrate_pins(pins: &[[i16; 2]]) -> Vec<[f32; 2]> {
    pins.as_chunks::<TICKS_PER_FRAME>()
        .0
        .iter()
        .map(|frame| {
            let sums = frame.iter().fold([0_i32; 2], |mut sum, p| {
                sum[0] += i32::from(p[0]);
                sum[1] += i32::from(p[1]);
                sum
            });
            [
                sums[0] as f32 / (24.0 * 256.0),
                sums[1] as f32 / (24.0 * 256.0),
            ]
        })
        .collect()
}
pub fn render_note(
    patch: &Instrument,
    request: &RenderRequest,
) -> Result<NativeAudio, EngineError> {
    let schedule = schedule_note(patch, request)?;
    let mut chip = NukedChip::new(request.profile)?;
    let pins = render_trace(
        &mut chip,
        &schedule.events,
        schedule.capture_start,
        schedule.frames * TICKS_PER_FRAME,
    )?;
    Ok(NativeAudio {
        frames: integrate_pins(&pins),
        sample_rate: request.clock.native_hz(),
        schedule,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clock_and_frequency_ranges() {
        assert!((ClockConfig::Ntsc.native_hz() - 53267.0387).abs() < 0.001);
        assert!(ClockConfig::Pal.native_hz() < ClockConfig::Ntsc.native_hz());
        for clock in [ClockConfig::Ntsc, ClockConfig::Pal] {
            for note in [0, 12, 60, 69, 96, 115] {
                let target = midi_hz(note).unwrap();
                let frequency = frequency_for_hz(target, clock).unwrap();
                let phase_quantum = clock.native_hz() / 1_048_576.0
                    * 2.0_f64.powi(i32::from(frequency.block()).saturating_sub(1).max(0));
                assert!(
                    (frequency_hz(frequency, clock) - target).abs() <= phase_quantum / 2.0 + 1e-9,
                    "{clock:?} note {note}: nearest representable pitch must be within half a phase increment"
                );
            }
        }
        assert!(frequency_for_hz(f64::NAN, ClockConfig::Ntsc).is_err());
        assert!(frequency_for_hz(1e9, ClockConfig::Ntsc).is_err());
        assert!(frequency_for_hz(midi_hz(127).unwrap(), ClockConfig::Ntsc).is_err());
    }
    #[test]
    fn invalid_requests_fail_before_allocation() {
        for seconds in [0.0, -1.0, f64::NAN, f64::INFINITY, 31.0] {
            assert!(
                RenderRequest {
                    total_seconds: seconds,
                    ..RenderRequest::default()
                }
                .validate()
                .is_err()
            );
        }
        assert!(NukedChip::new(ChipProfile::RawDiagnostic).is_err());
        assert!(
            validate_trace(
                &[PortEvent {
                    tick: 0,
                    port: 4,
                    data: 0
                }],
                0,
                24
            )
            .is_err()
        );
        assert!(validate_trace(&[], u64::MAX, 24).is_err());
    }
    #[test]
    fn reset_and_busy_sequence() {
        let mut chip = NukedChip::new(ChipProfile::Ym2612Discrete).unwrap();
        chip.write_port(0, 0x22).unwrap();
        chip.clock_into(&mut [[0; 2]; 2]).unwrap();
        chip.write_port(1, 8).unwrap();
        chip.clock_into(&mut [[0; 2]; 2]).unwrap();
        assert_ne!(chip.status().unwrap() & 0x80, 0);
        chip.clock_into(&mut [[0; 2]; 46]).unwrap();
        assert_eq!(chip.status().unwrap() & 0x80, 0);
        chip.reset().unwrap();
        let mut first = [[0; 2]; 96];
        chip.clock_into(&mut first).unwrap();
        chip.reset().unwrap();
        let mut second = [[0; 2]; 96];
        chip.clock_into(&mut second).unwrap();
        assert_eq!(first, second);
    }
}
