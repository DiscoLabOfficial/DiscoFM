//! Broad WAV import and bounded, deterministic monophonic reference analysis.
#![forbid(unsafe_code)]
mod features;
mod pitch;
mod spectrum;
mod wav;
pub use features::*;
use thiserror::Error;
pub use wav::{decode_audio, decode_mono_audio, read_wav, read_wav_cancellable};

pub const MAX_SECONDS: f64 = 30.0;

#[derive(Debug, Error)]
pub enum AnalysisError {
    #[error("WAV: {0}")]
    Wav(#[from] hound::Error),
    #[error("WAV: {0}")]
    Decode(String),
    #[error("WAV: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported reference: {0}")]
    Unsupported(&'static str),
    #[error("invalid analysis settings: {0}")]
    Settings(&'static str),
    #[error("reference must contain nonempty, finite audio at a positive sample rate")]
    InvalidAudio,
    #[error("analysis cancelled")]
    Cancelled,
    #[error(transparent)]
    Audio(#[from] discofm_audio::AudioError),
}
