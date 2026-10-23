use discofm_soundfont::{BankError, SoundBank, read_reference};
use std::{
    fs,
    io::Cursor,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
static COUNTER: AtomicUsize = AtomicUsize::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "discofm-bank-{}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn file(&self, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let p = self.0.join(name);
        fs::write(&p, bytes).unwrap();
        p
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        assert_eq!(self.0.parent().unwrap(), std::env::temp_dir());
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn chunk(id: &[u8; 4], b: &[u8]) -> Vec<u8> {
    let mut v = id.to_vec();
    v.extend((b.len() as u32).to_le_bytes());
    v.extend(b);
    if !b.len().is_multiple_of(2) {
        v.push(0);
    }
    v
}
fn list(kind: &[u8; 4], b: &[u8]) -> Vec<u8> {
    let mut v = kind.to_vec();
    v.extend(b);
    chunk(b"LIST", &v)
}
fn riff(kind: &[u8; 4], b: &[u8]) -> Vec<u8> {
    let mut v = kind.to_vec();
    v.extend(b);
    chunk(b"RIFF", &v)
}
fn shdr(
    name: &str,
    start: u32,
    end: u32,
    loop_range: [u32; 2],
    rate: u32,
    link: u16,
    kind: u16,
) -> Vec<u8> {
    let mut b = vec![0; 20];
    let n = name.len().min(20);
    b[..n].copy_from_slice(&name.as_bytes()[..n]);
    for n in [start, end, loop_range[0], loop_range[1], rate] {
        b.extend(n.to_le_bytes());
    }
    b.extend([60, 0]);
    b.extend(link.to_le_bytes());
    b.extend(kind.to_le_bytes());
    b
}
fn sf2(smpl: &[u8], sm24: Option<&[u8]>, headers: &[u8]) -> Vec<u8> {
    let mut data = chunk(b"smpl", smpl);
    if let Some(b) = sm24 {
        data.extend(chunk(b"sm24", b));
    }
    let mut h = headers.to_vec();
    h.extend(shdr("EOS", 0, 0, [0, 0], 0, 0, 0));
    let mut body = list(b"sdta", &data);
    body.extend(list(b"pdta", &chunk(b"shdr", &h)));
    riff(b"sfbk", &body)
}
fn pcm(values: &[i16]) -> Vec<u8> {
    values.iter().flat_map(|s| s.to_le_bytes()).collect()
}
fn wave(samples: &[i16], channels: u16) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(
            &mut cursor,
            hound::WavSpec {
                channels,
                sample_rate: 22050,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for s in samples {
            w.write_sample(*s).unwrap();
        }
        w.finalize().unwrap();
    }
    cursor.into_inner()
}
#[test]
fn sf2_pcm16_24_offsets_loops_and_no_guard_samples() {
    let t = Temp::new();
    let words = pcm(&[999, -32768, -1, 0, 32767, 777, 888]);
    let h = shdr("First", 1, 5, [2, 4], 22050, 0, 1);
    let p = t.file("pcm.sf2", sf2(&words, None, &h));
    let b = SoundBank::open(&p, &|| false).unwrap();
    assert_eq!(b.samples.len(), 1);
    assert_eq!(b.samples[0].loop_frames, Some([1, 3]));
    assert_eq!(b.samples[0].root_key, Some(60));
    assert_eq!(
        b.read_sample(0, &|| false).unwrap().frames,
        vec![[-1.; 2], [-1. / 32768.; 2], [0.; 2], [32767. / 32768.; 2]]
    );
    let low = [0, 0, 255, 1, 255, 0, 0];
    let p = t.file("24.sf2", sf2(&words, Some(&low), &h));
    let b = SoundBank::open(p, &|| false).unwrap();
    assert_eq!(
        b.read_sample(0, &|| false).unwrap().frames,
        vec![
            [-1.; 2],
            [-1. / 8388608.; 2],
            [1. / 8388608.; 2],
            [8388607. / 8388608.; 2]
        ]
    );
}
#[test]
fn sf2_stereo_orientation_duplicate_names_invalid_link_and_rom() {
    let t = Temp::new();
    let mut h = shdr("same", 0, 2, [0, 2], 22050, 1, 2);
    h.extend(shdr("same", 2, 4, [2, 4], 22050, 0, 4));
    h.extend(shdr("ROM", 0, 2, [0, 0], 22050, 0, 0x8001));
    let p = t.file("pair.sf2", sf2(&pcm(&[1000, 2000, -1000, -2000]), None, &h));
    let b = SoundBank::open(p, &|| false).unwrap();
    assert_eq!(b.samples[0].channels, 2);
    let expected = vec![
        [-1000. / 32768., 1000. / 32768.],
        [-2000. / 32768., 2000. / 32768.],
    ];
    assert_eq!(b.read_sample(0, &|| false).unwrap().frames, expected);
    assert_eq!(b.read_sample(1, &|| false).unwrap().frames, expected);
    assert!(b.read_sample(2, &|| false).is_err());
    assert_ne!(b.selection(0).unwrap(), b.selection(1).unwrap());
    let p = t.file(
        "badlink.sf2",
        sf2(
            &pcm(&[1, 2]),
            None,
            &shdr("mono", 0, 2, [0, 2], 22050, 99, 4),
        ),
    );
    let b = SoundBank::open(p, &|| false).unwrap();
    assert_eq!(b.samples[0].channels, 1);
    assert!(b.samples[0].warning.is_some());
    assert_eq!(b.read_sample(0, &|| false).unwrap().frames.len(), 2);
}
#[test]
fn riff_bounds_and_short_sm24_fail_visibly() {
    let t = Temp::new();
    let bytes = sf2(&pcm(&[1, 2]), None, &shdr("x", 0, 2, [0, 0], 22050, 0, 1));
    for n in [0, 8, bytes.len() - 3] {
        let p = t.file(&format!("truncated{n}.sf2"), &bytes[..n]);
        assert!(SoundBank::open(p, &|| false).is_err());
    }
    let p = t.file(
        "short.sf2",
        sf2(
            &pcm(&[1, 2]),
            Some(&[0]),
            &shdr("x", 0, 2, [0, 0], 22050, 0, 1),
        ),
    );
    let b = SoundBank::open(p, &|| false).unwrap();
    assert!(b.read_sample(0, &|| false).is_err());
    let p = t.file(
        "offset.sf2",
        sf2(
            &pcm(&[1, 2]),
            None,
            &shdr("x", 0, u32::MAX, [0, 0], 22050, 0, 1),
        ),
    );
    let b = SoundBank::open(p, &|| false).unwrap();
    assert!(b.read_sample(0, &|| false).is_err());
}
#[test]
fn sf3_inclusive_exclusive_ends_and_mixed_pcm() {
    let t = Temp::new();
    let ogg = include_bytes!("fixtures/tone.ogg");
    for inclusive in [false, true] {
        let h = shdr(
            "Vorbis",
            0,
            ogg.len() as u32 - u32::from(inclusive),
            [10, 100],
            22050,
            0,
            0x11,
        );
        let p = t.file(&format!("{inclusive}.sf3"), sf2(ogg, None, &h));
        let b = SoundBank::open(p, &|| false).unwrap();
        assert_eq!(b.format, "SF3");
        let a = b.read_sample(0, &|| false).unwrap();
        assert_eq!(a.sample_rate, 22050);
        assert!(a.frames.len() >= 1764);
        assert!(a.frames.iter().any(|f| f[0].abs() > 0.1));
        assert!(a.frames.iter().all(|f| f[0] == f[1]));
    }
    let mut mixed = pcm(&[-1234, 2345]);
    mixed.extend(ogg);
    let mut h = shdr("PCM", 0, 2, [0, 0], 22050, 0, 1);
    h.extend(shdr("OGG", 4, mixed.len() as u32, [0, 0], 22050, 0, 0x11));
    let p = t.file("mixed.sf3", sf2(&mixed, None, &h));
    let b = SoundBank::open(p, &|| false).unwrap();
    assert_eq!(b.read_sample(0, &|| false).unwrap().frames.len(), 2);
    assert!(b.read_sample(1, &|| false).unwrap().frames.len() > 1000);
}
#[test]
fn sf3_missing_padding_and_truncated_stream() {
    let t = Temp::new();
    let ogg = include_bytes!("fixtures/tone.ogg");
    let mut smpl = ogg.to_vec();
    smpl.push(0);
    if smpl.len().is_multiple_of(2) {
        smpl.push(0);
    } // force odd data
    let mut sdta = b"sdta".to_vec();
    sdta.extend(b"smpl");
    sdta.extend((smpl.len() as u32).to_le_bytes());
    sdta.extend(&smpl);
    let mut body = b"LIST".to_vec();
    body.extend((sdta.len() as u32).to_le_bytes());
    body.extend(sdta); // deliberately omit both paddings
    let mut h = shdr("OGG", 0, ogg.len() as u32, [0, 0], 22050, 0, 0x11);
    h.extend(shdr("EOS", 0, 0, [0, 0], 0, 0, 0));
    body.extend(list(b"pdta", &chunk(b"shdr", &h)));
    let p = t.file("odd.sf3", riff(b"sfbk", &body));
    let b = SoundBank::open(p, &|| false).unwrap();
    assert!(b.read_sample(0, &|| false).is_ok());
    let p = t.file(
        "short.sf3",
        sf2(
            &ogg[..ogg.len() - 30],
            None,
            &shdr("OGG", 0, (ogg.len() - 30) as u32, [0, 0], 22050, 0, 0x11),
        ),
    );
    let b = SoundBank::open(p, &|| false).unwrap();
    assert!(b.read_sample(0, &|| false).is_err());
}
#[test]
fn sfz_paths_inheritance_macros_includes_ranges_and_cycles() {
    let t = Temp::new();
    fs::create_dir(t.0.join("Samples")).unwrap();
    t.file("Samples/my tone.wav", wave(&[100, 200, 300, 400, 500], 1));
    t.file("regions.inc",b"<group> pitch_keycenter=B2\n<region> sample=$tone offset=1 end=3 loop_start=1 loop_end=2 region_label=Mandolin test");
    let p=t.file("bank.sfz",b"\xef\xbb\xbf#define $tone my tone.wav\n<control> default_path=Samples/\n// comment\n#include \"regions.inc\"\n<region> sample=*sine");
    let b = SoundBank::open(&p, &|| false).unwrap();
    assert_eq!(b.samples.len(), 2);
    assert_eq!(b.samples[0].name, "Mandolin test");
    assert_eq!(b.samples[0].root_key, Some(47));
    assert_eq!(b.samples[0].loop_frames, Some([0, 2]));
    assert_eq!(
        b.read_sample(0, &|| false).unwrap().frames,
        vec![[200. / 32768.; 2], [300. / 32768.; 2], [400. / 32768.; 2]]
    );
    assert!(b.read_sample(1, &|| false).is_err());
    let p = t.file("cycle.sfz", b"#include \"cycle.sfz\"");
    assert!(SoundBank::open(p, &|| false).is_err());
    let p = t.file("missing.sfz", b"<region> sample=missing.wav");
    let b = SoundBank::open(p, &|| false).unwrap();
    assert!(b.samples[0].warning.is_some());
    assert!(b.read_sample(0, &|| false).is_err());
}
#[test]
fn sf2_loop_starting_before_the_sample_is_dropped() {
    let t = Temp::new();
    let words = pcm(&[1, 2, 3, 4, 5, 6]);
    let p = t.file(
        "early-loop.sf2",
        sf2(&words, None, &shdr("early", 3, 6, [1, 5], 22050, 0, 1)),
    );
    let b = SoundBank::open(&p, &|| false).unwrap();
    assert_eq!(b.samples[0].frames, Some(3));
    assert_eq!(b.samples[0].loop_frames, None);
}
#[test]
fn dls_wave_pool_stereo_wsmp_and_odd_info() {
    let t = Temp::new();
    let wav = wave(&[100, -200, 300, -400], 2);
    let mut wave_body = wav[12..].to_vec();
    let mut wsmp = 20u32.to_le_bytes().to_vec();
    wsmp.extend(64u16.to_le_bytes());
    wsmp.extend((-12i16).to_le_bytes());
    wsmp.extend([0; 8]);
    wsmp.extend(1u32.to_le_bytes());
    wsmp.extend(16u32.to_le_bytes());
    wsmp.extend(0u32.to_le_bytes());
    wsmp.extend(0u32.to_le_bytes());
    wsmp.extend(2u32.to_le_bytes());
    wave_body.extend(chunk(b"wsmp", &wsmp));
    wave_body.extend(list(b"INFO", &chunk(b"INAM", b"Ab\0")));
    let p = t.file(
        "waves.dls",
        riff(b"DLS ", &list(b"wvpl", &list(b"wave", &wave_body))),
    );
    let b = SoundBank::open(p, &|| false).unwrap();
    assert_eq!(b.samples[0].name, "Ab");
    assert_eq!(b.samples[0].root_key, Some(64));
    assert_eq!(b.samples[0].pitch_correction, -12);
    assert_eq!(b.samples[0].loop_frames, Some([0, 2]));
    assert_eq!(
        b.read_sample(0, &|| false).unwrap().frames,
        vec![
            [100. / 32768., -200. / 32768.],
            [300. / 32768., -400. / 32768.]
        ]
    );
}
#[test]
fn selection_restore_rejects_changed_catalog_and_cancels() {
    let t = Temp::new();
    let p = t.file(
        "bank.sf2",
        sf2(
            &pcm(&[1; 20000]),
            None,
            &shdr("One", 0, 20000, [0, 0], 22050, 0, 1),
        ),
    );
    let b = SoundBank::open(&p, &|| false).unwrap();
    let selection = b.selection(0).unwrap();
    assert!(selection.validate());
    assert_eq!(
        read_reference(&p, Some(&selection), &|| false)
            .unwrap()
            .frames
            .len(),
        20000
    );
    assert!(read_reference(&p, None, &|| false).is_err());
    assert!(matches!(
        SoundBank::open(&p, &|| true),
        Err(BankError::Cancelled)
    ));
    let calls = AtomicUsize::new(0);
    assert!(matches!(
        b.read_sample(0, &|| calls.fetch_add(1, Ordering::Relaxed) > 1),
        Err(BankError::Cancelled)
    ));
    t.file(
        "bank.sf2",
        sf2(
            &pcm(&[1; 20000]),
            None,
            &shdr("Changed", 0, 20000, [0, 0], 22050, 0, 1),
        ),
    );
    assert!(read_reference(&p, Some(&selection), &|| false).is_err());
    assert!(b.read_sample(99, &|| false).is_err());
}
#[test]
#[ignore = "Opt-in external soundfont; set DISCOFM_TEST_SOUNDFONT"]
fn installed_soundfont_decodes_every_available_sample() {
    let path = std::env::var_os("DISCOFM_TEST_SOUNDFONT").expect("Set DISCOFM_TEST_SOUNDFONT");
    let b = SoundBank::open(PathBuf::from(path), &|| false).unwrap();
    let mut decoded = 0;
    for sample in &b.samples {
        if sample.warning.is_none() {
            let audio = b.read_sample(sample.index, &|| false).unwrap();
            assert!(!audio.frames.is_empty());
            assert!(audio.frames.iter().flatten().all(|v| v.is_finite()));
            decoded += 1;
        }
    }
    assert!(decoded > 0);
    println!(
        "{}: {decoded}/{} samples decodificados",
        b.format,
        b.samples.len()
    );
}

#[test]
fn sfz_master_silent_region_and_invalid_unicode_are_explicit() {
    let t = Temp::new();
    t.file("a.wav", wave(&[100, 200, 300, 400], 1));
    let p = t.file(
        "master.sfz",
        b"<global> sample=a.wav <master> offset=2 <group><region> end=3 <region> end=-1",
    );
    let b = SoundBank::open(p, &|| false).unwrap();
    assert_eq!(b.read_sample(0, &|| false).unwrap().frames.len(), 2);
    assert!(b.read_sample(1, &|| false).is_err());
    let p = t.file("unicode.sfz", format!("@{}é", "a".repeat(28)));
    assert!(SoundBank::open(p, &|| false).is_err());
}
#[test]
fn sf3_zero_terminal_is_accepted_and_cached_bank_detects_replacement() {
    let t = Temp::new();
    let mut bytes = sf2(&pcm(&[1, 2]), None, &shdr("x", 0, 2, [0, 0], 22050, 0, 1));
    let length = bytes.len();
    bytes[length - 46..].fill(0);
    let p = t.file("zero.sf3", bytes);
    let b = SoundBank::open(&p, &|| false).unwrap();
    let selected = b.selection(0).unwrap();
    assert_eq!(
        b.read_selection(&selected, &|| false).unwrap().frames.len(),
        2
    );
    t.file(
        "zero.sf3",
        sf2(
            &pcm(&[1, 2, 3]),
            None,
            &shdr("new", 0, 3, [0, 0], 22050, 0, 1),
        ),
    );
    assert!(b.read_selection(&selected, &|| false).is_err());
}

#[test]
fn sf3_rejects_dual_mono_stereo_streams_before_folding_channels() {
    let t = Temp::new();
    let ogg = include_bytes!("fixtures/stereo.ogg");
    let p = t.file(
        "stereo.sf3",
        sf2(
            ogg,
            None,
            &shdr("invalid", 0, ogg.len() as u32, [0, 0], 22050, 0, 0x11),
        ),
    );
    let b = SoundBank::open(p, &|| false).unwrap();
    assert!(
        b.read_sample(0, &|| false)
            .unwrap_err()
            .to_string()
            .contains("mono")
    );
}
