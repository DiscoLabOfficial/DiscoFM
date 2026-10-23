//! Integration checks and independent ymfm/recorded-audio comparisons.
#![forbid(unsafe_code)]
mod audio;
pub use audio::{AudioComparison, compare_audio, compare_ymfm};
use discofm_chip::Instrument;
use discofm_engine::{
    EngineError, NukedChip, RenderRequest, TICKS_PER_FRAME, direct_upstream_pins, render_trace,
    schedule_note,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Comparison {
    pub ticks: usize,
    pub mismatched_ticks: usize,
    pub max_absolute_error: i32,
    pub rms_error: f64,
    pub pin_hash_fnv1a: String,
}
pub fn compare_pins(actual: &[[i16; 2]], expected: &[[i16; 2]]) -> Result<Comparison, EngineError> {
    if actual.len() != expected.len() || actual.is_empty() {
        return Err(EngineError::InvalidTrace);
    }
    let mut mismatches = 0;
    let mut max_error = 0;
    let mut squares = 0.0;
    let mut hash = 0xcbf29ce484222325_u64;
    for (a, b) in actual.iter().zip(expected) {
        mismatches += usize::from(a != b);
        for channel in 0..2 {
            let delta = i32::from(a[channel]) - i32::from(b[channel]);
            max_error = max_error.max(delta.abs());
            squares += f64::from(delta).powi(2);
            for byte in a[channel].to_le_bytes() {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
            }
        }
    }
    Ok(Comparison {
        ticks: actual.len(),
        mismatched_ticks: mismatches,
        max_absolute_error: max_error,
        rms_error: (squares / (actual.len() * 2) as f64).sqrt(),
        pin_hash_fnv1a: format!("{hash:016x}"),
    })
}
pub fn compare_note(
    patch: &Instrument,
    request: &RenderRequest,
) -> Result<Comparison, EngineError> {
    let schedule = schedule_note(patch, request)?;
    let ticks = schedule.frames * TICKS_PER_FRAME;
    let mut chip = NukedChip::new(request.profile)?;
    let actual = render_trace(&mut chip, &schedule.events, schedule.capture_start, ticks)?;
    let expected = direct_upstream_pins(
        request.profile,
        &schedule.events,
        schedule.capture_start,
        ticks,
    )?;
    compare_pins(&actual, &expected)
}
