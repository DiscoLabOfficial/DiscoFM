use crate::AnalysisError;
use discofm_audio::AudioBuffer;
use std::io::{Cursor, Read, Seek, SeekFrom};
use symphonia::core::{
    codecs::audio::AudioDecoderOptions, formats::TrackType, formats::probe::Hint,
    io::MediaSourceStream,
};

fn invalid(message: &str) -> AnalysisError {
    AnalysisError::Decode(message.into())
}
fn allocation(error: impl std::fmt::Display) -> AnalysisError {
    invalid(&format!(
        "insufficient memory for decoded reference: {error}"
    ))
}
fn u16_at(bytes: &[u8], be: bool) -> u16 {
    let bytes = [bytes[0], bytes[1]];
    if be {
        u16::from_be_bytes(bytes)
    } else {
        u16::from_le_bytes(bytes)
    }
}
fn u32_at(bytes: &[u8], be: bool) -> u32 {
    let bytes = [bytes[0], bytes[1], bytes[2], bytes[3]];
    if be {
        u32::from_be_bytes(bytes)
    } else {
        u32::from_le_bytes(bytes)
    }
}
struct Wave {
    fmt: Vec<u8>,
    data: (u64, u64),
    fact: Option<u64>,
    be: bool,
    limit: u64,
}
/// Walk RIFF chunks, honoring RF64's ds64 and RIFX's big endian fields.
fn wave(reader: &mut (impl Read + Seek)) -> Result<Wave, AnalysisError> {
    let file_end = reader.seek(SeekFrom::End(0))?;
    reader.seek(SeekFrom::Start(0))?;
    let mut header = [0; 12];
    reader.read_exact(&mut header)?;
    let be = &header[..4] == b"RIFX";
    let rf64 = &header[..4] == b"RF64";
    if (!be && !rf64 && &header[..4] != b"RIFF") || &header[8..] != b"WAVE" {
        return Err(invalid("expected RIFF, RIFX or RF64 WAVE"));
    }
    let mut limit = if rf64 {
        file_end
    } else {
        u64::from(u32_at(&header[4..8], be)) + 8
    };
    if limit > file_end || limit < 12 {
        return Err(invalid("truncated RIFF container"));
    }
    let (mut fmt, mut data, mut fact, mut large_data) = (None, None, None, None);
    let mut large_frames = None;
    let mut table: Vec<([u8; 4], u64)> = Vec::new();
    let mut position = 12;
    while position + 8 <= limit {
        reader.seek(SeekFrom::Start(position))?;
        let mut chunk = [0; 8];
        reader.read_exact(&mut chunk)?;
        let short_size = u32_at(&chunk[4..], be);
        let size = if short_size == u32::MAX && rf64 {
            if &chunk[..4] == b"data" && data.is_none() {
                large_data.ok_or_else(|| invalid("RF64 data precedes ds64"))?
            } else {
                let index = table
                    .iter()
                    .position(|(id, _)| id == &chunk[..4])
                    .ok_or_else(|| invalid("RF64 chunk has no ds64 table size"))?;
                table.remove(index).1
            }
        } else {
            u64::from(short_size)
        };
        let begin = position + 8;
        let end = begin
            .checked_add(size)
            .ok_or_else(|| invalid("chunk size overflow"))?;
        if end > limit {
            return Err(invalid("truncated WAVE chunk"));
        }
        match &chunk[..4] {
            b"ds64" if rf64 && size >= 28 => {
                let mut sizes = [0; 28];
                reader.read_exact(&mut sizes)?;
                limit = u64::from_le_bytes(sizes[..8].try_into().unwrap())
                    .checked_add(8)
                    .ok_or_else(|| invalid("RF64 size overflow"))?;
                if limit > file_end || limit < end {
                    return Err(invalid("truncated RF64"));
                }
                large_data = Some(u64::from_le_bytes(sizes[8..16].try_into().unwrap()));
                large_frames = Some(u64::from_le_bytes(sizes[16..24].try_into().unwrap()));
                let count = u64::from(u32_at(&sizes[24..28], false));
                if count > (size - 28) / 12 {
                    return Err(invalid("truncated ds64 table"));
                }
                table
                    .try_reserve_exact(count as usize)
                    .map_err(allocation)?;
                for _ in 0..count {
                    let mut entry = [0; 12];
                    reader.read_exact(&mut entry)?;
                    table.push((
                        entry[..4].try_into().unwrap(),
                        u64::from_le_bytes(entry[4..].try_into().unwrap()),
                    ));
                }
            }
            b"fmt " => {
                if !(16..=65536).contains(&size) || fmt.is_some() {
                    return Err(invalid("invalid fmt chunk"));
                }
                let mut bytes = vec![0; size as usize];
                reader.read_exact(&mut bytes)?;
                fmt = Some(bytes);
            }
            b"data" if data.is_none() => data = Some((begin, size)),
            b"fact" if size >= 4 => {
                let mut bytes = [0; 4];
                reader.read_exact(&mut bytes)?;
                fact = Some(u64::from(u32_at(&bytes, be)));
            }
            _ => {}
        }
        position = end
            .checked_add(size & 1)
            .ok_or_else(|| invalid("chunk padding overflow"))?;
    }
    if rf64 && large_data.is_none() {
        return Err(invalid("RF64 has no ds64"));
    }
    Ok(Wave {
        fmt: fmt.ok_or_else(|| invalid("missing fmt chunk"))?,
        data: data.ok_or_else(|| invalid("missing data chunk"))?,
        fact: large_frames
            .filter(|n| *n > 0)
            .or(fact.filter(|n| *n != u64::from(u32::MAX))),
        be,
        limit,
    })
}
/// No duration, sample-rate or channel-count admission limit. PCM/float retain
/// rate, gain and phase; mono is duplicated. Multichannel is folded to the mean
/// of all channels in both stereo lanes, the reference representation.
pub fn read_wav(reader: impl Read + Seek) -> Result<AudioBuffer, AnalysisError> {
    read_wav_cancellable(reader, &|| false)
}
pub fn read_wav_cancellable(
    mut reader: impl Read + Seek,
    cancel: &impl Fn() -> bool,
) -> Result<AudioBuffer, AnalysisError> {
    if cancel() {
        return Err(AnalysisError::Cancelled);
    }
    let wave = wave(&mut reader)?;
    let channels = usize::from(u16_at(&wave.fmt[2..4], wave.be));
    let rate = u32_at(&wave.fmt[4..8], wave.be);
    let stride = usize::from(u16_at(&wave.fmt[12..14], wave.be));
    let bits = usize::from(u16_at(&wave.fmt[14..16], wave.be));
    let mut tag = u16_at(&wave.fmt, wave.be);
    if channels == 0 || rate == 0 || wave.data.1 == 0 {
        return Err(AnalysisError::InvalidAudio);
    }
    if tag == 0xfffe {
        if wave.fmt.len() < 40 || u16_at(&wave.fmt[16..18], wave.be) < 22 {
            return Err(invalid("truncated WAVE_FORMAT_EXTENSIBLE"));
        }
        let valid = usize::from(u16_at(&wave.fmt[18..20], wave.be));
        if !bits.is_multiple_of(8) || valid > bits {
            return Err(invalid("invalid extensible valid bits"));
        }
        if wave.fmt[26..40] == [0, 0, 0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113] {
            tag = u16_at(&wave.fmt[24..26], false);
        }
    }
    reader.seek(SeekFrom::Start(wave.data.0))?;
    if tag == 1 || tag == 3 {
        let bytes = bits.div_ceil(8);
        if (tag == 1 && !(1..=64).contains(&bits))
            || (tag == 3 && ![32, 64].contains(&bits))
            || stride < channels * bytes
            || !stride.is_multiple_of(channels)
            || !wave.data.1.is_multiple_of(stride as u64)
        {
            return Err(invalid("invalid PCM/float frame layout"));
        }
        let count = usize::try_from(wave.data.1 / stride as u64).map_err(allocation)?;
        let channel_stride = stride / channels;
        let mut frames = Vec::new();
        frames.try_reserve_exact(count).map_err(allocation)?;
        let batch = (65536 / stride).max(1);
        let mut input = vec![0; batch * stride];
        let mut remaining = count;
        while remaining > 0 {
            if cancel() {
                return Err(AnalysisError::Cancelled);
            }
            let n = remaining.min(batch);
            reader.read_exact(&mut input[..n * stride])?;
            for frame in input[..n * stride].chunks_exact(stride) {
                let sample = |index: usize| -> f32 {
                    let offset =
                        index * channel_stride + if wave.be { channel_stride - bytes } else { 0 };
                    let source = &frame[offset..offset + bytes];
                    let mut raw = [0; 8];
                    if wave.be {
                        raw[8 - bytes..].copy_from_slice(source);
                    } else {
                        raw[..bytes].copy_from_slice(source);
                    }
                    let raw = if wave.be {
                        u64::from_be_bytes(raw)
                    } else {
                        u64::from_le_bytes(raw)
                    };
                    if tag == 3 {
                        if bits == 32 {
                            f32::from_bits(raw as u32)
                        } else {
                            f64::from_bits(raw) as f32
                        }
                    } else if bytes == 1 {
                        (raw as f32 - 128.0) / 128.0
                    } else {
                        // Valid PCM bits, including extensible 20-in-24, are left aligned.
                        let shift = 64 - bytes * 8;
                        let signed = ((raw << shift) as i64) >> shift;
                        (signed as f64 / 2.0_f64.powi((bytes * 8 - 1) as i32)) as f32
                    }
                };
                let stereo = if channels == 1 {
                    [sample(0); 2]
                } else if channels == 2 {
                    [sample(0), sample(1)]
                } else {
                    [((0..channels).map(|i| f64::from(sample(i))).sum::<f64>() / channels as f64)
                        as f32; 2]
                };
                if stereo.iter().any(|s| !s.is_finite()) {
                    return Err(AnalysisError::InvalidAudio);
                }
                frames.push(stereo);
            }
            remaining -= n;
        }
        return Ok(AudioBuffer {
            frames,
            sample_rate: rate,
        });
    }
    // Vorbis ACM block sizes are unrelated to PCM bits. data contains a complete
    // Ogg stream, excluding RIFF padding and trailing metadata chunks.
    let ogg = [0x674f, 0x6750, 0x6751, 0x676f, 0x6770, 0x6771].contains(&tag);
    if !ogg && ![2, 6, 7, 17, 0xfffe].contains(&tag) {
        return Err(invalid(&format!("unsupported WAVE codec 0x{tag:04X}")));
    }
    let mut bytes = Vec::new();
    let size = usize::try_from(if ogg { wave.data.1 } else { wave.limit }).map_err(allocation)?;
    bytes.try_reserve_exact(size).map_err(allocation)?;
    reader.seek(SeekFrom::Start(if ogg { wave.data.0 } else { 0 }))?;
    let mut remaining = size;
    let mut buffer = [0; 65536];
    while remaining > 0 {
        if cancel() {
            return Err(AnalysisError::Cancelled);
        }
        let count = remaining.min(buffer.len());
        reader.read_exact(&mut buffer[..count])?;
        bytes.extend_from_slice(&buffer[..count]);
        remaining -= count;
    }
    if ogg && !bytes.starts_with(b"OggS") {
        return Err(invalid("Vorbis ACM data has no complete Ogg headers"));
    }
    let mut audio = decode_audio(bytes, cancel).map_err(|error| match error {
        AnalysisError::Cancelled => AnalysisError::Cancelled,
        error => invalid(&format!("codec 0x{tag:04X}: {error}")),
    })?;
    if audio.sample_rate != rate {
        return Err(invalid("codec sample rate disagrees with WAVE header"));
    }
    if ogg && let Some(count) = wave.fact.filter(|n| *n > 0) {
        let count = usize::try_from(count).map_err(allocation)?;
        if count > audio.frames.len() {
            return Err(invalid("Vorbis stream is shorter than fact sample count"));
        }
        audio.frames.truncate(count);
    }
    Ok(audio)
}
/// Decode an in-memory Ogg/Vorbis stream with cancellation between packets.
pub fn decode_audio(
    bytes: Vec<u8>,
    cancel: &impl Fn() -> bool,
) -> Result<AudioBuffer, AnalysisError> {
    decode_stream(bytes, cancel, false)
}
/// SF3 stores one mono stream per sample header, including stereo-linked samples.
pub fn decode_mono_audio(
    bytes: Vec<u8>,
    cancel: &impl Fn() -> bool,
) -> Result<AudioBuffer, AnalysisError> {
    decode_stream(bytes, cancel, true)
}
fn decode_stream(
    bytes: Vec<u8>,
    cancel: &impl Fn() -> bool,
    mono: bool,
) -> Result<AudioBuffer, AnalysisError> {
    if cancel() {
        return Err(AnalysisError::Cancelled);
    }
    let source = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
    let mut format = symphonia::default::get_probe()
        .probe(&Hint::new(), source, Default::default(), Default::default())
        .map_err(|e| invalid(&e.to_string()))?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| invalid("no audio track"))?;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| invalid("missing audio codec parameters"))?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|e| invalid(&e.to_string()))?;
    let id = track.id;
    let mut audio = AudioBuffer {
        frames: Vec::new(),
        sample_rate: 0,
    };
    let mut samples = Vec::<f32>::new();
    while let Some(packet) = format.next_packet().map_err(|e| invalid(&e.to_string()))? {
        if cancel() {
            return Err(AnalysisError::Cancelled);
        }
        if packet.track_id != id {
            continue;
        }
        let decoded = decoder
            .decode(&packet)
            .map_err(|e| invalid(&e.to_string()))?;
        let rate = decoded.spec().rate();
        let channels = decoded.spec().channels().count();
        if mono && channels != 1 {
            return Err(invalid("SF3 sample stream must be mono"));
        }
        if rate == 0 || channels == 0 || (audio.sample_rate != 0 && audio.sample_rate != rate) {
            return Err(invalid("invalid or changing stream layout"));
        }
        audio.sample_rate = rate;
        samples.resize(decoded.samples_interleaved(), 0.0);
        decoded.copy_to_slice_interleaved(&mut samples);
        if samples.iter().any(|s| !s.is_finite()) {
            return Err(AnalysisError::InvalidAudio);
        }
        audio
            .frames
            .try_reserve(decoded.frames())
            .map_err(allocation)?;
        for frame in samples.chunks_exact(channels) {
            audio.frames.push(if channels == 1 {
                [frame[0]; 2]
            } else if channels == 2 {
                [frame[0], frame[1]]
            } else {
                [(frame.iter().map(|s| f64::from(*s)).sum::<f64>() / channels as f64) as f32; 2]
            });
        }
    }
    if audio.frames.is_empty() {
        return Err(AnalysisError::InvalidAudio);
    }
    Ok(audio)
}
