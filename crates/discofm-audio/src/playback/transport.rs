use super::{PlaybackError, PlaybackStats};
use crate::{AudioBuffer, StreamingResampler};
use discofm_chip::{Instrument, Value};
use discofm_engine::{
    ChipProfile, ClockConfig, EngineError, LiveConfig, LiveRenderer, frequency_for_hz, midi_hz,
};
use rtrb::{Consumer, Producer};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
        mpsc::SyncSender,
    },
    thread::{self, Thread},
    time::Duration,
};

pub(super) struct Shared {
    pub generation: AtomicUsize,
    pub released: AtomicUsize,
    pub muted: AtomicBool,
    pub shutdown: AtomicBool,
    pub failure: AtomicU8,
    pub callbacks: AtomicUsize,
    pub device_frames: AtomicUsize,
    pub note_frames: AtomicUsize,
    pub underruns: AtomicUsize,
    pub notifications: AtomicUsize,
    pub completed: AtomicUsize,
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            generation: AtomicUsize::new(0),
            released: AtomicUsize::new(0),
            muted: AtomicBool::new(true),
            shutdown: AtomicBool::new(false),
            failure: AtomicU8::new(0),
            callbacks: AtomicUsize::new(0),
            device_frames: AtomicUsize::new(0),
            note_frames: AtomicUsize::new(0),
            underruns: AtomicUsize::new(0),
            notifications: AtomicUsize::new(0),
            completed: AtomicUsize::new(0),
        }
    }
}
impl Shared {
    pub fn stats(&self) -> PlaybackStats {
        PlaybackStats {
            callbacks: self.callbacks.load(Ordering::Relaxed),
            device_frames: self.device_frames.load(Ordering::Relaxed),
            note_frames: self.note_frames.load(Ordering::Relaxed),
            underrun_frames: self.underruns.load(Ordering::Relaxed),
            notifications: self.notifications.load(Ordering::Relaxed),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceId(usize);
#[derive(Debug, Clone, Copy)]
struct Voice {
    id: VoiceId,
    note: u8,
    velocity: Value<127>,
}
pub(super) struct Start {
    generation: usize,
    source: Source,
}
enum Source {
    Note {
        voice: Voice,
        profile: ChipProfile,
        patch: Arc<Instrument>,
    },
    Clip(Clip),
}
const MAX_RETAINED_CLIP_BYTES: usize = 128 * 1024 * 1024;
struct Clip {
    audio: Arc<AudioBuffer>,
    cursor: usize,
    produced: usize,
    budget: Arc<AtomicUsize>,
    bytes: usize,
}
impl Drop for Clip {
    fn drop(&mut self) {
        self.budget.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
#[derive(Debug, Clone, Copy)]
pub(super) enum Frame {
    Sample {
        generation: usize,
        samples: [f32; 2],
        last: bool,
    },
    End {
        generation: usize,
    },
}
impl Frame {
    pub fn generation(self) -> usize {
        match self {
            Self::Sample { generation, .. } | Self::End { generation } => generation,
        }
    }
}

/// C4=60. Signed arithmetic prevents wrapping low/high octaves into other notes.
pub fn octave_note(octave: i8, semitone: u8, clock: ClockConfig) -> Result<u8, PlaybackError> {
    if !(-1..=8).contains(&octave) || semitone > 12 {
        return Err(PlaybackError::Octave);
    }
    let note = i16::from(octave + 1) * 12 + i16::from(semitone);
    let note = u8::try_from(note).map_err(|_| PlaybackError::Octave)?;
    frequency_for_hz(midi_hz(note)?, clock)?;
    Ok(note)
}
/// Single UI/control handle. Atomics deliver release/Stop even when the trigger
/// queue is full. Voice IDs reject a late key-up from an older held note.
pub struct AuditionControls {
    pub(super) shared: Arc<Shared>,
    commands: Producer<Start>,
    thread: Thread,
    patch: Arc<Instrument>,
    config: LiveConfig,
    octave: i8,
    velocity: Value<127>,
    held: Option<Voice>,
    next_voice: usize,
    clip_budget: Arc<AtomicUsize>,
}
impl AuditionControls {
    pub(super) fn new(
        patch: Instrument,
        config: LiveConfig,
        commands: Producer<Start>,
        shared: Arc<Shared>,
        thread: Thread,
    ) -> Self {
        Self {
            shared,
            commands,
            thread,
            patch: Arc::new(patch),
            config,
            octave: 4,
            velocity: Value::new(127).expect("valid constant"),
            held: None,
            next_voice: 0,
            clip_budget: Arc::new(AtomicUsize::new(0)),
        }
    }
    pub fn profile(&self) -> ChipProfile {
        self.config.profile
    }
    pub fn octave(&self) -> i8 {
        self.octave
    }
    pub fn velocity(&self) -> Value<127> {
        self.velocity
    }
    pub fn held_voice(&self) -> Option<VoiceId> {
        self.held.map(|voice| voice.id)
    }
    pub fn clip_finished(&self) -> bool {
        let generation = self.shared.generation.load(Ordering::Acquire);
        generation != 0 && self.shared.completed.load(Ordering::Acquire) == generation
    }
    /// Original or generated immutable PCM. Switches use the same tagged ring
    /// as live notes and retain at most 128 MiB across queued/active clips.
    pub fn play_clip(&mut self, audio: Arc<AudioBuffer>) -> Result<(), PlaybackError> {
        if audio.sample_rate == 0
            || audio.frames.is_empty()
            || audio
                .frames
                .iter()
                .flatten()
                .any(|sample| !sample.is_finite())
        {
            return Err(PlaybackError::Config(
                "clip must be finite, at a positive sample rate and nonempty",
            ));
        }
        if self.commands.slots() == 0 {
            return Err(PlaybackError::QueueFull);
        }
        let bytes = audio
            .frames
            .capacity()
            .checked_mul(std::mem::size_of::<[f32; 2]>())
            .ok_or(PlaybackError::Config("clip storage exceeds address space"))?;
        self.clip_budget
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes)
                    .filter(|next| *next <= MAX_RETAINED_CLIP_BYTES)
            })
            .map_err(|_| PlaybackError::Config("queued clips exceed 128 MiB; Stop and retry"))?;
        let clip = Clip {
            audio,
            cursor: 0,
            produced: 0,
            budget: Arc::clone(&self.clip_budget),
            bytes,
        };
        self.publish(Source::Clip(clip))?;
        self.held = None;
        Ok(())
    }
    pub fn set_octave(&mut self, octave: i8) -> Result<(), PlaybackError> {
        octave_note(octave, 0, self.config.clock)?;
        self.octave = octave;
        Ok(())
    }
    pub fn set_velocity(&mut self, velocity: Value<127>) {
        self.velocity = velocity;
    }
    pub fn key_on(&mut self, semitone: u8) -> Result<VoiceId, PlaybackError> {
        let note = octave_note(self.octave, semitone, self.config.clock)?;
        self.trigger(note)
    }
    /// Trigger an absolute MIDI pitch with the configured audition velocity.
    pub fn trigger(&mut self, note: u8) -> Result<VoiceId, PlaybackError> {
        self.note_on(note, self.velocity)
    }
    pub fn note_on(&mut self, note: u8, velocity: Value<127>) -> Result<VoiceId, PlaybackError> {
        frequency_for_hz(midi_hz(note)?, self.config.clock)?;
        let id = VoiceId(self.next_voice.wrapping_add(1));
        let voice = Voice { id, note, velocity };
        self.start(voice, self.config.profile, Arc::clone(&self.patch))?;
        self.next_voice = id.0;
        self.held = Some(voice);
        Ok(id)
    }
    pub fn note_off(&mut self, id: VoiceId) {
        if self.held.is_some_and(|voice| voice.id == id) {
            self.shared.released.store(
                self.shared.generation.load(Ordering::Acquire),
                Ordering::Release,
            );
            self.held = None;
            self.wake();
        }
    }
    pub fn release(&mut self) {
        if let Some(id) = self.held_voice() {
            self.note_off(id);
        }
    }
    pub fn stop(&mut self) {
        self.shared.muted.store(true, Ordering::Release);
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        self.held = None;
        self.wake();
    }
    pub fn set_profile(&mut self, profile: ChipProfile) -> Result<(), PlaybackError> {
        if profile == ChipProfile::RawDiagnostic {
            return Err(EngineError::UnsupportedRawProfile.into());
        }
        if profile == self.config.profile {
            return Ok(());
        }
        if let Some(voice) = self.held {
            self.start(voice, profile, Arc::clone(&self.patch))?;
        } else {
            self.stop();
        }
        self.config.profile = profile;
        Ok(())
    }
    pub fn set_instrument(&mut self, patch: Instrument) -> Result<(), PlaybackError> {
        patch
            .validate_channel(self.config.channel)
            .map_err(EngineError::from)?;
        let patch = Arc::new(patch);
        if let Some(voice) = self.held {
            self.start(voice, self.config.profile, Arc::clone(&patch))?;
        } else {
            self.stop();
        }
        self.patch = patch;
        Ok(())
    }
    fn start(
        &mut self,
        voice: Voice,
        profile: ChipProfile,
        patch: Arc<Instrument>,
    ) -> Result<(), PlaybackError> {
        self.publish(Source::Note {
            voice,
            profile,
            patch,
        })
    }
    fn publish(&mut self, source: Source) -> Result<(), PlaybackError> {
        // This is the sole producer. Once a slot is available only the worker
        // can increase available capacity, so publication cannot fail midway.
        if self.commands.slots() == 0 {
            return Err(PlaybackError::QueueFull);
        }
        let generation = self
            .shared
            .generation
            .load(Ordering::Acquire)
            .wrapping_add(1);
        self.shared.generation.store(generation, Ordering::Release);
        self.shared.muted.store(false, Ordering::Release);
        self.commands
            .push(Start { generation, source })
            .map_err(|_| PlaybackError::QueueFull)?;
        self.wake();
        Ok(())
    }
    pub(super) fn wake(&self) {
        self.thread.unpark();
    }
}

pub(super) fn worker(
    config: LiveConfig,
    rate: u32,
    mut commands: Consumer<Start>,
    mut output: Producer<Frame>,
    shared: &Shared,
    ready: &SyncSender<Result<(), String>>,
) -> Result<(), PlaybackError> {
    let mut renderer = LiveRenderer::new(config)?;
    let mut converter = StreamingResampler::new(renderer.sample_rate(), rate)?;
    let mut generation = 0;
    let mut released = false;
    let mut clip: Option<Clip> = None;
    let mut native = [[0.0; 2]; 128];
    let mut converted = [[0.0; 2]; 128];
    let _ = ready.try_send(Ok(()));
    while !shared.shutdown.load(Ordering::Acquire) && shared.failure.load(Ordering::Acquire) == 0 {
        if generation != 0 && generation != shared.generation.load(Ordering::Acquire) {
            renderer.stop()?;
            converter.reset();
            clip = None;
            generation = 0;
        }
        for _ in 0..32 {
            let Ok(command) = commands.pop() else {
                break;
            };
            if command.generation != shared.generation.load(Ordering::Acquire) {
                continue;
            }
            match command.source {
                Source::Note {
                    voice,
                    profile,
                    patch,
                } => {
                    renderer.set_profile(profile)?;
                    renderer.note_on(&patch, voice.note, voice.velocity)?;
                    converter = StreamingResampler::new(renderer.sample_rate(), rate)?;
                    clip = None;
                }
                Source::Clip(source) => {
                    renderer.stop()?;
                    converter = StreamingResampler::new(f64::from(source.audio.sample_rate), rate)?;
                    clip = Some(source);
                }
            }
            generation = command.generation;
            released = false;
        }
        if generation == 0 || shared.muted.load(Ordering::Acquire) || output.slots() == 0 {
            thread::park_timeout(Duration::from_millis(2));
            continue;
        }
        if clip.is_none() && !released && shared.released.load(Ordering::Acquire) == generation {
            renderer.release()?;
            released = true;
        }
        let count = output.slots().min(converted.len());
        let mut written = 0;
        let total_clip_frames = clip.as_ref().map(|clip| {
            (clip.audio.frames.len() as f64 * f64::from(rate) / f64::from(clip.audio.sample_rate))
                .round() as usize
        });
        while written < count {
            written += converter.read_into(&mut converted[written..count]);
            if written < count {
                if let Some(clip) = clip.as_mut() {
                    if clip.cursor == clip.audio.frames.len() {
                        break;
                    }
                    let end = (clip.cursor + 128).min(clip.audio.frames.len());
                    converter.push(&clip.audio.frames[clip.cursor..end])?;
                    clip.cursor = end;
                    if end == clip.audio.frames.len() {
                        converter.finish();
                    }
                } else {
                    renderer.render_into(&mut native)?;
                    converter.push(&native)?;
                }
            }
        }
        if generation != shared.generation.load(Ordering::Acquire)
            || shared.muted.load(Ordering::Acquire)
        {
            continue;
        }
        for samples in &converted[..written] {
            let last = if let Some(clip) = clip.as_mut() {
                clip.produced += 1;
                Some(clip.produced) == total_clip_frames
            } else {
                false
            };
            output
                .push(Frame::Sample {
                    generation,
                    samples: *samples,
                    last,
                })
                .map_err(|_| PlaybackError::Worker("PCM queue contract violated".into()))?;
        }
        if clip
            .as_ref()
            .is_some_and(|clip| Some(clip.produced) == total_clip_frames)
        {
            if total_clip_frames == Some(0) {
                output
                    .push(Frame::End { generation })
                    .map_err(|_| PlaybackError::Worker("EOF queue contract violated".into()))?;
            }
            clip = None;
            generation = 0;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    fn controls(capacity: usize) -> (AuditionControls, Consumer<Start>, Arc<Shared>) {
        let (producer, consumer) = rtrb::RingBuffer::new(capacity);
        let shared = Arc::new(Shared::default());
        (
            AuditionControls::new(
                Instrument::default(),
                LiveConfig::default(),
                producer,
                Arc::clone(&shared),
                thread::current(),
            ),
            consumer,
            shared,
        )
    }
    #[test]
    fn full_trigger_queue_cannot_drop_release_or_stop_or_mutate_failed_trigger() {
        let (mut controls, mut commands, shared) = controls(1);
        let id = controls.note_on(69, Value::new(127).unwrap()).unwrap();
        let generation = shared.generation.load(Ordering::Acquire);
        assert!(matches!(
            controls.note_on(60, Value::new(127).unwrap()),
            Err(PlaybackError::QueueFull)
        ));
        assert_eq!(controls.held_voice(), Some(id));
        assert_eq!(shared.generation.load(Ordering::Acquire), generation);
        controls.note_off(id);
        assert_eq!(shared.released.load(Ordering::Acquire), generation);
        controls.stop();
        assert!(shared.muted.load(Ordering::Acquire));
        assert_ne!(
            commands.pop().unwrap().generation,
            shared.generation.load(Ordering::Acquire)
        );
    }
    #[test]
    fn absolute_and_octave_triggers_share_configured_velocity() {
        let (mut controls, mut commands, _) = controls(4);
        controls.set_velocity(Value::new(37).unwrap());
        controls.trigger(69).unwrap();
        controls.key_on(9).unwrap();
        let Source::Note {
            voice: absolute, ..
        } = commands.pop().unwrap().source
        else {
            panic!("note command");
        };
        let Source::Note { voice: octave, .. } = commands.pop().unwrap().source else {
            panic!("note command");
        };
        assert_eq!((absolute.note, absolute.velocity.get()), (69, 37));
        assert_eq!((octave.note, octave.velocity.get()), (69, 37));
    }
    #[test]
    fn clip_capacity_is_charged_and_failed_commands_preserve_current_generation() {
        let (mut controls, mut commands, shared) = controls(1);
        let mut frames = Vec::with_capacity(1024);
        frames.push([0.1, 0.2]);
        controls
            .play_clip(Arc::new(AudioBuffer {
                sample_rate: 48000,
                frames,
            }))
            .unwrap();
        assert_eq!(controls.clip_budget.load(Ordering::Acquire), 8192);
        let generation = shared.generation.load(Ordering::Acquire);
        assert!(matches!(
            controls.play_clip(Arc::new(AudioBuffer {
                sample_rate: 48000,
                frames: vec![[0.0; 2]; 2]
            })),
            Err(PlaybackError::QueueFull)
        ));
        assert_eq!(shared.generation.load(Ordering::Acquire), generation);
        drop(commands.pop().unwrap());
        assert_eq!(controls.clip_budget.load(Ordering::Acquire), 0);
        assert!(
            controls
                .play_clip(Arc::new(AudioBuffer {
                    sample_rate: 48000,
                    frames: vec![[f32::NAN; 2]; 2]
                }))
                .is_err()
        );
        assert_eq!(shared.generation.load(Ordering::Acquire), generation);
    }
    #[test]
    fn finite_clips_match_complete_offline_resampling_including_last_impulse_and_empty_rounding() {
        for (source, target, length) in [
            (8000, 48000, 1573),
            (192000, 8000, 1573),
            (44100, 48000, 3),
            (192000, 8000, 1),
        ] {
            let mut frames = (0..length)
                .map(|i| [(i as f32 * 0.1).sin() * 0.2, (i as f32 * 0.2).cos() * 0.1])
                .collect::<Vec<_>>();
            *frames.last_mut().unwrap() = [0.7, -0.3];
            let expected = crate::resample(&frames, f64::from(source), target).unwrap();
            let (mut controls, commands, shared) = controls(32);
            controls
                .play_clip(Arc::new(AudioBuffer {
                    sample_rate: source,
                    frames,
                }))
                .unwrap();
            let (audio_tx, mut audio_rx) = rtrb::RingBuffer::new(512);
            let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
            let worker_shared = Arc::clone(&shared);
            let worker = thread::spawn(move || {
                worker(
                    LiveConfig::default(),
                    target,
                    commands,
                    audio_tx,
                    &worker_shared,
                    &ready_tx,
                )
            });
            ready_rx
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .unwrap();
            let mut actual = Vec::new();
            let mut ended = false;
            let deadline = Instant::now() + Duration::from_secs(5);
            while !ended && Instant::now() < deadline {
                match audio_rx.pop() {
                    Ok(Frame::Sample { samples, last, .. }) => {
                        actual.push(samples);
                        ended = last;
                    }
                    Ok(Frame::End { .. }) => ended = true,
                    Err(_) => thread::sleep(Duration::from_millis(1)),
                }
            }
            shared.shutdown.store(true, Ordering::Release);
            worker.thread().unpark();
            worker.join().unwrap().unwrap();
            assert!(ended, "clip must publish completion");
            assert_eq!(
                actual, expected.frames,
                "{source}->{target}, length {length}"
            );
            assert!(
                audio_rx.pop().is_err(),
                "no synthesized samples after finite EOF"
            );
            assert_eq!(controls.clip_budget.load(Ordering::Acquire), 0);
        }
    }
    #[test]
    fn stale_key_up_octaves_profiles_and_invalid_controls_preserve_current_voice() {
        let (mut controls, _, shared) = controls(16);
        let a = controls.note_on(69, Value::new(63).unwrap()).unwrap();
        let b = controls.key_on(0).unwrap();
        controls.note_off(a);
        assert_eq!(controls.held_voice(), Some(b));
        assert_ne!(
            shared.released.load(Ordering::Acquire),
            shared.generation.load(Ordering::Acquire)
        );
        controls.set_octave(5).unwrap();
        assert_eq!(controls.held_voice(), Some(b));
        controls.set_profile(ChipProfile::Ym3438).unwrap();
        assert_eq!(controls.held_voice(), Some(b));
        let epoch = shared.generation.load(Ordering::Acquire);
        assert!(controls.set_profile(ChipProfile::RawDiagnostic).is_err());
        assert!(controls.note_on(127, Value::new(127).unwrap()).is_err());
        assert!(controls.set_octave(-2).is_err());
        assert_eq!(shared.generation.load(Ordering::Acquire), epoch);
        assert_eq!(controls.held_voice(), Some(b));
        controls.note_off(b);
        assert_eq!(shared.released.load(Ordering::Acquire), epoch);
        assert_eq!(octave_note(-1, 0, ClockConfig::Ntsc).unwrap(), 0);
        assert_eq!(octave_note(4, 9, ClockConfig::Ntsc).unwrap(), 69);
        assert!(octave_note(8, 11, ClockConfig::Ntsc).is_err());
        assert!(octave_note(4, 13, ClockConfig::Ntsc).is_err());
    }
    #[test]
    fn worker_shutdown_does_not_wait_for_a_full_pcm_queue_to_drain() {
        let shared = Arc::new(Shared::default());
        let (commands, command_rx) = rtrb::RingBuffer::new(32);
        let (audio_tx, audio_rx) = rtrb::RingBuffer::new(32);
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let worker_shared = Arc::clone(&shared);
        let worker = thread::spawn(move || {
            worker(
                LiveConfig::default(),
                48000,
                command_rx,
                audio_tx,
                &worker_shared,
                &ready_tx,
            )
            .unwrap()
        });
        ready_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        let mut controls = AuditionControls::new(
            Instrument::default(),
            LiveConfig::default(),
            commands,
            Arc::clone(&shared),
            worker.thread().clone(),
        );
        controls.key_on(9).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while audio_rx.slots() != 32 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        let filled = audio_rx.slots() == 32;
        shared.shutdown.store(true, Ordering::Release);
        controls.wake();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !worker.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(2));
        }
        assert!(
            worker.is_finished(),
            "worker must cancel even without a consuming callback"
        );
        worker.join().unwrap();
        assert!(filled, "test must shut down with a full PCM queue");
    }
    #[test]
    fn release_before_worker_starts_reaches_the_chip_and_decays() {
        let (mut controls, commands, shared) = controls(32);
        let mut patch = Instrument::default();
        patch.operators[3].rr = Value::new(15).unwrap();
        controls.set_instrument(patch).unwrap();
        controls.key_on(9).unwrap();
        controls.release();
        let (audio_tx, mut audio_rx) = rtrb::RingBuffer::new(512);
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let worker_shared = Arc::clone(&shared);
        let worker = thread::spawn(move || {
            worker(
                LiveConfig::default(),
                48000,
                commands,
                audio_tx,
                &worker_shared,
                &ready_tx,
            )
        });
        ready_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        let mut frames = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(2);
        while frames.len() < 6000 && Instant::now() < deadline {
            if let Ok(frame) = audio_rx.pop() {
                let Frame::Sample { samples, .. } = frame else {
                    panic!("live sample");
                };
                frames.push(samples);
            } else {
                thread::sleep(Duration::from_millis(1));
            }
        }
        shared.shutdown.store(true, Ordering::Release);
        worker.thread().unpark();
        worker.join().unwrap().unwrap();
        assert_eq!(frames.len(), 6000);
        let tail = &frames[5000..];
        let mean = tail.iter().map(|frame| frame[0]).sum::<f32>() / tail.len() as f32;
        let peak = tail
            .iter()
            .map(|frame| (frame[0] - mean).abs())
            .fold(0.0_f32, f32::max);
        assert!(peak < 1e-5, "released worker tail peak={peak}");
    }
    #[test]
    fn complete_samples_longer_than_analysis_window_are_playable() {
        let (mut controls, mut commands, _) = controls(1);
        controls
            .play_clip(Arc::new(AudioBuffer {
                sample_rate: 8000,
                frames: vec![[0.; 2]; 8000 * 31],
            }))
            .unwrap();
        assert!(commands.pop().is_ok());
        let mut oversized = Vec::with_capacity(MAX_RETAINED_CLIP_BYTES / 8 + 1);
        oversized.push([0.; 2]);
        assert!(
            controls
                .play_clip(Arc::new(AudioBuffer {
                    sample_rate: 8000,
                    frames: oversized
                }))
                .is_err()
        );
    }
}
