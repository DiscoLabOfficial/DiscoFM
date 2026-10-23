//! Device management and lifetime. Synthesis lives on a dedicated worker.
mod callback;
mod transport;
use crate::AudioError;
use callback::Callback;
use cpal::{
    SampleFormat, SupportedStreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use discofm_chip::Instrument;
use discofm_engine::{EngineError, LiveConfig};
use serde::Serialize;
use std::{
    sync::{Arc, mpsc},
    thread::{self, JoinHandle},
    time::Duration,
};
use thiserror::Error;
use transport::Shared;
pub use transport::{AuditionControls, VoiceId, octave_note};

#[derive(Debug, Error)]
pub enum PlaybackError {
    #[error("audio device: {0}")]
    Device(#[from] cpal::Error),
    #[error("no default audio output device is available")]
    NoDevice,
    #[error("output device was not found: {0}")]
    DeviceNotFound(String),
    #[error("invalid playback configuration: {0}")]
    Config(&'static str),
    #[error("audio command queue is full; retry the trigger (Stop and release remain available)")]
    QueueFull,
    #[error("octave must be -1..8 and the selected note must fit the chip")]
    Octave,
    #[error("audio stream failed: {0}; select an available device and reopen playback")]
    Stream(&'static str),
    #[error("synthesis worker: {0}")]
    Worker(String),
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error(transparent)]
    Audio(#[from] AudioError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
#[derive(Debug, Clone)]
pub struct DeviceOptions {
    pub device_id: Option<String>,
    pub sample_rate: Option<u32>,
    pub queue_ms: u32,
}
impl Default for DeviceOptions {
    fn default() -> Self {
        Self {
            device_id: None,
            sample_rate: None,
            queue_ms: 40,
        }
    }
}
impl DeviceOptions {
    pub fn validate(&self) -> Result<(), PlaybackError> {
        if !(10..=100).contains(&self.queue_ms) {
            return Err(PlaybackError::Config("queue duration must be 10..100 ms"));
        }
        if self
            .sample_rate
            .is_some_and(|rate| !(8000..=192000).contains(&rate))
        {
            return Err(PlaybackError::Config("sample rate must be 8000..192000 Hz"));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub sample_format: Option<String>,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct OutputInfo {
    pub device_id: String,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: String,
    pub device_buffer_frames: Option<u32>,
    pub queue_frames: usize,
}
#[derive(Debug, Clone, Copy, Serialize)]
pub struct PlaybackStats {
    pub callbacks: usize,
    pub device_frames: usize,
    pub note_frames: usize,
    pub underrun_frames: usize,
    pub notifications: usize,
}

pub fn output_devices() -> Result<Vec<DeviceInfo>, PlaybackError> {
    let host = cpal::default_host();
    let default = host
        .default_output_device()
        .and_then(|device| device.id().ok());
    host.output_devices()?
        .map(|device| {
            let id = device.id()?;
            let config = device.default_output_config();
            Ok(DeviceInfo {
                is_default: default.as_ref() == Some(&id),
                id: id.to_string(),
                name: device.to_string(),
                sample_rate: config.as_ref().ok().map(|config| config.sample_rate()),
                channels: config.as_ref().ok().map(|config| config.channels()),
                sample_format: config
                    .as_ref()
                    .ok()
                    .map(|config| config.sample_format().to_string()),
                error: config.err().map(|error| error.to_string()),
            })
        })
        .collect()
}
fn supported_format(format: SampleFormat) -> bool {
    matches!(
        format,
        SampleFormat::F32
            | SampleFormat::F64
            | SampleFormat::I8
            | SampleFormat::I16
            | SampleFormat::I24
            | SampleFormat::I32
            | SampleFormat::I64
            | SampleFormat::U8
            | SampleFormat::U16
            | SampleFormat::U32
            | SampleFormat::U64
    )
}
fn choose_config(
    device: &cpal::Device,
    rate: Option<u32>,
) -> Result<SupportedStreamConfig, PlaybackError> {
    let default = device.default_output_config()?;
    let target = rate.unwrap_or(default.sample_rate());
    if !(8000..=192000).contains(&target) {
        return Err(PlaybackError::Config(
            "device rate is outside 8000..192000 Hz",
        ));
    }
    if target == default.sample_rate()
        && (1..=32).contains(&default.channels())
        && supported_format(default.sample_format())
    {
        return Ok(default);
    }
    device
        .supported_output_configs()?
        .filter(|range| {
            range.min_sample_rate() <= target
                && target <= range.max_sample_rate()
                && (1..=32).contains(&range.channels())
                && supported_format(range.sample_format())
        })
        .min_by_key(|range| {
            (
                range.channels() != default.channels(),
                range.sample_format() != default.sample_format(),
                range.sample_format() != SampleFormat::F32,
            )
        })
        .map(|range| range.with_sample_rate(target))
        .ok_or(PlaybackError::Config(
            "requested output configuration is unsupported",
        ))
}

/// Dropping playback mutes immediately, drops the stream and joins a cancellable
/// worker. Shutdown never depends on a device callback freeing queue slots.
pub struct Playback {
    controls: AuditionControls,
    stream: Option<cpal::Stream>,
    worker: Option<JoinHandle<()>>,
    errors: mpsc::Receiver<String>,
    info: OutputInfo,
}
impl Playback {
    pub fn open(
        patch: Instrument,
        config: LiveConfig,
        options: DeviceOptions,
    ) -> Result<Self, PlaybackError> {
        options.validate()?;
        patch
            .validate_channel(config.channel)
            .map_err(EngineError::from)?;
        if config.profile == discofm_engine::ChipProfile::RawDiagnostic {
            return Err(EngineError::UnsupportedRawProfile.into());
        }
        let host = cpal::default_host();
        let device = match &options.device_id {
            Some(id) => host
                .output_devices()?
                .find(|device| device.id().is_ok_and(|found| found.to_string() == *id))
                .ok_or_else(|| PlaybackError::DeviceNotFound(id.clone()))?,
            None => host
                .default_output_device()
                .ok_or(PlaybackError::NoDevice)?,
        };
        let supported = choose_config(&device, options.sample_rate)?;
        let capacity =
            (u64::from(supported.sample_rate()) * u64::from(options.queue_ms) / 1000) as usize;
        let shared = Arc::new(Shared::default());
        let (audio_tx, audio_rx) = rtrb::RingBuffer::new(capacity);
        let (command_tx, command_rx) = rtrb::RingBuffer::new(32);
        let callback = Callback::new(
            audio_rx,
            Arc::clone(&shared),
            usize::from(supported.channels()),
            capacity,
        );
        let stream = callback::build_stream(&device, &supported, callback, Arc::clone(&shared))?;
        let info = OutputInfo {
            device_id: device.id()?.to_string(),
            device_name: device.to_string(),
            sample_rate: supported.sample_rate(),
            channels: supported.channels(),
            sample_format: supported.sample_format().to_string(),
            device_buffer_frames: stream.buffer_size().ok(),
            queue_frames: capacity,
        };
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (error_tx, errors) = mpsc::sync_channel(1);
        let worker_shared = Arc::clone(&shared);
        let rate = info.sample_rate;
        let worker = thread::Builder::new()
            .name("discofm-synthesis".into())
            .spawn(move || {
                if let Err(error) = transport::worker(
                    config,
                    rate,
                    command_rx,
                    audio_tx,
                    &worker_shared,
                    &ready_tx,
                ) {
                    worker_shared
                        .muted
                        .store(true, std::sync::atomic::Ordering::Release);
                    let message = error.to_string();
                    let _ = ready_tx.try_send(Err(message.clone()));
                    let _ = error_tx.try_send(message);
                }
            })?;
        let controls =
            AuditionControls::new(patch, config, command_tx, shared, worker.thread().clone());
        let playback = Self {
            controls,
            stream: Some(stream),
            worker: Some(worker),
            errors,
            info,
        };
        ready_rx
            .recv_timeout(Duration::from_secs(3))
            .map_err(|_| PlaybackError::Worker("startup timed out or worker exited".into()))?
            .map_err(PlaybackError::Worker)?;
        playback
            .stream
            .as_ref()
            .ok_or_else(|| PlaybackError::Worker("audio stream was not created".into()))?
            .play()?;
        Ok(playback)
    }
    pub fn controls(&mut self) -> &mut AuditionControls {
        &mut self.controls
    }
    pub fn info(&self) -> &OutputInfo {
        &self.info
    }
    pub fn stats(&self) -> PlaybackStats {
        self.controls.shared.stats()
    }
    pub fn check_health(&self) -> Result<(), PlaybackError> {
        let failure = self
            .controls
            .shared
            .failure
            .load(std::sync::atomic::Ordering::Acquire);
        if failure != 0 {
            return Err(PlaybackError::Stream(match failure {
                1 => "device disconnected",
                2 => "stream invalidated",
                _ => "audio backend error",
            }));
        }
        if let Ok(error) = self.errors.try_recv() {
            return Err(PlaybackError::Worker(error));
        }
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            return Err(PlaybackError::Worker("worker exited unexpectedly".into()));
        }
        Ok(())
    }
}
impl Drop for Playback {
    fn drop(&mut self) {
        self.controls.stop();
        self.controls
            .shared
            .shutdown
            .store(true, std::sync::atomic::Ordering::Release);
        self.controls.wake();
        self.stream.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
