use discofm_analysis::{AnalysisOptions, analyze, read_wav};
use std::io::Cursor;

fn chunk(id: &[u8; 4], data: &[u8], be: bool) -> Vec<u8> {
    let mut bytes = id.to_vec();
    bytes.extend(if be {
        (data.len() as u32).to_be_bytes()
    } else {
        (data.len() as u32).to_le_bytes()
    });
    bytes.extend(data);
    if data.len() % 2 == 1 {
        bytes.push(0);
    }
    bytes
}
fn wrap(
    tag: u16,
    channels: u16,
    rate: u32,
    bits: u16,
    stride: u16,
    data: &[u8],
    be: bool,
) -> Vec<u8> {
    let mut fmt = Vec::new();
    for bytes in [
        if be {
            tag.to_be_bytes()
        } else {
            tag.to_le_bytes()
        },
        if be {
            channels.to_be_bytes()
        } else {
            channels.to_le_bytes()
        },
    ] {
        fmt.extend(bytes);
    }
    for value in [rate, rate * u32::from(stride)] {
        fmt.extend(if be {
            value.to_be_bytes()
        } else {
            value.to_le_bytes()
        });
    }
    for value in [stride, bits] {
        fmt.extend(if be {
            value.to_be_bytes()
        } else {
            value.to_le_bytes()
        });
    }
    if tag != 1 && tag != 3 {
        fmt.extend([0, 0]);
    }
    let mut body = b"WAVE".to_vec();
    body.extend(chunk(b"JUNK", &[1, 2, 3], be));
    body.extend(chunk(b"fmt ", &fmt, be));
    body.extend(chunk(b"data", data, be));
    let mut bytes = if be {
        b"RIFX".to_vec()
    } else {
        b"RIFF".to_vec()
    };
    bytes.extend(if be {
        (body.len() as u32).to_be_bytes()
    } else {
        (body.len() as u32).to_le_bytes()
    });
    bytes.extend(body);
    bytes
}

#[test]
fn vorbis_acm_ignores_pcm_block_layout_and_respects_fact_and_riff_padding() {
    let ogg = include_bytes!("fixtures/sine.ogg");
    let mut bytes = wrap(0x674f, 2, 44100, 16, 1, ogg, false);
    bytes.extend(chunk(b"fact", &8192_u32.to_le_bytes(), false));
    bytes.extend(chunk(b"LIST", b"OggS metadata is not audio", false));
    let size = (bytes.len() - 8) as u32;
    bytes[4..8].copy_from_slice(&size.to_le_bytes());
    let mut ds64 = vec![0; 40];
    ds64[8..16].copy_from_slice(&(ogg.len() as u64).to_le_bytes());
    ds64[16..24].copy_from_slice(&8192_u64.to_le_bytes());
    ds64[24..28].copy_from_slice(&1_u32.to_le_bytes());
    ds64[28..32].copy_from_slice(b"JUNK");
    ds64[32..40].copy_from_slice(&3_u64.to_le_bytes());
    let mut chunks = bytes[12..].to_vec();
    chunks[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    let data_header = chunks.windows(4).position(|b| b == b"data").unwrap();
    chunks[data_header + 4..data_header + 8].copy_from_slice(&u32::MAX.to_le_bytes());
    let fact = chunks.windows(4).position(|b| b == b"fact").unwrap();
    chunks[fact + 8..fact + 12].copy_from_slice(&u32::MAX.to_le_bytes());
    let mut rf = b"RF64".to_vec();
    rf.extend(u32::MAX.to_le_bytes());
    rf.extend(b"WAVE");
    rf.extend(chunk(b"ds64", &ds64, false));
    rf.extend(chunks);
    let size = rf.len() as u64 - 8;
    rf[20..28].copy_from_slice(&size.to_le_bytes());
    let from_rf64 = read_wav(Cursor::new(rf)).unwrap();
    let audio = read_wav(Cursor::new(bytes)).unwrap();
    assert_eq!(audio, from_rf64);
    assert_eq!((audio.sample_rate, audio.frames.len()), (44100, 8192));
    let peak = audio
        .frames
        .iter()
        .map(|s| s[0].abs())
        .fold(0.0_f32, f32::max);
    assert!(peak > 0.1 && peak < 0.2);
    let error = audio
        .frames
        .iter()
        .map(|s| (s[1] - s[0] * 0.5).abs())
        .fold(0.0_f32, f32::max);
    assert!(error < 0.01, "{error}");
    let result = analyze(&audio, &AnalysisOptions::default()).unwrap();
    assert!((result.pitch.unwrap().hz - 330.0).abs() < 2.0);
}

#[test]
fn float64_rifx_rf64_extensible_and_adpcm_decode() {
    let padded = wrap(
        1,
        2,
        44100,
        24,
        8,
        &[0, 0, 0x40, 0, 0, 0, 0xc0, 0xff],
        false,
    );
    assert_eq!(
        read_wav(Cursor::new(padded)).unwrap().frames,
        vec![[0.5, -0.5]]
    );
    let float = wrap(3, 1, 384000, 64, 8, &0.25_f64.to_le_bytes(), false);
    assert_eq!(
        read_wav(Cursor::new(float)).unwrap().frames,
        vec![[0.25; 2]]
    );
    let be = wrap(1, 2, 4000, 16, 4, &[0x40, 0, 0xc0, 0], true);
    assert_eq!(read_wav(Cursor::new(be)).unwrap().frames, vec![[0.5, -0.5]]);
    let mut fmt = Vec::new();
    fmt.extend(0xfffe_u16.to_le_bytes());
    fmt.extend(1_u16.to_le_bytes());
    fmt.extend(48000_u32.to_le_bytes());
    fmt.extend(144000_u32.to_le_bytes());
    fmt.extend(3_u16.to_le_bytes());
    fmt.extend(24_u16.to_le_bytes());
    fmt.extend(22_u16.to_le_bytes());
    fmt.extend(20_u16.to_le_bytes());
    fmt.extend(4_u32.to_le_bytes());
    fmt.extend([1, 0, 0, 0, 0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113]);
    let mut body = b"WAVE".to_vec();
    let mut ds64 = vec![0; 28];
    ds64[8..16].copy_from_slice(&3_u64.to_le_bytes());
    body.extend(chunk(b"ds64", &ds64, false));
    body.extend(chunk(b"fmt ", &fmt, false));
    body.extend(b"data");
    body.extend(u32::MAX.to_le_bytes());
    body.extend([0, 0, 0x40, 0]);
    let mut rf = b"RF64".to_vec();
    rf.extend(u32::MAX.to_le_bytes());
    rf.extend(body);
    let size = rf.len() as u64 - 8;
    rf[20..28].copy_from_slice(&size.to_le_bytes());
    assert_eq!(read_wav(Cursor::new(rf)).unwrap().frames, vec![[0.5; 2]]);
    let ima = read_wav(Cursor::new(include_bytes!("fixtures/ima.wav"))).unwrap();
    assert_eq!(ima.sample_rate, 22050);
    assert!(ima.frames.len() >= 22050 / 10);
    assert!(ima.frames.iter().any(|s| s[0].abs() > 0.1));
    for tag in [6, 7] {
        let law = read_wav(Cursor::new(wrap(tag, 1, 8000, 8, 1, &[0, 0xff], false))).unwrap();
        assert_eq!((law.sample_rate, law.frames.len()), (8000, 2));
        assert!(law.frames.iter().all(|s| s[0].is_finite()));
    }
}

#[test]
fn long_multichannel_reference_opens_and_selection_can_start_after_thirty_seconds() {
    let frame = [16384_i16, 0, -8192]
        .into_iter()
        .flat_map(i16::to_le_bytes)
        .collect::<Vec<_>>();
    let data = frame.repeat(4000 * 65);
    let audio = read_wav(Cursor::new(wrap(1, 3, 4000, 16, 6, &data, false))).unwrap();
    assert_eq!(audio.frames.len(), 260000);
    assert!((audio.frames[0][0] - 1.0 / 12.0).abs() < 1e-6);
    let options = AnalysisOptions {
        start_seconds: 60.0,
        end_seconds: Some(60.2),
        trim_silence: false,
        ..Default::default()
    };
    let result = analyze(&audio, &options).unwrap();
    assert_eq!(result.original_seconds, 65.0);
    assert_eq!(result.region_start_seconds, 60.0);
    assert_eq!(result.region_end_seconds, 60.2);
    let result = analyze(&audio, &AnalysisOptions::default()).unwrap();
    assert!(result.region_end_seconds <= 30.0);
    let mut truncated = wrap(1, 1, 48000, 16, 2, &[0, 1], false);
    truncated.pop();
    assert!(read_wav(Cursor::new(truncated)).is_err());
    assert!(
        AnalysisOptions {
            start_seconds: 60.0,
            end_seconds: Some(91.0),
            ..Default::default()
        }
        .validate()
        .is_err()
    );
}

#[test]
#[ignore = "requires an installed FL Studio sample; set DISCOFM_TEST_WAV"]
fn fl_studio_hit2_vorbis_acm_opens_with_its_declared_duration() {
    let file = std::fs::File::open(std::env::var("DISCOFM_TEST_WAV").unwrap()).unwrap();
    let audio = read_wav(file).unwrap();
    assert_eq!((audio.sample_rate, audio.frames.len()), (44100, 80273));
    assert!(audio.frames.iter().flatten().all(|s| s.is_finite()));
    assert!(audio.frames.iter().flatten().any(|s| s.abs() > 0.1));
    let result = analyze(&audio, &AnalysisOptions::default()).unwrap();
    println!(
        "HIT_2: {} frames, {:.6} s, peak {}, pitch {:?}",
        audio.frames.len(),
        result.original_seconds,
        result.peak,
        result.pitch
    );
}

#[test]
#[ignore = "requires DISCOFM_TEST_MANDOLIN_WAV pointing to the installed FL Studio sample"]
fn fl_studio_mandolin_pitch_refines_decay_bias_without_an_octave_jump() {
    let file = std::fs::File::open(std::env::var("DISCOFM_TEST_MANDOLIN_WAV").unwrap()).unwrap();
    let audio = read_wav(file).unwrap();
    assert_eq!((audio.sample_rate, audio.frames.len()), (44100, 41773));
    let result = analyze(
        &audio,
        &AnalysisOptions {
            normalize: true,
            fft_size: 8192,
            ..Default::default()
        },
    )
    .unwrap();
    let pitch = result.pitch.expect("tonal mandolin sample");
    println!("STR_MandoLynn_B2: pitch {pitch:?}");
    assert!((pitch.hz - 247.091).abs() < 0.5, "{pitch:?}");
}
