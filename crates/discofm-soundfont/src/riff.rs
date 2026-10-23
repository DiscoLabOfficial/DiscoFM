use crate::{BankError, Entry, SampleInfo, Source, check, invalid, read_bytes};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};
#[derive(Clone, Copy, Debug)]
struct Chunk {
    id: [u8; 4],
    offset: u64,
    size: u64,
}
fn u16le(b: &[u8]) -> u16 {
    u16::from_le_bytes(b[..2].try_into().unwrap())
}
fn u32le(b: &[u8]) -> u32 {
    u32::from_le_bytes(b[..4].try_into().unwrap())
}
fn name(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes.split(|b| *b == 0).next().unwrap_or_default())
        .trim()
        .into()
}
fn chunks(
    file: &mut File,
    start: u64,
    limit: u64,
    sf3_padding: bool,
    cancel: &impl Fn() -> bool,
) -> Result<Vec<Chunk>, BankError> {
    let mut offset = start;
    let mut result = Vec::new();
    while offset < limit {
        check(cancel)?;
        if limit - offset < 8 {
            return Err(invalid("Truncated RIFF chunk header"));
        }
        file.seek(SeekFrom::Start(offset))?;
        let mut header = [0; 8];
        file.read_exact(&mut header)?;
        let size = u64::from(u32le(&header[4..]));
        let data = offset + 8;
        let end = data
            .checked_add(size)
            .filter(|n| *n <= limit)
            .ok_or_else(|| invalid("Chunk RIFF ultrapassa o pai"))?;
        let id = header[..4].try_into().unwrap();
        result.push(Chunk {
            id,
            offset: data,
            size,
        });
        offset = end;
        if size & 1 != 0 && offset < limit {
            // MuseScore SF3 may omit padding after odd smpl/LIST payloads.
            let mut byte = [0];
            file.seek(SeekFrom::Start(offset))?;
            file.read_exact(&mut byte)?;
            if !sf3_padding || byte[0] == 0 {
                offset += 1;
            }
        }
    }
    Ok(result)
}
fn kind(file: &mut File, chunk: Chunk) -> Result<[u8; 4], BankError> {
    if chunk.size < 4 {
        return Err(invalid("LIST RIFF sem tipo"));
    }
    file.seek(SeekFrom::Start(chunk.offset))?;
    let mut b = [0; 4];
    file.read_exact(&mut b)?;
    Ok(b)
}
fn unique(list: &[Chunk], id: &[u8; 4]) -> Result<Option<Chunk>, BankError> {
    let found: Vec<_> = list.iter().filter(|c| c.id == *id).copied().collect();
    if found.len() > 1 {
        return Err(invalid(format!(
            "Chunk duplicado {}",
            String::from_utf8_lossy(id)
        )));
    }
    Ok(found.first().copied())
}
fn required(list: &[Chunk], id: &[u8; 4]) -> Result<Chunk, BankError> {
    unique(list, id)?
        .ok_or_else(|| invalid(format!("Chunk ausente {}", String::from_utf8_lossy(id))))
}
pub(crate) fn read(
    path: &Path,
    cancel: &impl Fn() -> bool,
) -> Result<(String, Vec<Entry>), BankError> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    let mut header = [0; 12];
    file.read_exact(&mut header)?;
    if &header[..4] != b"RIFF" {
        return Err(invalid("Esperado banco RIFF SF2/SF3 ou DLS"));
    }
    let limit = 8 + u64::from(u32le(&header[4..8]));
    if limit > length || limit < 12 {
        return Err(invalid("Invalid or truncated RIFF size"));
    }
    match &header[8..] {
        b"sfbk" => sf2(&mut file, limit, cancel),
        b"DLS " => dls(&mut file, limit, cancel).map(|e| ("DLS".into(), e)),
        _ => Err(invalid("Unsupported RIFF bank type")),
    }
}
fn sf2(
    file: &mut File,
    limit: u64,
    cancel: &impl Fn() -> bool,
) -> Result<(String, Vec<Entry>), BankError> {
    let top = chunks(file, 12, limit, true, cancel)?;
    let mut data = None;
    let mut headers = None;
    for chunk in top.into_iter().filter(|c| c.id == *b"LIST") {
        let slot = match &kind(file, chunk)? {
            b"sdta" => &mut data,
            b"pdta" => &mut headers,
            _ => continue,
        };
        if slot.replace(chunk).is_some() {
            return Err(invalid("Duplicate required LIST in sound bank"));
        }
    }
    let data = data.ok_or_else(|| invalid("Soundfont sem sdta"))?;
    let headers = headers.ok_or_else(|| invalid("Soundfont sem pdta"))?;
    let sub = chunks(file, data.offset + 4, data.offset + data.size, true, cancel)?;
    let smpl = required(&sub, b"smpl")?;
    let sm24 = unique(&sub, b"sm24")?;
    let sub = chunks(
        file,
        headers.offset + 4,
        headers.offset + headers.size,
        true,
        cancel,
    )?;
    let shdr = required(&sub, b"shdr")?;
    if shdr.size < 46 || shdr.size % 46 != 0 || shdr.size > 46 * 65536 {
        return Err(invalid("Invalid shdr table"));
    }
    let bytes = read_bytes(file, shdr.offset, shdr.size, cancel)?;
    if !["", "EOS"].contains(&name(&bytes[bytes.len() - 46..bytes.len() - 26]).as_str()) {
        return Err(invalid("shdr sem registro terminal EOS"));
    }
    let mut entries = Vec::new();
    let mut links = Vec::new();
    let mut compressed = false;
    for (index, b) in bytes[..bytes.len() - 46]
        .as_chunks::<46>()
        .0
        .iter()
        .enumerate()
    {
        check(cancel)?;
        let start = u64::from(u32le(&b[20..]));
        let end = u64::from(u32le(&b[24..]));
        let ls = u64::from(u32le(&b[28..]));
        let le = u64::from(u32le(&b[32..]));
        let rate = u32le(&b[36..]);
        let typ = u16le(&b[44..]);
        let ogg = typ & 0x10 != 0;
        let mut warning = None;
        let source = if typ & 0x8000 != 0 {
            warning = Some("ROM sample: audio is not stored in this file".into());
            Source::Unavailable
        } else if rate == 0 || start >= end || (typ & 0xf != 1 && typ & 0xf != 2 && typ & 0xf != 4)
        {
            warning = Some("Unsupported sample header or type".into());
            Source::Unavailable
        } else if ogg {
            compressed = true;
            if end > smpl.size || start >= smpl.size {
                warning = Some("SF3: offsets fora de smpl".into());
                Source::Unavailable
            } else {
                // Accept inclusive FluidSynth and exclusive MuseScore ends. Stop on EOS;
                // a decoder never receives the neighboring sample's first byte.
                let available = (end.saturating_add(1).min(smpl.size)) - start;
                match ogg_length(file, smpl.offset + start, available, cancel) {
                    Ok(length) => Source::Ogg {
                        offset: smpl.offset + start,
                        bytes: length,
                    },
                    Err(BankError::Cancelled) => return Err(BankError::Cancelled),
                    Err(error) => {
                        warning = Some(error.to_string());
                        Source::Unavailable
                    }
                }
            }
        } else if end > smpl.size / 2 {
            warning = Some("SF2: offsets PCM fora de smpl".into());
            Source::Unavailable
        } else if sm24.is_some_and(|c| end > c.size) {
            warning = Some("SF2: sm24 menor que o sample PCM".into());
            Source::Unavailable
        } else {
            Source::Pcm {
                offset: smpl.offset + start * 2,
                frames: end - start,
                low: sm24.map(|c| c.offset + start),
            }
        };
        let frames = if ogg {
            None
        } else {
            Some(end.saturating_sub(start))
        };
        let loop_frames = if ogg {
            (ls < le).then_some([ls, le])
        } else {
            // `then`, not `then_some`: the subtraction must not run when a loop
            // starts before the sample, which overflows on malformed banks.
            (start <= ls && ls < le && le <= end).then(|| [ls - start, le - start])
        };
        entries.push(Entry {
            info: SampleInfo {
                index,
                name: name(&b[..20]),
                sample_rate: Some(rate),
                frames,
                channels: 1,
                root_key: (b[40] <= 127).then_some(b[40]),
                pitch_correction: i16::from(b[41] as i8),
                loop_frames,
                warning,
            },
            source,
            pair: None,
            left: typ & 0xf == 4,
        });
        links.push((usize::from(u16le(&b[42..])), typ & 0xf));
    }
    for i in 0..entries.len() {
        let (j, typ) = links[i];
        if typ != 2 && typ != 4 {
            continue;
        }
        if j != i
            && j < entries.len()
            && links[j] == (i, if typ == 2 { 4 } else { 2 })
            && entries[i].info.sample_rate == entries[j].info.sample_rate
            && entries[i].info.frames == entries[j].info.frames
            && !matches!(entries[i].source, Source::Unavailable)
            && !matches!(entries[j].source, Source::Unavailable)
        {
            entries[i].pair = Some(j);
            entries[i].info.channels = 2;
        } else if entries[i].info.warning.is_none() {
            entries[i].info.warning = Some("Invalid stereo link; playing mono".into());
        }
    }
    Ok((if compressed { "SF3" } else { "SF2" }.into(), entries))
}
fn ogg_length(
    file: &mut File,
    offset: u64,
    available: u64,
    cancel: &impl Fn() -> bool,
) -> Result<u64, BankError> {
    let mut consumed = 0;
    while consumed < available {
        check(cancel)?;
        if available - consumed < 27 {
            return Err(invalid("SF3: truncated Ogg page"));
        }
        file.seek(SeekFrom::Start(offset + consumed))?;
        let mut h = [0; 27];
        file.read_exact(&mut h)?;
        if &h[..4] != b"OggS" || h[4] != 0 {
            return Err(invalid("SF3: compression is not Ogg/Vorbis"));
        }
        let mut lacing = [0; 255];
        let n = usize::from(h[26]);
        file.read_exact(&mut lacing[..n])?;
        let size = 27 + n as u64 + lacing[..n].iter().map(|b| u64::from(*b)).sum::<u64>();
        if size > available - consumed {
            return Err(invalid("SF3: dados Ogg truncados"));
        }
        consumed += size;
        if h[5] & 4 != 0 {
            return Ok(consumed);
        }
    }
    Err(invalid("SF3: stream without a final EOS page"))
}
fn dls(file: &mut File, limit: u64, cancel: &impl Fn() -> bool) -> Result<Vec<Entry>, BankError> {
    let top = chunks(file, 12, limit, false, cancel)?;
    let mut pool = None;
    for c in top.into_iter().filter(|c| c.id == *b"LIST") {
        if &kind(file, c)? == b"wvpl" && pool.replace(c).is_some() {
            return Err(invalid("wvpl duplicado"));
        }
    }
    let pool = pool.ok_or_else(|| invalid("DLS sem wave pool"))?;
    let mut entries = Vec::new();
    for wave in chunks(
        file,
        pool.offset + 4,
        pool.offset + pool.size,
        false,
        cancel,
    )? {
        if wave.id != *b"LIST" || &kind(file, wave)? != b"wave" {
            continue;
        }
        let sub = chunks(
            file,
            wave.offset + 4,
            wave.offset + wave.size,
            false,
            cancel,
        )?;
        let fmt = required(&sub, b"fmt ")?;
        let data = required(&sub, b"data")?;
        if fmt.size < 16 || fmt.size > 65536 {
            return Err(invalid("DLS: invalid fmt"));
        }
        let b = read_bytes(file, fmt.offset, fmt.size, cancel)?;
        let rate = u32le(&b[4..]);
        let channels = u16le(&b[2..]);
        let align = u16le(&b[12..]);
        if rate == 0 || channels == 0 || align == 0 {
            return Err(invalid("DLS: invalid layout"));
        }
        let index = entries.len();
        let mut sample_name = format!("Wave {}", index + 1);
        for c in sub.iter().filter(|c| c.id == *b"LIST") {
            if &kind(file, *c)? == b"INFO" {
                let info = chunks(file, c.offset + 4, c.offset + c.size, false, cancel)?;
                if let Some(n) = unique(&info, b"INAM")? {
                    sample_name = name(&read_bytes(file, n.offset, n.size.min(65536), cancel)?);
                }
            }
        }
        let mut root_key = None;
        let mut pitch_correction = 0;
        let mut loop_frames = None;
        if let Some(wsmp) = unique(&sub, b"wsmp")? {
            if wsmp.size < 20 {
                return Err(invalid("DLS: wsmp truncado"));
            }
            let b = read_bytes(file, wsmp.offset, wsmp.size.min(65536), cancel)?;
            let header = u32le(&b) as usize;
            if header < 20 || header > b.len() {
                return Err(invalid("DLS: invalid wsmp header"));
            }
            let note = u16le(&b[4..]);
            root_key = (note <= 127).then_some(note as u8);
            pitch_correction = i16::from_le_bytes(b[6..8].try_into().unwrap());
            if u32le(&b[16..]) > 0 {
                if b.len() - header < 16 {
                    return Err(invalid("DLS: loop truncado"));
                }
                let ls = u64::from(u32le(&b[header + 8..]));
                let length = u64::from(u32le(&b[header + 12..]));
                if length > 0 {
                    loop_frames = Some([ls, ls + length]);
                }
            }
        }
        entries.push(Entry {
            info: SampleInfo {
                index,
                name: sample_name,
                sample_rate: Some(rate),
                frames: [1, 3]
                    .contains(&u16le(&b))
                    .then_some(data.size / u64::from(align)),
                channels: channels.min(2) as u8,
                root_key,
                pitch_correction,
                loop_frames,
                warning: None,
            },
            source: Source::Wave {
                offset: wave.offset + 4,
                bytes: wave.size - 4,
            },
            pair: None,
            left: false,
        });
    }
    Ok(entries)
}
