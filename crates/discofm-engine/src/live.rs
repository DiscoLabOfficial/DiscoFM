//! Persistent monophonic auditioning, using exactly the offline register setup.
use crate::{
    ChipBackend, ChipProfile, ClockConfig, EngineError, NukedChip, PortEvent, REGISTER_TICKS,
    RenderRequest, TICKS_PER_FRAME, schedule_note, schedule_write,
};
use discofm_chip::{Channel, Instrument, Value, key_write};
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LiveConfig {
    pub channel: Channel,
    pub profile: ChipProfile,
    pub clock: ClockConfig,
    pub tuning_cents: f64,
}
impl Default for LiveConfig {
    fn default() -> Self {
        Self {
            channel: Channel::ONE,
            profile: ChipProfile::default(),
            clock: ClockConfig::default(),
            tuning_cents: 0.0,
        }
    }
}

/// Owned by the synthesis worker, never by a device callback. Each trigger is a
/// deterministic fresh voice; release preserves the same chip and envelope tail.
pub struct LiveRenderer {
    chip: NukedChip,
    config: LiveConfig,
    events: VecDeque<PortEvent>,
    tick: u64,
    next_write: u64,
    active: bool,
    held: bool,
}
impl LiveRenderer {
    pub fn new(config: LiveConfig) -> Result<Self, EngineError> {
        Ok(Self {
            chip: NukedChip::new(config.profile)?,
            config,
            events: VecDeque::with_capacity(128),
            tick: 0,
            next_write: 0,
            active: false,
            held: false,
        })
    }
    pub fn config(&self) -> LiveConfig {
        self.config
    }
    pub fn is_held(&self) -> bool {
        self.held
    }
    pub fn sample_rate(&self) -> f64 {
        self.config.clock.native_hz()
    }

    /// Setup is excluded from the note, identically to `render_note`. Validate
    /// before resetting, so an invalid trigger leaves the existing voice intact.
    pub fn note_on(
        &mut self,
        patch: &Instrument,
        note: u8,
        velocity: Value<127>,
    ) -> Result<(), EngineError> {
        let request = RenderRequest {
            note,
            tuning_cents: self.config.tuning_cents,
            velocity,
            channel: self.config.channel,
            profile: self.config.profile,
            clock: self.config.clock,
            key_on_seconds: 1.0,
            total_seconds: 1.0,
        };
        let schedule = schedule_note(patch, &request)?;
        self.chip.reset()?;
        self.events.clear();
        self.events.extend(
            schedule
                .events
                .into_iter()
                .take_while(|event| event.tick < schedule.capture_start + REGISTER_TICKS),
        );
        self.tick = 0;
        let mut scratch = [[0; 2]; 3072];
        while self.tick < schedule.capture_start {
            let count = (schedule.capture_start - self.tick).min(scratch.len() as u64) as usize;
            self.pins_into(&mut scratch[..count])?;
        }
        self.next_write = schedule.capture_start + REGISTER_TICKS;
        self.active = true;
        self.held = true;
        Ok(())
    }
    pub fn release(&mut self) -> Result<(), EngineError> {
        if !self.held {
            return Ok(());
        }
        let tick = self.tick.max(self.next_write);
        let mut writes = Vec::with_capacity(2);
        schedule_write(
            &mut writes,
            tick,
            key_write(self.config.channel, Value::new(0)?),
        );
        self.events.extend(writes);
        self.next_write = tick + REGISTER_TICKS;
        self.held = false;
        Ok(())
    }
    /// Hard transport stop intentionally returns digital silence. It does not
    /// alter the hardware residual in normal held/released note rendering.
    pub fn stop(&mut self) -> Result<(), EngineError> {
        self.active = false;
        self.held = false;
        self.events.clear();
        self.chip.reset()
    }
    pub fn set_profile(&mut self, profile: ChipProfile) -> Result<(), EngineError> {
        if profile == self.config.profile {
            return Ok(());
        }
        let chip = NukedChip::new(profile)?;
        self.chip = chip;
        self.config.profile = profile;
        self.events.clear();
        self.active = false;
        self.held = false;
        self.tick = 0;
        self.next_write = 0;
        Ok(())
    }
    /// Full native frames keep the 24-tick integration boundary across every
    /// caller block. There is no note-length limit or growing PCM allocation.
    pub fn render_into(&mut self, output: &mut [[f32; 2]]) -> Result<(), EngineError> {
        if !self.active {
            output.fill([0.0; 2]);
            return Ok(());
        }
        let mut pins = [[0_i16; 2]; 3072];
        for frames in output.chunks_mut(128) {
            let count = frames.len() * TICKS_PER_FRAME;
            self.pins_into(&mut pins[..count])?;
            for (frame, values) in frames
                .iter_mut()
                .zip(pins[..count].as_chunks::<TICKS_PER_FRAME>().0)
            {
                let sum = values.iter().fold([0_i32; 2], |mut sum, value| {
                    sum[0] += i32::from(value[0]);
                    sum[1] += i32::from(value[1]);
                    sum
                });
                *frame = [
                    sum[0] as f32 / (24.0 * 256.0),
                    sum[1] as f32 / (24.0 * 256.0),
                ];
            }
        }
        Ok(())
    }
    fn pins_into(&mut self, output: &mut [[i16; 2]]) -> Result<(), EngineError> {
        let end = self.tick + output.len() as u64;
        let mut offset = 0;
        while self.tick < end {
            if let Some(event) = self.events.pop_front_if(|event| event.tick == self.tick) {
                self.chip.write_port(event.port as u8, event.data)?;
            }
            let next = self.events.front().map_or(end, |event| event.tick.min(end));
            let count = (next - self.tick) as usize;
            self.chip.clock_into(&mut output[offset..offset + count])?;
            self.tick = next;
            offset += count;
        }
        Ok(())
    }
}
