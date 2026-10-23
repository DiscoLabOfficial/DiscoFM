use discofm_audio::{StreamingResampler, resample};
use discofm_engine::ClockConfig;

#[test]
fn streaming_sinc_matches_offline_at_every_output_rate_with_irregular_chunks() {
    for clock in [ClockConfig::Ntsc, ClockConfig::Pal] {
        for rate in [8000, 44100, 48000, 96000, 192000] {
            let source = clock.native_hz();
            let frames = (0..1800)
                .map(|i| {
                    let wave = (std::f64::consts::TAU * 4200.0 * i as f64 / source).sin() as f32;
                    [if i == 350 { 1.0 } else { wave * 0.2 + 0.1 }, -wave * 0.3]
                })
                .collect::<Vec<_>>();
            let expected = resample(&frames, source, rate).unwrap();
            let mut converter = StreamingResampler::new(source, rate).unwrap();
            let mut actual = Vec::new();
            let mut output = [[0.0; 2]; 137];
            let mut start = 0;
            for count in [0, 1, 23, 24, 25, 333, 0, 511, 883] {
                let end = (start + count).min(frames.len());
                converter.push(&frames[start..end]).unwrap();
                start = end;
                loop {
                    let count = converter.read_into(&mut output);
                    actual.extend_from_slice(&output[..count]);
                    if count == 0 {
                        break;
                    }
                }
            }
            assert_eq!(start, frames.len());
            converter.finish();
            loop {
                let count = converter.read_into(&mut output);
                actual.extend_from_slice(&output[..count]);
                if count == 0 {
                    break;
                }
            }
            assert_eq!(actual.len(), expected.frames.len());
            assert_eq!(
                actual
                    .iter()
                    .zip(&expected.frames)
                    .position(|(a, b)| a != b),
                None,
                "{clock:?} at {rate} Hz"
            );
        }
    }
}

#[test]
fn streaming_storage_is_bounded_for_long_holds_and_reset_removes_history() {
    let mut converter = StreamingResampler::new(53267.03869, 48000).unwrap();
    let mut output = [[0.0; 2]; 256];
    for _ in 0..2000 {
        converter.push(&[[0.25, -0.25]; 128]).unwrap();
        loop {
            let count = converter.read_into(&mut output);
            assert!(
                output[..count]
                    .iter()
                    .all(|frame| (frame[0] - 0.25).abs() < 1e-6 && (frame[1] + 0.25).abs() < 1e-6)
            );
            if count == 0 {
                break;
            }
        }
        assert!(converter.pending_frames() < 256);
    }
    converter.reset();
    assert_eq!(converter.pending_frames(), 0);
    assert_eq!(converter.read_into(&mut output), 0);
    converter.push(&[[0.0; 2]; 128]).unwrap();
    converter.finish();
    let count = converter.read_into(&mut output);
    assert!(count > 0);
    assert!(output[..count].iter().flatten().all(|v| *v == 0.0));
    assert!(converter.push(&[]).is_err());
    let mut converter = StreamingResampler::new(53267.0, 8000).unwrap();
    assert!(converter.push(&vec![[0.0; 2]; 10000]).is_err());
    assert!(converter.push(&[[f32::NAN, 0.0]]).is_err());
}
