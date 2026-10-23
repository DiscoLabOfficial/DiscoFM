use discofm_chip::{Algorithm, Channel, Frequency, Instrument, SsgEg, Value};
use discofm_engine::{
    ChannelSnapshot, ChipBackend, ChipProfile, ClockConfig, EnvelopeStage, NukedChip,
    RenderRequest, frequency_hz, inspect_instrument, render_note, schedule_note,
};

fn request() -> RenderRequest {
    RenderRequest {
        profile: ChipProfile::Ym3438,
        key_on_seconds: 0.2,
        total_seconds: 0.3,
        ..RenderRequest::default()
    }
}

/// Advance the real chip bus, sampling after complete native frames. Diagnostics
/// must not change state or audio; assert idempotence on every sampled frame.
fn probe(
    patch: &Instrument,
    request: &RenderRequest,
    stride: u64,
    count: usize,
) -> Vec<ChannelSnapshot> {
    let schedule = schedule_note(patch, request).unwrap();
    assert!(stride * count as u64 <= schedule.frames as u64);
    let mut chip = NukedChip::new(request.profile).unwrap();
    let mut scratch = [[0; 2]; 4096];
    let mut tick = 0;
    let mut event = 0;
    let mut result = Vec::with_capacity(count);
    for i in 1..=count {
        let target = schedule.capture_start + i as u64 * stride * 24;
        while tick < target {
            if let Some(write) = schedule.events.get(event)
                && write.tick == tick
            {
                chip.write_port(write.port as u8, write.data).unwrap();
                event += 1;
            }
            let boundary = schedule
                .events
                .get(event)
                .map_or(target, |write| write.tick.min(target));
            let amount = (boundary - tick).min(scratch.len() as u64) as usize;
            chip.clock_into(&mut scratch[..amount]).unwrap();
            tick += amount as u64;
        }
        let state = chip.snapshot(request.channel).unwrap();
        assert_eq!(chip.snapshot(request.channel).unwrap(), state);
        result.push(state);
    }
    result
}
fn rms(frames: &[[f32; 2]]) -> f64 {
    let mean = frames.iter().map(|v| f64::from(v[0])).sum::<f64>() / frames.len() as f64;
    (frames
        .iter()
        .map(|v| (f64::from(v[0]) - mean).powi(2))
        .sum::<f64>()
        / frames.len() as f64)
        .sqrt()
}
fn pitch(frames: &[[f32; 2]], rate: f64) -> f64 {
    let mean = frames.iter().map(|v| v[0]).sum::<f32>() / frames.len() as f32;
    let crossings = frames
        .windows(2)
        .enumerate()
        .filter(|(_, v)| v[0][0] <= mean && v[1][0] > mean)
        .map(|(i, v)| i as f64 + f64::from((mean - v[0][0]) / (v[1][0] - v[0][0])))
        .collect::<Vec<_>>();
    assert!(crossings.len() > 5);
    (crossings.len() - 1) as f64 * rate / (crossings.last().unwrap() - crossings[0])
}

#[test]
fn all_register_fields_are_loaded_into_actual_native_slots() {
    for profile in [ChipProfile::Ym2612Discrete, ChipProfile::Ym3438] {
        for number in 1..=6 {
            let mut patch = Instrument::default();
            patch.channel.algorithm = Algorithm::new(number).unwrap();
            patch.channel.feedback = Value::new(7).unwrap();
            patch.channel.ams = Value::new(3).unwrap();
            patch.channel.fms = Value::new(5).unwrap();
            patch.channel.left = false;
            patch.global.lfo_enabled = true;
            patch.global.lfo_rate = Value::new(6).unwrap();
            for (i, op) in patch.operators.iter_mut().enumerate() {
                let i = i as u8;
                op.dt = Value::new(4 + i).unwrap();
                op.mul = Value::new(i * 5).unwrap();
                op.tl = Value::new(10 + i * 20).unwrap();
                op.rs = Value::new(i).unwrap();
                op.ar = Value::new(i * 10).unwrap();
                op.d1r = Value::new(31 - i).unwrap();
                op.d2r = Value::new(i * 8).unwrap();
                op.sl = Value::new(i * 5).unwrap();
                op.rr = Value::new(15 - i).unwrap();
                op.am = !i.is_multiple_of(2);
                op.ssg = SsgEg::try_from(8 + i).unwrap();
                patch.audition.velocity_tl_depth[i as usize] = Value::new(i * 30).unwrap();
            }
            if number == 3 {
                patch.channel3_special = Some([
                    Frequency::new(700, 3).unwrap(),
                    Frequency::new(1100, 4).unwrap(),
                    Frequency::new(1500, 2).unwrap(),
                ]);
            }
            let request = RenderRequest {
                profile,
                channel: Channel::new(number).unwrap(),
                velocity: Value::new(63).unwrap(),
                ..request()
            };
            let state = inspect_instrument(&patch, &request).unwrap();
            assert_eq!(state.channel, patch.channel);
            assert_eq!(state.global, patch.global);
            assert!(!state.dac_enabled);
            assert_eq!(state.channel3_mode, u8::from(number == 3));
            for (i, snapshot) in state.operators.iter().enumerate() {
                let mut expected = patch.operators[i];
                let extra = (64 * u16::from(patch.audition.velocity_tl_depth[i].get()) + 63) / 127;
                expected.tl =
                    Value::new((u16::from(expected.tl.get()) + extra).min(127) as u8).unwrap();
                assert_eq!(
                    snapshot.registers, expected,
                    "channel {number} operator {i}"
                );
            }
            if let Some(frequencies) = patch.channel3_special {
                assert_eq!(state.special_frequencies, frequencies);
            }
        }
    }
}

#[test]
fn all_algorithms_have_the_declared_audible_carriers() {
    let request = RenderRequest {
        key_on_seconds: 0.025,
        total_seconds: 0.03,
        ..request()
    };
    for (algorithm, mask) in [8, 8, 8, 8, 10, 14, 14, 15].into_iter().enumerate() {
        for slot in 0..4 {
            let mut patch = Instrument::default();
            patch.channel.algorithm = Algorithm::new(algorithm as u8).unwrap();
            for op in &mut patch.operators {
                op.tl = Value::new(127).unwrap();
            }
            patch.operators[slot].tl = Value::new(24).unwrap();
            let audio = render_note(&patch, &request).unwrap();
            let level = rms(&audio.frames[300..1200]);
            if mask & (1 << slot) != 0 {
                assert!(level > 0.001, "algorithm {algorithm}, slot {slot}: {level}");
            } else {
                assert!(
                    level < 0.00001,
                    "algorithm {algorithm}, slot {slot}: {level}"
                );
            }
        }
    }
}

#[test]
fn algorithm_five_fans_out_and_six_only_modulates_s2() {
    let request = RenderRequest {
        key_on_seconds: 0.03,
        total_seconds: 0.04,
        ..request()
    };
    for algorithm in [5, 6] {
        for carrier in 1..4 {
            let mut patch = Instrument::default();
            patch.channel.algorithm = Algorithm::new(algorithm).unwrap();
            for op in &mut patch.operators {
                op.tl = Value::new(127).unwrap();
            }
            patch.operators[carrier].tl = Value::new(24).unwrap();
            let unmodulated = render_note(&patch, &request).unwrap();
            patch.operators[0].tl = Value::new(16).unwrap();
            let modulated = render_note(&patch, &request).unwrap();
            assert_eq!(
                unmodulated.frames != modulated.frames,
                algorithm == 5 || carrier == 1
            );
        }
    }
    let mut patch = Instrument::default();
    patch.operators[0].tl = Value::new(24).unwrap();
    let no_feedback = render_note(&patch, &request).unwrap();
    patch.channel.feedback = Value::new(7).unwrap();
    assert_ne!(
        render_note(&patch, &request).unwrap().frames,
        no_feedback.frames
    );
}

#[test]
fn algorithms_zero_through_three_use_datasheet_s2_s3_causal_paths() {
    let request = RenderRequest {
        key_on_seconds: 0.025,
        total_seconds: 0.03,
        ..request()
    };
    for active in 0..=3 {
        let mut patch = Instrument::default();
        patch.channel.algorithm = Algorithm::new(0).unwrap();
        for slot in [1, 2] {
            if active & (1 << (slot - 1)) != 0 {
                patch.operators[slot].tl = Value::new(16).unwrap();
            }
        }
        let baseline = render_note(&patch, &request).unwrap();
        patch.operators[0].tl = Value::new(16).unwrap();
        assert_eq!(
            render_note(&patch, &request).unwrap().frames != baseline.frames,
            active == 3,
            "algorithm 0: both intermediate operators are necessary"
        );
    }
    // With S1 silent, algorithms 1/2 have S2 -> S3 -> S4. Only S3 can
    // directly modulate S4. Register order and pipeline order differ here.
    for algorithm in [1, 2] {
        let mut patch = Instrument::default();
        patch.channel.algorithm = Algorithm::new(algorithm).unwrap();
        let baseline = render_note(&patch, &request).unwrap();
        patch.operators[1].tl = Value::new(16).unwrap();
        assert_eq!(
            render_note(&patch, &request).unwrap().frames,
            baseline.frames,
            "algorithm {algorithm}: S2 requires S3"
        );
        patch.operators[1].tl = Value::new(127).unwrap();
        patch.operators[2].tl = Value::new(16).unwrap();
        assert_ne!(
            render_note(&patch, &request).unwrap().frames,
            baseline.frames,
            "algorithm {algorithm}: S3 directly modulates S4"
        );
    }
    for intermediate in [1, 2] {
        let mut patch = Instrument::default();
        patch.channel.algorithm = Algorithm::new(3).unwrap();
        patch.operators[intermediate].tl = Value::new(16).unwrap();
        let baseline = render_note(&patch, &request).unwrap();
        patch.operators[0].tl = Value::new(16).unwrap();
        assert_eq!(
            render_note(&patch, &request).unwrap().frames != baseline.frames,
            intermediate == 1,
            "algorithm 3: S1 reaches S4 through S2"
        );
    }
}

#[test]
fn diagnostic_snapshots_preserve_exact_pin_streams() {
    for profile in [ChipProfile::Ym2612Discrete, ChipProfile::Ym3438] {
        let request = RenderRequest {
            profile,
            key_on_seconds: 0.003,
            total_seconds: 0.006,
            ..request()
        };
        let schedule = schedule_note(&Instrument::default(), &request).unwrap();
        let expected = discofm_engine::direct_upstream_pins(
            profile,
            &schedule.events,
            schedule.capture_start,
            schedule.frames * 24,
        )
        .unwrap();
        let mut chip = NukedChip::new(profile).unwrap();
        let mut event = 0;
        let mut actual = Vec::new();
        for number in 1..=6 {
            chip.snapshot(Channel::new(number).unwrap()).unwrap();
        }
        for tick in 0..schedule.capture_start + expected.len() as u64 {
            if let Some(write) = schedule.events.get(event)
                && write.tick == tick
            {
                chip.write_port(write.port as u8, write.data).unwrap();
                event += 1;
            }
            if tick % 17 == 0 {
                chip.snapshot(request.channel).unwrap();
            }
            let mut pins = [[0; 2]];
            chip.clock_into(&mut pins).unwrap();
            if tick >= schedule.capture_start {
                actual.push(pins[0]);
            }
        }
        assert_eq!(actual, expected);
    }
}

#[test]
fn ssg_initial_direction_uses_the_inversion_bit() {
    let mut patch = Instrument::default();
    patch.operators[3].tl = Value::new(0).unwrap();
    patch.operators[3].d1r = Value::new(16).unwrap();
    patch.operators[3].d2r = Value::new(16).unwrap();
    patch.operators[3].sl = Value::new(1).unwrap();
    for shape in 8..=15 {
        patch.operators[3].ssg = SsgEg::try_from(shape).unwrap();
        let states = probe(&patch, &request(), 1, 64);
        let segment = &states[8..32];
        let first = segment.first().unwrap().operators[3].output_attenuation;
        let last = segment.last().unwrap().operators[3].output_attenuation;
        if shape & 4 == 0 {
            assert!(
                first < 100 && last > first,
                "falling amplitude shape {shape:x}: {first}->{last}"
            );
        } else {
            assert!(
                first > 400 && last < first,
                "rising amplitude shape {shape:x}: {first}->{last}"
            );
        }
    }
}

#[test]
fn detune_signs_multiplier_zero_and_special_operator_pitches_reach_phase_generator() {
    let request = request();
    let mut patch = Instrument::default();
    let base = probe(&patch, &request, 64, 2)[1].operators[3].phase_increment;
    for (dt, sign) in [
        (0, 0),
        (4, 0),
        (1, 1),
        (2, 1),
        (3, 1),
        (5, -1),
        (6, -1),
        (7, -1),
    ] {
        patch.operators[3].dt = Value::new(dt).unwrap();
        let increment = probe(&patch, &request, 64, 2)[1].operators[3].phase_increment;
        assert_eq!((i64::from(increment) - i64::from(base)).signum(), sign);
    }
    patch.operators[3].dt = Value::new(0).unwrap();
    for mul in 0..=15 {
        patch.operators[3].mul = Value::new(mul).unwrap();
        let increment = probe(&patch, &request, 64, 2)[1].operators[3].phase_increment;
        assert_eq!(
            increment,
            if mul == 0 {
                base >> 1
            } else {
                base * u32::from(mul)
            }
        );
    }
    patch = Instrument::default();
    patch.channel3_special = Some([
        Frequency::new(700, 3).unwrap(),
        Frequency::new(1100, 4).unwrap(),
        Frequency::new(1500, 2).unwrap(),
    ]);
    let request = RenderRequest {
        channel: Channel::THREE,
        ..request
    };
    let schedule = schedule_note(&patch, &request).unwrap();
    let frequencies = patch.operator_frequencies(schedule.frequency);
    let state = &probe(&patch, &request, 64, 2)[1];
    for (slot, frequency) in frequencies.into_iter().enumerate() {
        assert_eq!(
            state.operators[slot].phase_increment,
            (u32::from(frequency.fnum()) << frequency.block()) >> 1
        );
        for op in &mut patch.operators {
            op.tl = Value::new(127).unwrap();
        }
        patch.operators[slot].tl = Value::new(24).unwrap();
        let audio = render_note(&patch, &request).unwrap();
        let measured = pitch(&audio.frames[1000..9000], audio.sample_rate);
        assert!(
            (measured - frequency_hz(frequency, request.clock)).abs() < 0.5,
            "slot {slot}: {measured}"
        );
    }
}

#[test]
fn native_envelope_attack_decay_sustain_release_and_key_scaling_are_observable() {
    let request = request();
    let mut patch = Instrument::default();
    patch.operators[3].ar = Value::new(0).unwrap();
    assert!(
        probe(&patch, &request, 128, 32)
            .iter()
            .all(|s| s.operators[3].envelope_attenuation == 1023)
    );
    patch.operators[3].ar = Value::new(31).unwrap();
    patch.operators[3].sl = Value::new(4).unwrap();
    patch.operators[3].d2r = Value::new(31).unwrap();
    assert!(
        probe(&patch, &request, 128, 32)
            .iter()
            .all(|s| s.operators[3].envelope_attenuation == 0)
    ); // D1R=0 never reaches sustain.
    patch.operators[3].d1r = Value::new(31).unwrap();
    patch.operators[3].d2r = Value::new(0).unwrap();
    let held = probe(&patch, &request, 128, 32);
    assert!(held.last().unwrap().operators[3].stage == EnvelopeStage::Sustain);
    assert_eq!(
        held[16].operators[3].envelope_attenuation,
        held[31].operators[3].envelope_attenuation
    );
    assert!(held[31].operators[3].envelope_attenuation >= 128);
    patch.operators[3].d2r = Value::new(20).unwrap();
    assert!(
        probe(&patch, &request, 128, 32)[31].operators[3].envelope_attenuation
            > held[31].operators[3].envelope_attenuation
    );
    patch = Instrument::default();
    let released = RenderRequest {
        key_on_seconds: 0.02,
        total_seconds: 0.2,
        ..request.clone()
    };
    patch.operators[3].rr = Value::new(0).unwrap();
    let slow = probe(&patch, &released, 128, 64);
    patch.operators[3].rr = Value::new(15).unwrap();
    let fast = probe(&patch, &released, 128, 64);
    assert!(
        fast[63].operators[3].envelope_attenuation > slow[63].operators[3].envelope_attenuation
    );
    assert_eq!(fast[63].operators[3].stage, EnvelopeStage::Release);
    patch = Instrument::default();
    patch.operators[3].ar = Value::new(12).unwrap();
    patch.operators[3].rs = Value::new(3).unwrap();
    let low = probe(
        &patch,
        &RenderRequest {
            note: 36,
            ..request.clone()
        },
        64,
        64,
    );
    let high = probe(
        &patch,
        &RenderRequest {
            note: 96,
            ..request
        },
        64,
        64,
    );
    let threshold = |states: &[ChannelSnapshot]| {
        states
            .iter()
            .position(|s| s.operators[3].envelope_attenuation < 512)
    };
    let low_time = threshold(&low).expect("low note must cross the attack threshold");
    let high_time = threshold(&high).expect("high note must cross the attack threshold");
    assert!(high_time < low_time, "high={high_time}, low={low_time}");
}

#[test]
fn lfo_requires_sensitivity_and_operator_am_enable() {
    let mut patch = Instrument::default();
    let request = request();
    let disabled = render_note(&patch, &request).unwrap();
    patch.global.lfo_enabled = true;
    patch.global.lfo_rate = Value::new(7).unwrap();
    assert_eq!(
        render_note(&patch, &request).unwrap().frames,
        disabled.frames
    );
    patch.channel.ams = Value::new(3).unwrap();
    assert_eq!(
        render_note(&patch, &request).unwrap().frames,
        disabled.frames
    ); // Operator AM is off.
    patch.operators[3].am = true;
    let states = probe(&patch, &request, 32, 256);
    let min = states
        .iter()
        .map(|s| s.operators[3].output_attenuation)
        .min()
        .unwrap();
    let max = states
        .iter()
        .map(|s| s.operators[3].output_attenuation)
        .max()
        .unwrap();
    assert!(max - min > 100);
    patch.operators[3].am = false;
    patch.channel.fms = Value::new(7).unwrap();
    let states = probe(&patch, &request, 32, 256);
    assert!(
        states
            .windows(2)
            .any(|w| w[0].operators[3].phase_increment != w[1].operators[3].phase_increment)
    );
    patch.global.lfo_enabled = false;
    let states = probe(&patch, &request, 32, 256);
    assert!(
        states
            .windows(2)
            .all(|w| w[0].operators[3].phase_increment == w[1].operators[3].phase_increment)
    );
    for rate in 0..=7 {
        patch.global.lfo_rate = Value::new(rate).unwrap();
        assert_eq!(
            inspect_instrument(&patch, &request).unwrap().global,
            patch.global
        );
    }
    // Count AM triangle direction changes, avoiding a claim about nominal LFO Hz.
    patch.operators[3].am = true;
    patch.channel.fms = Value::new(0).unwrap();
    patch.global.lfo_enabled = true;
    let turns = |states: &[ChannelSnapshot]| {
        let signs = states
            .windows(2)
            .map(|w| {
                (i32::from(w[1].operators[3].output_attenuation)
                    - i32::from(w[0].operators[3].output_attenuation))
                .signum()
            })
            .filter(|sign| *sign != 0)
            .collect::<Vec<_>>();
        signs.windows(2).filter(|w| w[0] != w[1]).count()
    };
    for clock in [ClockConfig::Ntsc, ClockConfig::Pal] {
        let request = RenderRequest {
            clock,
            ..request.clone()
        };
        patch.global.lfo_rate = Value::new(0).unwrap();
        let slow = turns(&probe(&patch, &request, 32, 256));
        patch.global.lfo_rate = Value::new(7).unwrap();
        let fast = turns(&probe(&patch, &request, 32, 256));
        assert!(fast > slow * 4, "{clock:?}: fast={fast}, slow={slow}");
    }
}

#[test]
fn all_ssg_shapes_loop_or_hold_and_release_after_key_off() {
    let mut patch = Instrument::default();
    patch.operators[3].d1r = Value::new(31).unwrap();
    patch.operators[3].d2r = Value::new(31).unwrap();
    patch.operators[3].sl = Value::new(1).unwrap();
    let request = RenderRequest {
        key_on_seconds: 0.08,
        total_seconds: 0.18,
        ..request()
    };
    for raw in 8..=15 {
        patch.operators[3].ssg = SsgEg::try_from(raw).unwrap();
        let states = probe(&patch, &request, 16, 256);
        let tail = &states[128..];
        let changes = tail
            .windows(2)
            .filter(|w| {
                w[0].operators[3].output_attenuation != w[1].operators[3].output_attenuation
            })
            .count();
        if raw & 1 != 0 {
            assert_eq!(changes, 0, "held shape {raw:x}");
        } else {
            assert!(changes > 10, "looping shape {raw:x}: {changes}");
        }
        if raw & 1 != 0 {
            let quiet = matches!(raw, 9 | 15);
            assert_eq!(
                tail.last().unwrap().operators[3].output_attenuation > 900,
                quiet,
                "shape {raw:x}"
            );
        }
        let released = probe(&patch, &request, 128, 70);
        assert_eq!(released[69].operators[3].stage, EnvelopeStage::Release);
        assert!(
            released[69].operators[3].envelope_attenuation > 900,
            "release shape {raw:x}"
        );
    }
}
