use discofm_audio::{AudioBuffer, from_native};
use discofm_chip::{Algorithm, Channel, Instrument, Value};
use discofm_engine::{ChipProfile, ClockConfig, RenderRequest, render_ymfm_reference};
use discofm_reference::{compare_audio, compare_ymfm};

#[test]
fn independent_ymfm_carriers_match_pitch_gain_and_envelope_across_profiles_clocks_banks() {
    for profile in [ChipProfile::Ym2612Discrete, ChipProfile::Ym3438] {
        for clock in [ClockConfig::Ntsc, ClockConfig::Pal] {
            for channel in [Channel::ONE, Channel::new(4).unwrap()] {
                let request = RenderRequest {
                    profile,
                    clock,
                    channel,
                    key_on_seconds: 0.12,
                    total_seconds: 0.25,
                    tuning_cents: 17.0,
                    ..Default::default()
                };
                let patch = Instrument::default();
                let a = render_ymfm_reference(&patch, &request).unwrap();
                let b = render_ymfm_reference(&patch, &request).unwrap();
                assert_eq!(
                    a.frames, b.frames,
                    "fresh construction must reset independent engine deterministically"
                );
                let comparison = compare_ymfm(&patch, &request).unwrap();
                eprintln!("{profile:?}/{clock:?}/{channel:?}: {comparison:?}");
                assert!(comparison.spectral_cosine_full > 0.98);
                assert!(comparison.envelope_relative_error < 0.1);
                assert!(
                    (comparison.dominant_actual_hz / comparison.dominant_reference_hz)
                        .log2()
                        .abs()
                        * 1200.0
                        < 2.0
                );
                assert!((comparison.rms_actual / comparison.rms_reference - 1.0).abs() < 0.05);
                assert_eq!(comparison.recording_gain, 1.0);
            }
        }
    }
}

#[test]
fn independent_ymfm_algorithms_envelopes_detune_lfo_and_ssg_have_finite_measured_differences() {
    for algorithm in 0..8 {
        let mut patch = Instrument::default();
        patch.channel.algorithm = Algorithm::new(algorithm).unwrap();
        patch.channel.feedback = Value::new(3).unwrap();
        patch.global.lfo_enabled = true;
        patch.global.lfo_rate = Value::new(4).unwrap();
        patch.channel.ams = Value::new(1).unwrap();
        patch.channel.fms = Value::new(2).unwrap();
        for (i, op) in patch.operators.iter_mut().enumerate() {
            op.tl = Value::new(32 + i as u8 * 5).unwrap();
            op.mul = Value::new(1 + i as u8).unwrap();
            op.dt = Value::new(1).unwrap();
            op.ar = Value::new(24).unwrap();
            op.d1r = Value::new(14).unwrap();
            op.sl = Value::new(6).unwrap();
            op.d2r = Value::new(5).unwrap();
            op.rr = Value::new(9).unwrap();
            op.am = true;
            op.ssg = discofm_chip::SsgEg::try_from(8 + algorithm).unwrap();
        }
        let request = RenderRequest {
            profile: ChipProfile::Ym3438,
            key_on_seconds: 0.25,
            total_seconds: 0.35,
            ..Default::default()
        };
        let report = compare_ymfm(&patch, &request).unwrap();
        eprintln!("Algorithm {algorithm}: {report:?}");
        assert!(report.relative_waveform_error.is_finite());
        assert!(
            report.spectral_cosine_full > 0.8,
            "algorithm{algorithm}: {}",
            report.spectral_cosine_full
        );
        assert!(
            report.envelope_relative_error < 0.35,
            "algorithm{algorithm}: {}",
            report.envelope_relative_error
        );
    }
}

#[test]
fn recorded_audio_alignment_gain_and_short_invalid_inputs_are_explicit() {
    let request = RenderRequest {
        key_on_seconds: 0.12,
        total_seconds: 0.2,
        ..Default::default()
    };
    let actual = from_native(
        &render_ymfm_reference(&Instrument::default(), &request).unwrap(),
        24000,
    )
    .unwrap();
    let mut recording = AudioBuffer {
        sample_rate: 24000,
        frames: vec![[actual.frames[0][0] * 4.0, actual.frames[0][1] * 4.0]; 48],
    };
    recording
        .frames
        .extend(actual.frames.iter().map(|f| [f[0] * 4.0, f[1] * 4.0]));
    let fixed = compare_audio(&actual, &recording, 0.005, false).unwrap();
    let matched = compare_audio(&actual, &recording, 0.005, true).unwrap();
    assert_eq!(matched.alignment_samples, -48);
    assert!((matched.recording_gain - 4.0).abs() < 0.01);
    assert!(matched.envelope_relative_error < 0.001);
    assert!(fixed.envelope_relative_error > 0.7);
    let short = AudioBuffer {
        sample_rate: 192000,
        frames: vec![[0.0; 2]],
    };
    assert!(compare_audio(&short, &short, 0.0, false).is_err());
}
