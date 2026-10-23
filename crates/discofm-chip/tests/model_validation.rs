use discofm_chip::{
    Algorithm, AuditionSettings, Channel, Frequency, Instrument, OperatorId, RegisterWrite, SsgEg,
    Value, encode_instrument, frequency_writes, key_write,
};

fn bounded<const MAX: u8>() {
    for raw in 0..=255 {
        let expected = raw <= MAX;
        assert_eq!(Value::<MAX>::new(raw).is_ok(), expected);
        let decoded = serde_json::from_str::<Value<MAX>>(&raw.to_string());
        assert_eq!(decoded.is_ok(), expected);
        if let Ok(value) = decoded {
            assert_eq!(value.get(), raw);
            assert_eq!(serde_json::to_string(&value).unwrap(), raw.to_string());
        }
    }
    for invalid in ["-1", "256", "1.5", "null", "true", "\"1\""] {
        assert!(serde_json::from_str::<Value<MAX>>(invalid).is_err());
    }
}

#[test]
fn every_scalar_and_ssg_encoding_is_checked_at_construction_and_json_boundaries() {
    bounded::<3>();
    bounded::<7>();
    bounded::<15>();
    bounded::<31>();
    bounded::<127>();
    for raw in 0..=255 {
        assert_eq!(Channel::new(raw).is_ok(), (1..=6).contains(&raw));
        assert_eq!(
            serde_json::from_str::<Channel>(&raw.to_string()).is_ok(),
            (1..=6).contains(&raw)
        );
        assert_eq!(Algorithm::new(raw).is_ok(), raw <= 7);
        assert_eq!(
            serde_json::from_str::<Algorithm>(&raw.to_string()).is_ok(),
            raw <= 7
        );
        let valid_ssg = raw == 0 || (8..=15).contains(&raw);
        assert_eq!(SsgEg::try_from(raw).is_ok(), valid_ssg);
        assert_eq!(
            serde_json::from_str::<SsgEg>(&raw.to_string()).is_ok(),
            valid_ssg
        );
    }
    assert!(Channel::new(0).unwrap_err().to_string().contains("1..6"));
    assert!(!SsgEg::OFF.is_enabled());
    assert_eq!(SsgEg::OFF.shape(), None);
    for ssg in SsgEg::ALL.into_iter().skip(1) {
        assert!(ssg.is_enabled());
        assert_eq!(SsgEg::from_shape(ssg.shape().unwrap()), ssg);
        assert_eq!(
            SsgEg::from_flags(ssg.inverted(), ssg.alternate(), ssg.hold()),
            ssg
        );
        assert_eq!(serde_json::to_string(&ssg).unwrap(), ssg.raw().to_string());
    }
}

#[test]
fn all_frequency_pairs_and_channel_codes_reconstruct_without_reserved_bits() {
    for block in 0..=7 {
        for fnum in 0..=2047 {
            let f = Frequency::new(fnum, block).unwrap();
            assert_eq!(((u16::from(f.high()) & 7) << 8) | u16::from(f.low()), fnum);
            assert_eq!(f.high() >> 3, block);
            assert_eq!(f.high() & 0xc0, 0);
            assert_eq!(
                serde_json::from_str::<Frequency>(&format!("[{fnum},{block}]")).unwrap(),
                f
            );
        }
    }
    for invalid in [
        "[2048,0]", "[0,8]", "[-1,0]", "[0,-1]", "[0]", "[0,0,0]", "[1.5,0]",
    ] {
        assert!(
            serde_json::from_str::<Frequency>(invalid).is_err(),
            "{invalid}"
        );
    }
    for (index, code) in [0, 1, 2, 4, 5, 6].into_iter().enumerate() {
        let ch = Channel::new(index as u8 + 1).unwrap();
        for mask in 0..=15 {
            assert_eq!(
                key_write(ch, Value::new(mask).unwrap()),
                RegisterWrite::new(0, 0x28, mask << 4 | code)
            );
        }
        let writes = frequency_writes(ch, Frequency::new(2047, 7).unwrap());
        let bank = (index / 3) as u8;
        let offset = (index % 3) as u8;
        assert_eq!(
            writes,
            [
                RegisterWrite::new(bank, 0xa4 + offset, 0x3f),
                RegisterWrite::new(bank, 0xa0 + offset, 0xff)
            ]
        );
    }
}

#[test]
fn every_operator_field_reaches_the_correct_slot_on_all_six_channels() {
    // Each tuple fixes a register independently of the encoder's slot helper.
    for number in 1..=6 {
        let ch = Channel::new(number).unwrap();
        let bank = (number - 1) / 3;
        let channel_offset = (number - 1) % 3;
        for (slot, offset) in [0, 8, 4, 12].into_iter().enumerate() {
            for raw in 0..=255 {
                let mut patch = Instrument::default();
                let op = &mut patch.operators[slot];
                op.dt = Value::new((raw / 16) % 8).unwrap();
                op.mul = Value::new(raw % 16).unwrap();
                op.tl = Value::new(raw % 128).unwrap();
                op.rs = Value::new((raw / 32) % 4).unwrap();
                op.ar = Value::new(raw % 32).unwrap();
                op.d1r = Value::new(raw % 32).unwrap();
                op.d2r = Value::new(raw % 32).unwrap();
                op.sl = Value::new(raw / 16).unwrap();
                op.rr = Value::new(raw % 16).unwrap();
                op.am = (raw / 32) % 2 != 0;
                op.ssg = SsgEg::ALL[raw as usize % 9];
                let expected = [
                    (0x30, (((raw / 16) % 8) << 4) | (raw % 16)),
                    (0x40, raw % 128),
                    (0x50, (((raw / 32) % 4) << 6) | (raw % 32)),
                    (0x60, (((raw / 32) % 2) << 7) | (raw % 32)),
                    (0x70, raw % 32),
                    (0x80, ((raw / 16) << 4) | (raw % 16)),
                    (0x90, SsgEg::ALL[raw as usize % 9].raw()),
                ];
                let writes = encode_instrument(&patch, ch, Value::new(127).unwrap()).unwrap();
                assert_eq!(writes.len(), 33);
                for (base, data) in expected {
                    assert!(
                        writes.contains(&RegisterWrite::new(
                            bank,
                            base + offset + channel_offset,
                            data
                        )),
                        "channel {number}, slot {slot}, raw {raw}, base {base:x}"
                    );
                }
            }
        }
    }
}

#[test]
fn channel_bitfields_cover_algorithms_feedback_lfo_sensitivities_and_stereo() {
    for number in 1..=6 {
        let ch = Channel::new(number).unwrap();
        for algorithm in 0..=7 {
            for feedback in 0..=7 {
                for ams in 0..=3 {
                    for fms in 0..=7 {
                        let mut patch = Instrument::default();
                        patch.channel.algorithm = Algorithm::new(algorithm).unwrap();
                        patch.channel.feedback = Value::new(feedback).unwrap();
                        patch.channel.ams = Value::new(ams).unwrap();
                        patch.channel.fms = Value::new(fms).unwrap();
                        for stereo in 0..=3 {
                            patch.channel.left = stereo & 2 != 0;
                            patch.channel.right = stereo & 1 != 0;
                            patch.global.lfo_enabled = feedback & 1 != 0;
                            patch.global.lfo_rate = Value::new(fms).unwrap();
                            let writes =
                                encode_instrument(&patch, ch, Value::new(127).unwrap()).unwrap();
                            let bank = (number - 1) / 3;
                            let offset = (number - 1) % 3;
                            assert!(writes.contains(&RegisterWrite::new(
                                bank,
                                0xb0 + offset,
                                feedback << 3 | algorithm
                            )));
                            assert!(writes.contains(&RegisterWrite::new(
                                bank,
                                0xb4 + offset,
                                stereo << 6 | ams << 4 | fms
                            )));
                            assert_eq!(
                                writes[0],
                                RegisterWrite::new(0, 0x22, (feedback & 1) << 3 | fms)
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn algorithm_graphs_use_datasheet_operator_order_and_s1_feedback() {
    use OperatorId::{S1, S2, S3, S4};
    let edges: [&[(OperatorId, OperatorId)]; 8] = [
        &[(S1, S2), (S2, S3), (S3, S4)],
        &[(S1, S3), (S2, S3), (S3, S4)],
        &[(S1, S4), (S2, S3), (S3, S4)],
        &[(S1, S2), (S2, S4), (S3, S4)],
        &[(S1, S2), (S3, S4)],
        &[(S1, S2), (S1, S3), (S1, S4)],
        &[(S1, S2)],
        &[],
    ];
    for (index, algorithm) in Algorithm::ALL.into_iter().enumerate() {
        let mut actual = algorithm.routing().connections.to_vec();
        actual.sort_by_key(|(a, b)| (a.index(), b.index()));
        let mut expected = edges[index].to_vec();
        expected.sort_by_key(|(a, b)| (a.index(), b.index()));
        assert_eq!(actual, expected);
        assert_eq!(algorithm.display_number(), index as u8 + 1);
        assert_eq!(algorithm.feedback_operator(), S1);
        let mask = [8, 8, 8, 8, 10, 14, 14, 15][index];
        for op in OperatorId::ALL {
            assert_eq!(algorithm.is_carrier(op), mask & (1 << op.index()) != 0);
        }
    }
}

#[test]
fn special_frequencies_and_software_velocity_preserve_hardware_parameters() {
    let frequencies = [
        Frequency::new(0, 0).unwrap(),
        Frequency::new(987, 3).unwrap(),
        Frequency::new(2047, 7).unwrap(),
    ];
    let patch = Instrument {
        channel3_special: Some(frequencies),
        ..Instrument::default()
    };
    assert_eq!(patch.required_channel(), Some(Channel::THREE));
    let pitch = Frequency::new(1083, 4).unwrap();
    assert_eq!(
        patch.operator_frequencies(pitch),
        [frequencies[0], frequencies[1], frequencies[2], pitch]
    );
    for number in 1..=6 {
        let result = encode_instrument(
            &patch,
            Channel::new(number).unwrap(),
            Value::new(127).unwrap(),
        );
        assert_eq!(result.is_ok(), number == 3);
        if let Ok(writes) = result {
            assert_eq!(
                &writes[33..],
                &[
                    RegisterWrite::new(0, 0xad, 0),
                    RegisterWrite::new(0, 0xa9, 0),
                    RegisterWrite::new(0, 0xae, 0x1b),
                    RegisterWrite::new(0, 0xaa, 0xdb),
                    RegisterWrite::new(0, 0xac, 0x3f),
                    RegisterWrite::new(0, 0xa8, 0xff)
                ]
            );
        }
    }
    let settings = AuditionSettings {
        velocity_tl_depth: [Value::new(127).unwrap(); 4],
    };
    for raw in 0..=127 {
        assert_eq!(
            settings
                .total_level(
                    OperatorId::S4,
                    Value::new(0).unwrap(),
                    Value::new(raw).unwrap()
                )
                .get(),
            127 - raw
        );
        assert_eq!(
            settings
                .total_level(
                    OperatorId::S4,
                    Value::new(127).unwrap(),
                    Value::new(raw).unwrap()
                )
                .get(),
            127
        );
    }
    let mut op = patch.operators[0];
    op.dt = Value::new(4).unwrap();
    op.mul = Value::new(0).unwrap();
    assert_eq!(op.signed_detune(), 0);
    assert_eq!(op.dt.get(), 4);
    assert_eq!(op.multiplier_ratio(), (1, 2));
}
