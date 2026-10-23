use discofm_analysis::{AnalysisOptions, ChannelMode, analyze, analyze_cancellable, read_wav};
use discofm_audio::AudioBuffer;
use std::{f64::consts::TAU, io::Cursor};

fn tone(rate: u32, hz: f64, seconds: f64) -> AudioBuffer {
    AudioBuffer {
        sample_rate: rate,
        frames: (0..(f64::from(rate) * seconds) as usize)
            .map(|i| {
                let sample = (0.4 * (TAU * hz * i as f64 / f64::from(rate)).sin()) as f32;
                [sample; 2]
            })
            .collect(),
    }
}
#[test]
fn pitch_spectrum_harmonics_and_centroid_are_measured_at_multiple_rates() {
    for rate in [8000, 44100, 48000, 96000, 192000] {
        for hz in [20.0, 55.0, 220.0, 440.0, 997.0, 2000.0, 4000.0] {
            if hz >= f64::from(rate) * 0.45 {
                continue;
            }
            let audio = tone(rate, hz, 0.45);
            let original = audio.clone();
            let result = analyze(
                &audio,
                &AnalysisOptions {
                    fft_size: 4096,
                    trim_silence: false,
                    ..AnalysisOptions::default()
                },
            )
            .unwrap();
            let estimate = result.pitch.as_ref().expect("periodic pitch");
            assert!(
                (estimate.hz - hz).abs() / hz < 0.012,
                "{rate} Hz / {hz} tone: {:?}",
                estimate
            );
            assert!(estimate.confidence > 0.8);
            assert_eq!(audio, original);
            assert!(!result.harmonics.is_empty());
            let peak = result
                .spectrum
                .magnitudes
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .unwrap()
                .0;
            assert!(
                (f64::from(result.spectrum.frequencies[peak]) - hz).abs()
                    < f64::from(rate) / 4096.0 * 1.6
            );
            assert_eq!(
                result.spectrogram.db.len(),
                result.spectrogram.times.len() * result.spectrogram.frequencies.len()
            );
            assert!(result.spectrogram.db.iter().all(|value| value.is_finite()));
        }
    }
    // Bin-centred sine: Hann coherent gain must preserve peak amplitude.
    let result = analyze(
        &tone(48000, 750.0, 0.5),
        &AnalysisOptions {
            fft_size: 4096,
            trim_silence: false,
            ..AnalysisOptions::default()
        },
    )
    .unwrap();
    assert!((result.spectrum.magnitudes[64] - 0.4).abs() < 0.002);
    assert!((result.spectral[3].centroid_hz - 750.0).abs() < 5.0);
    assert!((result.rms - 0.4_f32 / 2.0_f32.sqrt()).abs() < 0.002);
}
#[test]
fn selected_dc_extreme_float_and_mid_analysis_cancellation_are_explicit() {
    let mut audio = AudioBuffer {
        sample_rate: 8000,
        frames: vec![[0.5; 2]; 8000],
    };
    audio.frames[4000..].fill([-0.5; 2]);
    let original = audio.clone();
    let result = analyze(
        &audio,
        &AnalysisOptions {
            end_seconds: Some(0.5),
            trim_silence: false,
            ..AnalysisOptions::default()
        },
    )
    .unwrap();
    assert_eq!(result.peak, 0.0);
    assert_eq!(result.dc, [0.0; 2]);
    assert_eq!(audio, original);
    audio.frames.fill([1e37; 2]);
    assert!(
        analyze(
            &audio,
            &AnalysisOptions {
                remove_dc: false,
                ..AnalysisOptions::default()
            }
        )
        .is_err()
    );
    let calls = std::cell::Cell::new(0);
    let result = analyze_cancellable(&tone(48000, 55.0, 1.0), &AnalysisOptions::default(), || {
        calls.set(calls.get() + 1);
        calls.get() > 10
    });
    assert!(matches!(
        result,
        Err(discofm_analysis::AnalysisError::Cancelled)
    ));
    assert!(calls.get() > 10);
}
#[test]
fn yin_recovers_fundamental_with_a_dominant_second_harmonic() {
    let audio = AudioBuffer {
        sample_rate: 48000,
        frames: (0..24000)
            .map(|i| {
                let phase = TAU * 220.0 * i as f64 / 48000.0;
                let v = (0.03 * phase.sin() + 0.4 * (phase * 2.0).sin() + 0.2 * (phase * 3.0).sin())
                    as f32;
                [v; 2]
            })
            .collect(),
    };
    let result = analyze(&audio, &AnalysisOptions::default()).unwrap();
    let pitch = result.pitch.unwrap();
    assert!((pitch.hz - 220.0).abs() < 2.0, "{pitch:?}");
}
#[test]
fn silence_noise_dc_channels_trimming_and_manual_pitch_have_explicit_results() {
    let silent = AudioBuffer {
        sample_rate: 48000,
        frames: vec![[0.0; 2]; 4800],
    };
    let result = analyze(&silent, &AnalysisOptions::default()).unwrap();
    assert!(result.pitch.is_none());
    assert_eq!(result.peak, 0.0);
    let mut audio = tone(48000, 440.0, 0.5);
    for (i, frame) in audio.frames.iter_mut().enumerate() {
        frame[0] += 0.1;
        frame[1] = (0.2 * (TAU * 660.0 * i as f64 / 48000.0).sin()) as f32 - 0.2;
    }
    let original = audio.clone();
    let result = analyze(
        &audio,
        &AnalysisOptions {
            channel: ChannelMode::Right,
            normalize: true,
            pitch_override_hz: Some(330.0),
            ..AnalysisOptions::default()
        },
    )
    .unwrap();
    assert!((result.pitch.unwrap().hz - 660.0).abs() < 3.0);
    assert_eq!(result.used_pitch_hz, Some(330.0));
    assert!((result.dc[0] - 0.1).abs() < 0.001 && (result.dc[1] + 0.2).abs() < 0.001);
    assert!((result.normalization_gain - 5.0).abs() < 0.01);
    assert_eq!(audio, original);
    let mut delayed = tone(48000, 440.0, 0.2);
    delayed.frames.splice(0..0, vec![[0.0; 2]; 4800]);
    delayed.frames.extend(vec![[0.0; 2]; 4800]);
    let result = analyze(&delayed, &AnalysisOptions::default()).unwrap();
    assert!((result.region_start_seconds - 0.1).abs() < 0.001);
    assert!((result.region_end_seconds - 0.3).abs() < 0.001);
    assert!(
        result
            .transients
            .iter()
            .any(|time| (*time - 0.1).abs() < 0.03)
    );
    let mut state = 23_u32;
    let noise = AudioBuffer {
        sample_rate: 48000,
        frames: (0..12000)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                [(state as f64 / u32::MAX as f64 - 0.5) as f32; 2]
            })
            .collect(),
    };
    assert!(
        analyze(&noise, &AnalysisOptions::default())
            .unwrap()
            .pitch
            .is_none()
    );
    assert!(analyze_cancellable(&audio, &AnalysisOptions::default(), || true).is_err());
    assert!(
        analyze(
            &audio,
            &AnalysisOptions {
                start_seconds: 0.8,
                ..AnalysisOptions::default()
            }
        )
        .is_err()
    );
    assert!(
        analyze(
            &audio,
            &AnalysisOptions {
                fft_size: 777,
                ..AnalysisOptions::default()
            }
        )
        .is_err()
    );
}
#[test]
fn wav_decoder_preserves_all_supported_pcm_and_float_channels_and_rejects_bad_input() {
    for channels in [1, 2] {
        for bits in [8, 16, 24, 32] {
            let mut bytes = Cursor::new(Vec::new());
            let mut writer = hound::WavWriter::new(
                &mut bytes,
                hound::WavSpec {
                    channels,
                    sample_rate: 44100,
                    bits_per_sample: bits,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .unwrap();
            let scale = 1_i64 << (bits - 1);
            for value in [-(scale / 2) as i32, 0, (scale / 2) as i32, 0] {
                writer.write_sample(value).unwrap();
            }
            writer.finalize().unwrap();
            bytes.set_position(0);
            let audio = read_wav(bytes).unwrap();
            assert_eq!(audio.frames[0][0], -0.5);
            assert_eq!(audio.frames.len(), 4 / channels as usize);
            assert_eq!(audio.frames[0][1], if channels == 1 { -0.5 } else { 0.0 });
        }
    }
    for sample in [0.25_f32, f32::NAN, f32::INFINITY] {
        let mut bytes = Cursor::new(Vec::new());
        let mut writer = hound::WavWriter::new(
            &mut bytes,
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        writer.write_sample(sample).unwrap();
        writer.finalize().unwrap();
        bytes.set_position(0);
        let result = read_wav(bytes);
        assert_eq!(result.is_ok(), sample.is_finite());
    }
    assert!(read_wav(Cursor::new(b"not a wave")).is_err());
}
