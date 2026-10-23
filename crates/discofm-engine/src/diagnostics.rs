//! Read-only views of the actual core. These never approximate envelope behavior.
use crate::{EngineError, NukedChip, RenderRequest, render_trace, schedule_note};
use discofm_chip::{
    Algorithm, Channel, ChannelParameters, Frequency, GlobalParameters, Instrument, Operator,
    SsgEg, Value,
};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum EnvelopeStage {
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct OperatorSnapshot {
    /// Actual decoded hardware settings, including velocity-adjusted TL.
    pub registers: Operator,
    /// Raw generator attenuation before SSG inversion, AM and TL (0..1023).
    pub envelope_attenuation: u16,
    /// Effective generator output after SSG inversion, AM and TL (0..1023).
    pub output_attenuation: u16,
    pub stage: EnvelopeStage,
    pub phase_increment: u32,
    pub phase: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelSnapshot {
    pub operators: [OperatorSnapshot; 4],
    pub channel: ChannelParameters,
    pub global: GlobalParameters,
    pub frequency: Frequency,
    /// S1/S2/S3 order, matching the model, rather than native A8/A9/AA order.
    pub special_frequencies: [Frequency; 3],
    pub channel3_mode: u8,
    pub dac_enabled: bool,
}
fn byte(raw: u32) -> Result<u8, EngineError> {
    u8::try_from(raw).map_err(|_| EngineError::BackendState)
}
fn value<const MAX: u8>(raw: u32) -> Result<Value<MAX>, EngineError> {
    Ok(Value::new(byte(raw)?)?)
}
fn frequency(fnum: u32, block: u32) -> Result<Frequency, EngineError> {
    let fnum = u16::try_from(fnum).map_err(|_| EngineError::BackendState)?;
    Ok(Frequency::new(fnum, byte(block)?)?)
}
fn operator(words: &[u32]) -> Result<OperatorSnapshot, EngineError> {
    let multi = match words[1] {
        1 => 0,
        raw if raw <= 30 && raw % 2 == 0 => raw / 2,
        _ => return Err(EngineError::BackendState),
    };
    let sl = if words[7] == 31 { 15 } else { words[7] };
    let stage = match words[13] {
        0 => EnvelopeStage::Attack,
        1 => EnvelopeStage::Decay,
        2 => EnvelopeStage::Sustain,
        3 => EnvelopeStage::Release,
        _ => return Err(EngineError::BackendState),
    };
    if words[11] > 1023 || words[12] > 1023 || words[9] > 1 {
        return Err(EngineError::BackendState);
    }
    Ok(OperatorSnapshot {
        registers: Operator {
            dt: value(words[0])?,
            mul: value(multi)?,
            tl: value(words[2])?,
            rs: value(words[3])?,
            ar: value(words[4])?,
            d1r: value(words[5])?,
            d2r: value(words[6])?,
            sl: value(sl)?,
            rr: value(words[8])?,
            am: words[9] != 0,
            ssg: SsgEg::try_from(byte(words[10])?)?,
        },
        envelope_attenuation: words[11] as u16,
        output_attenuation: words[12] as u16,
        stage,
        phase_increment: words[14],
        phase: words[15],
    })
}
pub(crate) fn decode_snapshot(words: &[u32; 82]) -> Result<ChannelSnapshot, EngineError> {
    if words[68] > 1
        || words[69] > 1
        || words[74] > 3
        || words[75] > 1
        || ![0, 127].contains(&words[72])
    {
        return Err(EngineError::BackendState);
    }
    Ok(ChannelSnapshot {
        operators: [
            operator(&words[0..16])?,
            operator(&words[16..32])?,
            operator(&words[32..48])?,
            operator(&words[48..64])?,
        ],
        channel: ChannelParameters {
            algorithm: Algorithm::new(byte(words[64])?)?,
            feedback: value(words[65])?,
            ams: value(words[66])?,
            fms: value(words[67])?,
            left: words[68] != 0,
            right: words[69] != 0,
        },
        global: GlobalParameters {
            lfo_enabled: words[72] != 0,
            lfo_rate: value(words[73])?,
        },
        frequency: frequency(words[70], words[71])?,
        special_frequencies: [
            frequency(words[78], words[79])?,
            frequency(words[80], words[81])?,
            frequency(words[76], words[77])?,
        ],
        channel3_mode: words[74] as u8,
        dac_enabled: words[75] != 0,
    })
}

/// Load a patch through the normal bus schedule and inspect the real registers
/// before key-on. Useful for checking sessions without rendering a complete note.
pub fn inspect_instrument(
    patch: &Instrument,
    request: &RenderRequest,
) -> Result<ChannelSnapshot, EngineError> {
    let schedule = schedule_note(patch, request)?;
    let setup = schedule
        .events
        .iter()
        .copied()
        .take_while(|event| event.tick < schedule.capture_start)
        .collect::<Vec<_>>();
    let mut chip = NukedChip::new(request.profile)?;
    render_trace(&mut chip, &setup, schedule.capture_start, 24)?;
    chip.snapshot(request.channel)
}

/// Converts hardware channel numbering to native array indexing at one boundary.
pub(crate) fn channel_index(channel: Channel) -> u32 {
    u32::from(channel.number() - 1)
}

#[derive(Debug, Clone)]
pub struct EnvelopePoint {
    pub frame: usize,
    pub operators: [OperatorSnapshot; 4],
}
pub struct DiagnosticRender {
    pub audio: crate::NativeAudio,
    pub envelopes: Vec<EnvelopePoint>,
}

/// Actual snapshots at native-frame boundaries. Port writes, including a
/// non-frame-aligned key-off, are replayed at exactly their offline bus ticks.
/// Bounded native batches allow a new editor revision to cancel this work.
pub fn render_diagnostics(
    patch: &Instrument,
    request: &RenderRequest,
    max_points: usize,
    cancel: impl Fn() -> bool,
) -> Result<DiagnosticRender, EngineError> {
    if !(2..=4096).contains(&max_points) {
        return Err(EngineError::InvalidRequest(
            "envelope point budget must be 2..4096",
        ));
    }
    render_bounded(patch, request, Some(max_points), cancel)
}

/// Audio-only equivalent of offline rendering, with bounded pin storage and
/// cancellation between at most 128 native frames. Uses the same bus schedule.
pub fn render_note_cancellable(
    patch: &Instrument,
    request: &RenderRequest,
    cancel: impl Fn() -> bool,
) -> Result<crate::NativeAudio, EngineError> {
    Ok(render_bounded(patch, request, None, cancel)?.audio)
}

fn render_bounded(
    patch: &Instrument,
    request: &RenderRequest,
    max_points: Option<usize>,
    cancel: impl Fn() -> bool,
) -> Result<DiagnosticRender, EngineError> {
    use crate::{ChipBackend, TICKS_PER_FRAME, integrate_pins};
    let schedule = schedule_note(patch, request)?;
    let mut chip = NukedChip::new(request.profile)?;
    chip.reset()?;
    let mut tick = 0;
    let mut event = 0;
    let mut pins = [[0_i16; 2]; 3072];
    while tick < schedule.capture_start {
        if cancel() {
            return Err(EngineError::Cancelled);
        }
        let count = (schedule.capture_start - tick).min(pins.len() as u64) as usize;
        advance(
            &mut chip,
            &schedule,
            &mut tick,
            &mut event,
            &mut pins[..count],
        )?;
    }
    let mut envelopes = Vec::with_capacity(max_points.unwrap_or(0));
    if max_points.is_some() {
        envelopes.push(EnvelopePoint {
            frame: 0,
            operators: chip.snapshot(request.channel)?.operators,
        });
    }
    let stride = max_points.map_or(schedule.frames, |points| {
        schedule.frames.div_ceil(points - 1).max(1)
    });
    let mut frames = Vec::with_capacity(schedule.frames);
    while frames.len() < schedule.frames {
        if cancel() {
            return Err(EngineError::Cancelled);
        }
        let next_point = ((frames.len() / stride + 1) * stride).min(schedule.frames);
        let count = (next_point - frames.len()).min(128);
        advance(
            &mut chip,
            &schedule,
            &mut tick,
            &mut event,
            &mut pins[..count * TICKS_PER_FRAME],
        )?;
        frames.extend(integrate_pins(&pins[..count * TICKS_PER_FRAME]));
        if max_points.is_some() && frames.len() == next_point {
            envelopes.push(EnvelopePoint {
                frame: frames.len(),
                operators: chip.snapshot(request.channel)?.operators,
            });
        }
    }
    Ok(DiagnosticRender {
        audio: crate::NativeAudio {
            frames,
            sample_rate: request.clock.native_hz(),
            schedule,
        },
        envelopes,
    })
}

fn advance(
    chip: &mut NukedChip,
    schedule: &crate::NoteSchedule,
    tick: &mut u64,
    event: &mut usize,
    output: &mut [[i16; 2]],
) -> Result<(), EngineError> {
    use crate::ChipBackend;
    let end = *tick + output.len() as u64;
    let mut written = 0;
    while *tick < end {
        if *event < schedule.events.len() && schedule.events[*event].tick == *tick {
            let write = schedule.events[*event];
            chip.write_port(write.port as u8, write.data)?;
            *event += 1;
        }
        let next = schedule
            .events
            .get(*event)
            .map_or(end, |write| write.tick.min(end));
        let count = (next - *tick) as usize;
        chip.clock_into(&mut output[written..written + count])?;
        *tick = next;
        written += count;
    }
    Ok(())
}
