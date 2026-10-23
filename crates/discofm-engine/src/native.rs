//! The only Rust module allowed to call native code. Chip pointers never escape.
use crate::{
    ChannelSnapshot,
    diagnostics::{channel_index, decode_snapshot},
};
use crate::{ChipBackend, ChipProfile, EngineError, PortEvent, validate_trace};
use discofm_chip::Channel;
use std::{ffi::c_void, ptr::NonNull, sync::Mutex};

static ORACLE_LOCK: Mutex<()> = Mutex::new(());

unsafe extern "C" {
    fn dfm_ymfm_render(
        profile: u32,
        events: *const PortEvent,
        count: usize,
        start: u64,
        frames: usize,
        output: *mut i32,
    ) -> i32;
    fn dfm_create() -> *mut c_void;
    fn dfm_destroy(chip: *mut c_void);
    fn dfm_reset(chip: *mut c_void);
    fn dfm_write(chip: *mut c_void, profile: u32, port: u32, data: u8);
    fn dfm_status(chip: *mut c_void, profile: u32) -> u8;
    fn dfm_clock(chip: *mut c_void, profile: u32, output: *mut i16, ticks: usize);
    fn dfm_snapshot(chip: *mut c_void, channel: u32, words: *mut u32);
    fn dfm_oracle(
        profile: u32,
        events: *const PortEvent,
        count: usize,
        capture_start: u64,
        capture_ticks: usize,
        output: *mut i16,
    );
}

/// Independent sample-level ymfm renderer. No timers/CSM are scheduled by the
/// note API. This is not a cycle-level replacement for the production backend.
pub fn render_ymfm_reference(
    patch: &discofm_chip::Instrument,
    request: &crate::RenderRequest,
) -> Result<crate::NativeAudio, EngineError> {
    let schedule = crate::schedule_note(patch, request)?;
    let profile = mode(request.profile)?;
    validate_trace(
        &schedule.events,
        schedule.capture_start,
        schedule.frames * crate::TICKS_PER_FRAME,
    )?;
    if !schedule.capture_start.is_multiple_of(24) {
        return Err(EngineError::InvalidTrace);
    }
    let mut samples = vec![[0_i32; 2]; schedule.frames];
    // SAFETY: repr(C) events are validated; output has exactly frames stereo
    // i32 pairs. C++ owns local interface/chip and catches every exception.
    let ok = unsafe {
        dfm_ymfm_render(
            profile,
            schedule.events.as_ptr(),
            schedule.events.len(),
            schedule.capture_start,
            schedule.frames,
            samples.as_mut_ptr().cast(),
        )
    };
    if ok == 0 {
        return Err(EngineError::Allocation);
    }
    // Match the fixed DAC duty-cycle integration, including ymfm's 64/65
    // YM2612 compensation. No signal-dependent gain or fitting is applied.
    let duty = if profile == 1 {
        0.75 * 65.0 / 64.0
    } else {
        0.75
    };
    Ok(crate::NativeAudio {
        frames: samples
            .into_iter()
            .map(|f| [f[0] as f32 / 32768.0 * duty, f[1] as f32 / 32768.0 * duty])
            .collect(),
        sample_rate: request.clock.native_hz(),
        schedule,
    })
}

fn mode(profile: ChipProfile) -> Result<u32, EngineError> {
    match profile {
        ChipProfile::Ym2612Discrete => Ok(1),
        ChipProfile::Ym3438 => Ok(0),
        ChipProfile::RawDiagnostic => Err(EngineError::UnsupportedRawProfile),
    }
}

/// Owned opaque native chip. NonNull makes it !Send/!Sync; create it on its worker.
/// The generated production core stores the profile in each instance.
/// Each worker owns its chip; only the pristine oracle needs a global lock.
pub struct NukedChip {
    pointer: NonNull<c_void>,
    mode: u32,
}
impl NukedChip {
    pub fn new(profile: ChipProfile) -> Result<Self, EngineError> {
        let mode = mode(profile)?;
        // SAFETY: C allocates and initializes a complete chip, checked for null.
        let pointer = NonNull::new(unsafe { dfm_create() }).ok_or(EngineError::Allocation)?;
        Ok(Self { pointer, mode })
    }
    /// Read actual register, envelope and phase state without advancing the chip.
    pub fn snapshot(&self, channel: Channel) -> Result<ChannelSnapshot, EngineError> {
        let mut words = [0_u32; 82];
        // SAFETY: valid owned chip, validated channel 0..5, and the C shim writes
        // exactly 82 u32 words. Snapshot only reads state; instance is uniquely owned.
        unsafe {
            dfm_snapshot(
                self.pointer.as_ptr(),
                channel_index(channel),
                words.as_mut_ptr(),
            )
        };
        decode_snapshot(&words)
    }
}
impl ChipBackend for NukedChip {
    fn reset(&mut self) -> Result<(), EngineError> {
        // SAFETY: uniquely owned live C allocation; instance-local native state.
        unsafe { dfm_reset(self.pointer.as_ptr()) };
        Ok(())
    }
    fn write_port(&mut self, port: u8, data: u8) -> Result<(), EngineError> {
        if port > 3 {
            return Err(EngineError::InvalidPort(port));
        }
        // SAFETY: valid chip and checked hardware port; mode belongs to this instance.
        unsafe { dfm_write(self.pointer.as_ptr(), self.mode, port.into(), data) };
        Ok(())
    }
    fn clock_into(&mut self, output: &mut [[i16; 2]]) -> Result<(), EngineError> {
        // SAFETY: Rust arrays are contiguous stereo i16 pairs; C writes exactly len.
        unsafe {
            dfm_clock(
                self.pointer.as_ptr(),
                self.mode,
                output.as_mut_ptr().cast(),
                output.len(),
            )
        };
        Ok(())
    }
    fn status(&mut self) -> Result<u8, EngineError> {
        // SAFETY: valid owned chip, mode and uniquely borrowed instance.
        Ok(unsafe { dfm_status(self.pointer.as_ptr(), self.mode) })
    }
}
impl Drop for NukedChip {
    fn drop(&mut self) {
        // SAFETY: this allocation is uniquely owned and freed exactly once here.
        unsafe { dfm_destroy(self.pointer.as_ptr()) };
    }
}

/// Integration oracle: a separate C loop calls the unmodified upstream API.
pub fn direct_upstream_pins(
    profile: ChipProfile,
    events: &[PortEvent],
    start: u64,
    ticks: usize,
) -> Result<Vec<[i16; 2]>, EngineError> {
    let mode = mode(profile)?;
    validate_trace(events, start, ticks)?;
    let mut output = vec![[0; 2]; ticks];
    let _guard = ORACLE_LOCK
        .lock()
        .map_err(|_| EngineError::BackendPoisoned)?;
    // SAFETY: validated ordered events, repr(C) layout, bounded stereo output.
    unsafe {
        dfm_oracle(
            mode,
            events.as_ptr(),
            events.len(),
            start,
            ticks,
            output.as_mut_ptr().cast(),
        )
    };
    Ok(output)
}
