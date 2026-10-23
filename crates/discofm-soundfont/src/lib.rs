//! Lazy sample-bank reader. Extracts samples, rather than synthesizing MIDI presets.
#![forbid(unsafe_code)]
mod riff;
mod sfz;
use discofm_audio::AudioBuffer;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BankError {
    #[error("Soundfont: {0}")]
    Invalid(String),
    #[error("Soundfont: {0}")]
    Io(#[from] std::io::Error),
    #[error("Sample: {0}")]
    Audio(#[from] discofm_analysis::AnalysisError),
    #[error("Leitura do sample cancelada")]
    Cancelled,
}
pub(crate) fn invalid(message: impl Into<String>) -> BankError {
    BankError::Invalid(message.into())
}
pub(crate) fn check(cancel: &impl Fn() -> bool) -> Result<(), BankError> {
    if cancel() {
        Err(BankError::Cancelled)
    } else {
        Ok(())
    }
}
/// Index is zero based internally; the GUI and CLI display one-based sample IDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SampleSelection {
    pub index: usize,
    pub name: String,
    pub identity: String,
}
impl SampleSelection {
    pub fn validate(&self) -> bool {
        self.identity.len() == 16 && self.identity.bytes().all(|b| b.is_ascii_hexdigit())
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct SampleInfo {
    pub index: usize,
    pub name: String,
    pub sample_rate: Option<u32>,
    pub frames: Option<u64>,
    pub channels: u8,
    pub root_key: Option<u8>,
    pub pitch_correction: i16,
    /// Exclusive loop end in decoded frames. Informational; playback is one shot.
    pub loop_frames: Option<[u64; 2]>,
    pub warning: Option<String>,
}
#[derive(Debug, Clone)]
pub(crate) enum Source {
    Pcm {
        offset: u64,
        frames: u64,
        low: Option<u64>,
    },
    Ogg {
        offset: u64,
        bytes: u64,
    },
    Wave {
        offset: u64,
        bytes: u64,
    },
    External {
        path: PathBuf,
        start: u64,
        end: Option<u64>,
    },
    Unavailable,
}
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub info: SampleInfo,
    pub source: Source,
    pub pair: Option<usize>,
    pub left: bool,
}
#[derive(Debug, Clone)]
pub struct SoundBank {
    pub path: PathBuf,
    pub format: String,
    pub samples: Vec<SampleInfo>,
    entries: Vec<Entry>,
    stamp: u64,
    signatures: std::collections::BTreeMap<PathBuf, Option<(u64, Option<std::time::SystemTime>)>>,
}
pub fn is_bank(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        ["sf2", "sf3", "sfz", "dls"]
            .iter()
            .any(|e| s.eq_ignore_ascii_case(e))
    })
}
impl SoundBank {
    pub fn open(path: impl AsRef<Path>, cancel: &impl Fn() -> bool) -> Result<Self, BankError> {
        check(cancel)?;
        let path = path.as_ref().canonicalize()?;
        let (format, entries) = if path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("sfz"))
        {
            ("SFZ".into(), sfz::read(&path, cancel)?)
        } else {
            riff::read(&path, cancel)?
        };
        if entries.is_empty() {
            return Err(invalid("Banco sem samples"));
        }
        // Include the catalog plus source-file metadata. Replacement/touch requires reselection.
        let mut stamp = String::new();
        for entry in &entries {
            stamp.push_str(&format!("{:?}{:?}{}", entry.info, entry.pair, entry.left));
            match &entry.source {
                Source::External {
                    path: external,
                    start,
                    end,
                } => stamp.push_str(&format!(
                    "{:?}{start}{end:?}",
                    external
                        .strip_prefix(path.parent().unwrap_or(std::path::Path::new("")))
                        .unwrap_or(external)
                )),
                source => stamp.push_str(&format!("{source:?}")),
            }
        }
        let mut signatures = std::collections::BTreeMap::new();
        for source in std::iter::once(path.as_path()).chain(entries.iter().filter_map(|e| {
            if let Source::External { path, .. } = &e.source {
                Some(path.as_path())
            } else {
                None
            }
        })) {
            let signature = source.metadata().ok().map(|m| (m.len(), m.modified().ok()));
            stamp.push_str(&format!("{signature:?}"));
            signatures.insert(source.to_path_buf(), signature);
        }
        let stamp = stamp.bytes().fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
        });
        Ok(Self {
            path,
            format,
            samples: entries.iter().map(|e| e.info.clone()).collect(),
            entries,
            stamp,
            signatures,
        })
    }
    pub fn selection(&self, index: usize) -> Result<SampleSelection, BankError> {
        let info = self
            .samples
            .get(index)
            .ok_or_else(|| invalid("Sample index is outside the bank"))?;
        let mut hash = self.stamp;
        for byte in (index as u64).to_le_bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
        Ok(SampleSelection {
            index,
            name: info.name.clone(),
            identity: format!("{hash:016x}"),
        })
    }
    pub fn read_selection(
        &self,
        selected: &SampleSelection,
        cancel: &impl Fn() -> bool,
    ) -> Result<AudioBuffer, BankError> {
        if self.selection(selected.index)? != *selected {
            return Err(invalid(
                "The bank changed since the session was saved; select the sample again",
            ));
        }
        self.read_sample(selected.index, cancel)
    }
    pub fn read_sample(
        &self,
        index: usize,
        cancel: &impl Fn() -> bool,
    ) -> Result<AudioBuffer, BankError> {
        check(cancel)?;
        let entry = self
            .entries
            .get(index)
            .ok_or_else(|| invalid("Sample index is outside the bank"))?;
        for source in std::iter::once(&self.path).chain(match &entry.source {
            Source::External { path, .. } => Some(path),
            _ => None,
        }) {
            let current = source.metadata().ok().map(|m| (m.len(), m.modified().ok()));
            if self.signatures.get(source) != Some(&current) {
                return Err(invalid(
                    "O banco ou sample externo mudou; reabra o banco para selecionar novamente",
                ));
            }
        }
        let mut audio = self.decode(entry, cancel)?;
        if let Some(pair) = entry.pair {
            let other = self.decode(&self.entries[pair], cancel)?;
            if audio.sample_rate != other.sample_rate || audio.frames.len() != other.frames.len() {
                return Err(invalid("Stereo pair with mismatched length or rate"));
            }
            let left = entry.left;
            for (a, b) in audio.frames.iter_mut().zip(other.frames) {
                *a = if left { [a[0], b[0]] } else { [b[0], a[0]] };
            }
        }
        check(cancel)?;
        Ok(audio)
    }
    fn decode(&self, entry: &Entry, cancel: &impl Fn() -> bool) -> Result<AudioBuffer, BankError> {
        match &entry.source {
            Source::Unavailable => Err(invalid(
                entry
                    .info
                    .warning
                    .as_deref()
                    .unwrap_or("Sample unavailable"),
            )),
            Source::External { path, start, end } => {
                let mut file =
                    File::open(path).map_err(|e| invalid(format!("{}: {e}", path.display())))?;
                let mut magic = [0; 4];
                file.read_exact(&mut magic)?;
                file.rewind()?;
                let mut audio = if &magic == b"OggS" {
                    let bytes = read_bytes(&mut file, 0, file_len(path)?, cancel)?;
                    discofm_analysis::decode_audio(bytes, cancel)?
                } else {
                    discofm_analysis::read_wav_cancellable(file, cancel)?
                };
                check(cancel)?;
                let last = end.unwrap_or(audio.frames.len() as u64);
                if *start >= last || last > audio.frames.len() as u64 {
                    return Err(invalid("SFZ: offset/end fora do sample"));
                }
                audio.frames.truncate(last as usize);
                if *start > 0 {
                    audio.frames.drain(..*start as usize);
                }
                Ok(audio)
            }
            Source::Pcm {
                offset,
                frames,
                low,
            } => {
                let mut file = File::open(&self.path)?;
                let mut output = Vec::new();
                output
                    .try_reserve_exact(
                        usize::try_from(*frames).map_err(|_| invalid("Sample grande demais"))?,
                    )
                    .map_err(|_| invalid("Not enough memory for the sample"))?;
                let mut remaining = *frames;
                let mut done = 0;
                let mut high = [0u8; 16384];
                let mut lows = [0u8; 8192];
                while remaining > 0 {
                    check(cancel)?;
                    let count = remaining.min(8192) as usize;
                    file.seek(SeekFrom::Start(offset + done * 2))?;
                    file.read_exact(&mut high[..count * 2])?;
                    if let Some(low) = low {
                        file.seek(SeekFrom::Start(low + done))?;
                        file.read_exact(&mut lows[..count])?;
                    }
                    for (i, word) in high[..count * 2].as_chunks::<2>().0.iter().enumerate() {
                        let value = i32::from(i16::from_le_bytes([word[0], word[1]]));
                        let value = if low.is_some() {
                            ((value << 8) | i32::from(lows[i])) as f32 / 8388608.0
                        } else {
                            value as f32 / 32768.0
                        };
                        output.push([value; 2]);
                    }
                    done += count as u64;
                    remaining -= count as u64;
                }
                Ok(AudioBuffer {
                    frames: output,
                    sample_rate: entry.info.sample_rate.unwrap_or(0),
                })
            }
            Source::Ogg { offset, bytes } => {
                let bytes = read_bytes(&mut File::open(&self.path)?, *offset, *bytes, cancel)?;
                if !bytes.starts_with(b"OggS") {
                    return Err(invalid(
                        "SF3: unsupported compressed codec (expected Ogg/Vorbis)",
                    ));
                }
                let audio = discofm_analysis::decode_mono_audio(bytes, cancel)?;
                if Some(audio.sample_rate) != entry.info.sample_rate {
                    return Err(invalid("SF3: sample rate differs from the header"));
                }
                Ok(audio)
            }
            Source::Wave { offset, bytes } => {
                let data = read_bytes(&mut File::open(&self.path)?, *offset, *bytes, cancel)?;
                let mut wave = Vec::with_capacity(data.len() + 12);
                wave.extend(b"RIFF");
                wave.extend(
                    u32::try_from(bytes + 4)
                        .map_err(|_| invalid("DLS wave grande demais"))?
                        .to_le_bytes(),
                );
                wave.extend(b"WAVE");
                wave.extend(data);
                let audio =
                    discofm_analysis::read_wav_cancellable(std::io::Cursor::new(wave), cancel)?;
                check(cancel)?;
                Ok(audio)
            }
        }
    }
}
fn file_len(path: &Path) -> Result<u64, BankError> {
    Ok(path.metadata()?.len())
}
pub(crate) fn read_bytes(
    file: &mut File,
    offset: u64,
    length: u64,
    cancel: &impl Fn() -> bool,
) -> Result<Vec<u8>, BankError> {
    let size = usize::try_from(length).map_err(|_| invalid("Chunk grande demais"))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| invalid("Not enough memory"))?;
    file.seek(SeekFrom::Start(offset))?;
    let mut buffer = [0; 65536];
    let mut remaining = size;
    while remaining > 0 {
        check(cancel)?;
        let n = remaining.min(buffer.len());
        file.read_exact(&mut buffer[..n])?;
        bytes.extend(&buffer[..n]);
        remaining -= n;
    }
    Ok(bytes)
}
/// Shared GUI/CLI resolver. A bank requires an explicit sample selection.
pub fn read_reference(
    path: &Path,
    selected: Option<&SampleSelection>,
    cancel: &impl Fn() -> bool,
) -> Result<AudioBuffer, BankError> {
    if is_bank(path) {
        SoundBank::open(path, cancel)?.read_selection(
            selected.ok_or_else(|| invalid("Selecione um sample do banco"))?,
            cancel,
        )
    } else if selected.is_some() {
        Err(invalid("Bank selection does not match an audio file"))
    } else {
        check(cancel)?;
        let audio = discofm_analysis::read_wav_cancellable(File::open(path)?, cancel)?;
        check(cancel)?;
        Ok(audio)
    }
}
