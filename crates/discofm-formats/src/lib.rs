//! Versioned internal sessions, plus tracker instrument export/import (`export`).
#![forbid(unsafe_code)]
mod export;
use discofm_chip::Instrument;
use discofm_engine::{EngineError, RenderRequest};
pub use export::{ImportError, InstrumentFormat};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use thiserror::Error;

pub const SESSION_VERSION: u32 = 1;
pub const MAX_SESSION_BYTES: usize = 1_048_576;

#[derive(Debug, Error)]
pub enum SessionError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("invalid session field {path}: {source}")]
    Field {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("audition: {0}")]
    Render(#[from] EngineError),
    #[error("instrument.channel3_special: {0}")]
    Channel(#[from] discofm_chip::ParameterError),
    #[error("reference_analysis: {0}")]
    Analysis(#[from] discofm_analysis::AnalysisError),
    #[error("fitting: {0}")]
    Fitting(#[from] discofm_fit::FitError),
    #[error("reference_analysis: invalid note-on/key-off markers")]
    Markers,
    #[error("reference_sample: invalid selection or missing reference_path")]
    Sample,
    #[error("unsupported internal session version {0}")]
    Version(u32),
    #[error("internal session exceeds 1 MiB limit")]
    TooLarge,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Session {
    pub version: u32,
    pub instrument: Instrument,
    pub audition: RenderRequest,
    pub reference_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_sample: Option<discofm_soundfont::SampleSelection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_analysis: Option<ReferenceSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fitting: Option<discofm_fit::FittingState>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceSettings {
    pub options: discofm_analysis::AnalysisOptions,
    pub note_on_seconds: f64,
    pub key_off_seconds: f64,
}
impl ReferenceSettings {
    pub fn validate(&self) -> Result<(), SessionError> {
        self.options.validate()?;
        if !self.note_on_seconds.is_finite()
            || !self.key_off_seconds.is_finite()
            || self.note_on_seconds < 0.0
            || self.key_off_seconds < self.note_on_seconds
        {
            return Err(SessionError::Markers);
        }
        Ok(())
    }
}
/// Wire representation is private, so direct serde deserialization also enforces
/// cross-field constraints instead of bypassing the read_session boundary.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionData {
    version: u32,
    instrument: Instrument,
    audition: RenderRequest,
    reference_path: Option<String>,
    reference_sample: Option<discofm_soundfont::SampleSelection>,
    reference_analysis: Option<ReferenceSettings>,
    fitting: Option<discofm_fit::FittingState>,
}
impl TryFrom<SessionData> for Session {
    type Error = SessionError;
    fn try_from(data: SessionData) -> Result<Self, Self::Error> {
        let session = Self {
            version: data.version,
            instrument: data.instrument,
            audition: data.audition,
            reference_path: data.reference_path,
            reference_sample: data.reference_sample,
            reference_analysis: data.reference_analysis,
            fitting: data.fitting,
        };
        session.validate()?;
        Ok(session)
    }
}
impl<'de> Deserialize<'de> for Session {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        SessionData::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}
impl Default for Session {
    fn default() -> Self {
        Self {
            version: SESSION_VERSION,
            instrument: Instrument::default(),
            audition: RenderRequest::default(),
            reference_path: None,
            reference_sample: None,
            reference_analysis: None,
            fitting: None,
        }
    }
}
impl Session {
    pub fn validate(&self) -> Result<(), SessionError> {
        if self.version != SESSION_VERSION {
            return Err(SessionError::Version(self.version));
        }
        if let Some(sample) = &self.reference_sample
            && (!sample.validate()
                || self
                    .reference_path
                    .as_ref()
                    .is_none_or(|p| !discofm_soundfont::is_bank(std::path::Path::new(p))))
        {
            return Err(SessionError::Sample);
        }
        self.audition.validate()?;
        self.instrument.validate_channel(self.audition.channel)?;
        if let Some(reference) = &self.reference_analysis {
            reference.validate()?;
        }
        if let Some(fitting) = &self.fitting {
            fitting.validate()?;
        }
        Ok(())
    }
}
pub fn read_session(reader: impl Read) -> Result<Session, SessionError> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_SESSION_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_SESSION_BYTES {
        return Err(SessionError::TooLarge);
    }
    let mut deserializer = serde_json::Deserializer::from_slice(&bytes);
    let data: SessionData =
        serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
            SessionError::Field {
                path: error.path().to_string(),
                source: error.into_inner(),
            }
        })?;
    deserializer.end()?;
    data.try_into()
}
pub fn write_session(writer: impl Write, session: &Session) -> Result<(), SessionError> {
    session.validate()?;
    let mut bytes = serde_json::to_vec_pretty(session)?;
    // Indentation of nested candidates and history doubles a fitted session:
    // eight retained runs can exceed the limit pretty-printed, but they always
    // fit compact (worst-case test in tests/sessions.rs). Both forms read back.
    if bytes.len() > MAX_SESSION_BYTES {
        bytes = serde_json::to_vec(session)?;
    }
    if bytes.len() > MAX_SESSION_BYTES {
        return Err(SessionError::TooLarge);
    }
    let mut writer = writer;
    writer.write_all(&bytes)?;
    Ok(())
}

/// Encodes an instrument for other tools; [`InstrumentFormat`] covers the
/// tracker formats GENNY writes.
pub trait InstrumentExporter {
    type Error;
    fn export(&self, instrument: &Instrument) -> Result<Vec<u8>, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sessions_roundtrip_and_validate_versions_and_values() {
        let original = Session::default();
        let mut bytes = Vec::new();
        write_session(&mut bytes, &original).unwrap();
        assert_eq!(original, read_session(&bytes[..]).unwrap());
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["version"] = 999.into();
        assert!(read_session(&serde_json::to_vec(&value).unwrap()[..]).is_err());
        value["version"] = 1.into();
        value["instrument"]["operators"][0]["ar"] = 32.into();
        assert!(read_session(&serde_json::to_vec(&value).unwrap()[..]).is_err());
        assert!(read_session(&vec![b' '; 1_048_577][..]).is_err());
        for content in ["x".repeat(1_048_576), "\"".repeat(524_288)] {
            let oversized = Session {
                reference_path: Some(content),
                ..Session::default()
            };
            let mut output = Vec::new();
            assert!(matches!(
                write_session(&mut output, &oversized),
                Err(SessionError::TooLarge)
            ));
            assert!(output.is_empty());
        }
    }
    #[test]
    fn sample_selection_is_optional_validated_and_roundtrips() {
        let mut session = Session {
            reference_sample: Some(discofm_soundfont::SampleSelection {
                index: 2,
                name: "Duplicate".into(),
                identity: "0123456789abcdef".into(),
            }),
            ..Session::default()
        };
        assert!(matches!(session.validate(), Err(SessionError::Sample)));
        session.reference_path = Some("bank.sf2".into());
        let mut bytes = Vec::new();
        write_session(&mut bytes, &session).unwrap();
        assert_eq!(read_session(&bytes[..]).unwrap(), session);
        session.reference_sample.as_mut().unwrap().identity = "invalid".into();
        assert!(session.validate().is_err());
        let old = serde_json::to_value(Session::default()).unwrap();
        assert!(old.get("reference_sample").is_none());
    }
}
