use discofm_audio::from_native;
use discofm_chip::{Algorithm, Channel, Frequency, Instrument, SsgEg, Value};
use discofm_engine::{
    ChipProfile, ClockConfig, NukedChip, PortEvent, RenderRequest, TICKS_PER_FRAME,
    direct_upstream_pins, render_note, render_trace, schedule_note,
};
use discofm_reference::{compare_note, compare_pins};

fn request(profile: ChipProfile) -> RenderRequest {
    RenderRequest {
        profile,
        key_on_seconds: 0.06,
        total_seconds: 0.12,
        ..RenderRequest::default()
    }
}
fn ac_rms(frames: &[[f32; 2]], channel: usize) -> f64 {
    let mean = frames.iter().map(|p| f64::from(p[channel])).sum::<f64>() / frames.len() as f64;
    (frames
        .iter()
        .map(|p| (f64::from(p[channel]) - mean).powi(2))
        .sum::<f64>()
        / frames.len() as f64)
        .sqrt()
}
fn measured_hz(frames: &[[f32; 2]], sample_rate: f64) -> f64 {
    let mean = frames.iter().map(|p| p[0]).sum::<f32>() / frames.len() as f32;
    let crossings = frames
        .windows(2)
        .enumerate()
        .filter_map(|(i, p)| {
            if p[0][0] <= mean && p[1][0] > mean {
                Some(i as f64 + f64::from((mean - p[0][0]) / (p[1][0] - p[0][0])))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert!(crossings.len() > 3, "expected an audible periodic signal");
    (crossings.len() - 1) as f64 * sample_rate / (crossings.last().unwrap() - crossings[0])
}

#[test]
fn direct_upstream_agreement_across_algorithms_channels_profiles_and_ssg() {
    for profile in [ChipProfile::Ym2612Discrete, ChipProfile::Ym3438] {
        for channel in 1..=6 {
            for algorithm in 0..=7 {
                let mut patch = Instrument::default();
                patch.channel.algorithm = Algorithm::new(algorithm).unwrap();
                patch.channel.feedback = Value::new(algorithm).unwrap();
                patch.channel.ams = Value::new(3).unwrap();
                patch.channel.fms = Value::new(5).unwrap();
                patch.global.lfo_enabled = true;
                patch.global.lfo_rate = Value::new(7).unwrap();
                for (i, op) in patch.operators.iter_mut().enumerate() {
                    op.mul = Value::new(i as u8 + 1).unwrap();
                    op.tl = Value::new(24 + i as u8 * 7).unwrap();
                    op.dt = Value::new((algorithm + i as u8) % 8).unwrap();
                    op.rs = Value::new(i as u8).unwrap();
                    op.d1r = Value::new(8).unwrap();
                    op.d2r = Value::new(4).unwrap();
                    op.sl = Value::new(5).unwrap();
                    op.am = true;
                    op.ssg = SsgEg::try_from(8 + algorithm).unwrap();
                }
                let request = RenderRequest {
                    channel: Channel::new(channel).unwrap(),
                    key_on_seconds: 0.006,
                    total_seconds: 0.012,
                    ..request(profile)
                };
                let report = compare_note(&patch, &request).unwrap();
                assert_eq!(
                    report.mismatched_ticks, 0,
                    "{profile:?} channel {channel} algorithm {algorithm}"
                );
                assert_eq!(report.rms_error, 0.0);
            }
        }
    }
}

#[test]
fn deterministic_reset_and_concurrent_profile_isolation() {
    let expected = [ChipProfile::Ym2612Discrete, ChipProfile::Ym3438].map(|profile| {
        render_note(&Instrument::default(), &request(profile))
            .unwrap()
            .frames
    });
    assert_ne!(expected[0], expected[1]);
    std::thread::scope(|scope| {
        for index in 0..4 {
            let expected = &expected;
            scope.spawn(move || {
                let profile_index = index % 2;
                let profile = [ChipProfile::Ym2612Discrete, ChipProfile::Ym3438][profile_index];
                for _ in 0..3 {
                    assert_eq!(
                        render_note(&Instrument::default(), &request(profile))
                            .unwrap()
                            .frames,
                        expected[profile_index]
                    );
                }
            });
        }
    });
}

#[test]
fn measured_pitch_pal_ntsc_mul_zero_and_output_rates() {
    for clock in [ClockConfig::Ntsc, ClockConfig::Pal] {
        let request = RenderRequest {
            clock,
            key_on_seconds: 0.18,
            total_seconds: 0.2,
            ..request(ChipProfile::Ym3438)
        };
        let native = render_note(&Instrument::default(), &request).unwrap();
        let start = (0.02 * native.sample_rate) as usize;
        let end = (0.15 * native.sample_rate) as usize;
        let pitch = measured_hz(&native.frames[start..end], native.sample_rate);
        assert!((pitch - 440.0).abs() < 0.5, "measured {pitch} Hz");
        for rate in [44100, 48000, 96000] {
            let converted = from_native(&native, rate).unwrap();
            assert!(
                (converted.frames.len() as f64 / f64::from(rate) - 0.2).abs()
                    < 2.0 / f64::from(rate)
            );
            let pitch = measured_hz(
                &converted.frames[(rate / 50) as usize..(rate * 15 / 100) as usize],
                f64::from(rate),
            );
            assert!((pitch - 440.0).abs() < 0.5);
        }
        let mut half = Instrument::default();
        half.operators[3].mul = Value::new(0).unwrap();
        let audio = render_note(&half, &request).unwrap();
        assert!((measured_hz(&audio.frames[start..end], audio.sample_rate) - 220.0).abs() < 0.5);
    }
}

#[test]
fn key_off_releases_and_stereo_routing_is_hardware_driven() {
    let mut patch = Instrument::default();
    patch.operators[3].rr = Value::new(15).unwrap();
    patch.channel.right = false;
    for profile in [ChipProfile::Ym2612Discrete, ChipProfile::Ym3438] {
        let audio = render_note(&patch, &request(profile)).unwrap();
        let rate = audio.sample_rate;
        let active = &audio.frames[(0.01 * rate) as usize..(0.05 * rate) as usize];
        let released = &audio.frames[(0.10 * rate) as usize..];
        assert!(ac_rms(active, 0) > 0.001);
        assert!(ac_rms(released, 0) < ac_rms(active, 0) * 0.01);
        if profile == ChipProfile::Ym3438 {
            assert!(audio.frames.iter().all(|p| p[1] == 0.0));
        } else {
            assert!(ac_rms(active, 1) < ac_rms(active, 0) * 0.2);
        }
    }
}

#[test]
fn quiet_notes_zero_crossings_and_muted_residual_are_not_a_bitcrusher() {
    let mut patch = Instrument::default();
    patch.operators[3].tl = Value::new(48).unwrap();
    let mut hashes = Vec::new();
    for profile in [ChipProfile::Ym2612Discrete, ChipProfile::Ym3438] {
        let request = request(profile);
        let schedule = schedule_note(&patch, &request).unwrap();
        let pins = direct_upstream_pins(
            profile,
            &schedule.events,
            schedule.capture_start,
            schedule.frames * 24,
        )
        .unwrap();
        let report = compare_note(&patch, &request).unwrap();
        assert_eq!(report.mismatched_ticks, 0);
        assert!(pins.iter().any(|p| p[0] > 0));
        assert!(pins.iter().any(|p| p[0] < 0));
        if profile == ChipProfile::Ym2612Discrete {
            assert!(pins.iter().flatten().all(|v| v % 3 == 0));
        }
        hashes.push(report.pin_hash_fnv1a);
    }
    assert_ne!(hashes[0], hashes[1]);
    // At deeper attenuation a positive half-wave may quantize to zero while
    // negative values truncate to -1. Preserve this deterministic asymmetry.
    patch.operators[3].tl = Value::new(72).unwrap();
    let sub_lsb_request = request(ChipProfile::Ym3438);
    let schedule = schedule_note(&patch, &sub_lsb_request).unwrap();
    let sub_lsb = direct_upstream_pins(
        ChipProfile::Ym3438,
        &schedule.events,
        schedule.capture_start,
        schedule.frames * 24,
    )
    .unwrap();
    assert!(sub_lsb.iter().all(|p| p[0] <= 0));
    assert!(sub_lsb.iter().any(|p| p[0] == -1));
    // Upstream deliberately retains +3 pin baseline after reset in discrete mode.
    let clean = direct_upstream_pins(ChipProfile::Ym3438, &[], 96, 96).unwrap();
    let discrete = direct_upstream_pins(ChipProfile::Ym2612Discrete, &[], 96, 96).unwrap();
    assert!(clean.iter().all(|p| *p == [0, 0]));
    assert!(discrete.iter().all(|p| *p == [3, 3]));
}

#[test]
fn special_channel_three_uses_distinct_frequency_latches() {
    let mut patch = Instrument {
        channel3_special: Some([
            Frequency::new(700, 3).unwrap(),
            Frequency::new(1100, 4).unwrap(),
            Frequency::new(1500, 2).unwrap(),
        ]),
        ..Instrument::default()
    };
    for op in &mut patch.operators {
        op.tl = Value::new(32).unwrap();
    }
    let request = RenderRequest {
        channel: Channel::THREE,
        ..request(ChipProfile::Ym3438)
    };
    let report = compare_note(&patch, &request).unwrap();
    assert_eq!(report.max_absolute_error, 0);
    let special = render_note(&patch, &request).unwrap();
    patch.channel3_special = None;
    let ordinary = render_note(&patch, &request).unwrap();
    assert_ne!(special.frames, ordinary.frames);
}

#[test]
fn channel_six_pcm_interface_and_fm_pin_quantization_are_distinct() {
    // DAC test deliberately uses hardware 8-bit PCM data interface; note renderer
    // never uses this route to reconstruct instruments.
    let events = [
        PortEvent {
            tick: 96,
            port: 0,
            data: 0x2b,
        },
        PortEvent {
            tick: 98,
            port: 1,
            data: 0x80,
        },
        PortEvent {
            tick: 144,
            port: 0,
            data: 0x2a,
        },
        PortEvent {
            tick: 146,
            port: 1,
            data: 0,
        },
        PortEvent {
            tick: 384,
            port: 0,
            data: 0x2a,
        },
        PortEvent {
            tick: 386,
            port: 1,
            data: 255,
        },
    ];
    for profile in [ChipProfile::Ym2612Discrete, ChipProfile::Ym3438] {
        let expected = direct_upstream_pins(profile, &events, 192, 480).unwrap();
        let mut chip = NukedChip::new(profile).unwrap();
        let actual = render_trace(&mut chip, &events, 192, 480).unwrap();
        assert_eq!(
            compare_pins(&actual, &expected).unwrap().mismatched_ticks,
            0
        );
        assert!(actual[..192].iter().any(|p| p[0] < -100));
        assert!(actual[240..].iter().any(|p| p[0] > 100));
    }
}

#[test]
fn pin_capture_integration_handles_partial_native_batch_boundaries() {
    let request = request(ChipProfile::Ym2612Discrete);
    let schedule = schedule_note(&Instrument::default(), &request).unwrap();
    let mut chip = NukedChip::new(request.profile).unwrap();
    for ticks in [1, 23, 24, 25, 4095, 4096, 4097] {
        let events = schedule
            .events
            .iter()
            .copied()
            .filter(|e| e.tick < schedule.capture_start + ticks as u64)
            .collect::<Vec<_>>();
        let actual = render_trace(&mut chip, &events, schedule.capture_start, ticks).unwrap();
        let expected =
            direct_upstream_pins(request.profile, &events, schedule.capture_start, ticks).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(
            discofm_engine::integrate_pins(&actual).len(),
            ticks / TICKS_PER_FRAME
        );
    }
}

#[test]
fn pinned_backend_default_note_regression_hashes() {
    // Captured by the real `cargo run -- validate` invocation on 2026-10-10.
    // These pin-level snapshots deliberately exclude platform audio/resampling.
    for (profile, hash) in [
        (ChipProfile::Ym2612Discrete, "e5354d82d454959d"),
        (ChipProfile::Ym3438, "8dc66ae06d69b2dd"),
    ] {
        let request = RenderRequest {
            profile,
            key_on_seconds: 0.05,
            total_seconds: 0.1,
            ..RenderRequest::default()
        };
        let report = compare_note(&Instrument::default(), &request).unwrap();
        assert_eq!(report.pin_hash_fnv1a, hash);
        assert_eq!(report.ticks, 127848);
    }
}
