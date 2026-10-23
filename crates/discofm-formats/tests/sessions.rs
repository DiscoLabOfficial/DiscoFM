use discofm_chip::{Algorithm, Channel, Frequency, SsgEg, Value};
use discofm_engine::{ChipProfile, ClockConfig, schedule_note};
use discofm_formats::{MAX_SESSION_BYTES, Session, SessionError, read_session, write_session};
use serde_json::json;

#[test]
fn fitting_history_locks_and_ranked_patches_roundtrip_and_reject_forged_archives() {
    use discofm_fit::*;
    let mut session = Session::default();
    session.audition.key_on_seconds = 0.05;
    session.audition.total_seconds = 0.1;
    let reference = discofm_audio::from_native(
        &discofm_engine::render_note(&session.instrument, &session.audition).unwrap(),
        24000,
    )
    .unwrap();
    let locks = all_parameters(false)
        .into_iter()
        .collect::<ParameterLocks>();
    let run = fit(
        FitInput {
            reference: std::sync::Arc::new(reference),
            options: discofm_analysis::AnalysisOptions {
                pitch_override_hz: Some(440.0),
                ..Default::default()
            },
            note_on_seconds: 0.0,
            key_off_seconds: 0.05,
            initial: session.instrument.clone(),
            audition: session.audition.clone(),
            config: FitConfig::default(),
            locks: locks.clone(),
            seeds: Vec::new(),
            refine: false,
        },
        || false,
        |_| {},
    )
    .unwrap();
    session.fitting = Some(FittingState {
        config: FitConfig::default(),
        locks,
        runs: vec![run],
    });
    let mut bytes = Vec::new();
    write_session(&mut bytes, &session).unwrap();
    assert_eq!(read_session(&bytes[..]).unwrap(), session);
    let original: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for (pointer, bad) in [
        ("/fitting/config/evaluations", json!(1)),
        (
            "/fitting/locks",
            json!([{ "Operator": { "index": 4,"field": "Tl" } }]),
        ),
        ("/fitting/runs/0/candidates/0/loss/total", json!(-1)),
        ("/fitting/runs/0/context/frames", json!(999999999)),
        ("/fitting/runs/0/history/0/best", json!(999)),
    ] {
        let mut value = original.clone();
        *value.pointer_mut(pointer).unwrap() = bad;
        assert!(
            read_session(&serde_json::to_vec(&value).unwrap()[..]).is_err(),
            "{pointer}"
        );
    }
    session.fitting.as_mut().unwrap().runs[0].candidates[0]
        .loss
        .total = f64::NAN;
    let mut output = Vec::new();
    assert!(write_session(&mut output, &session).is_err());
    assert!(output.is_empty());
}

#[test]
fn optional_reference_preparation_roundtrips_and_rejects_invalid_markers_and_processing() {
    let mut session = Session {
        reference_analysis: Some(discofm_formats::ReferenceSettings {
            options: discofm_analysis::AnalysisOptions {
                channel: discofm_analysis::ChannelMode::Right,
                fft_size: 4096,
                start_seconds: 0.1,
                end_seconds: Some(0.8),
                pitch_override_hz: Some(330.0),
                ..Default::default()
            },
            note_on_seconds: 0.1,
            key_off_seconds: 0.6,
        }),
        ..Session::default()
    };
    let mut bytes = Vec::new();
    write_session(&mut bytes, &session).unwrap();
    assert_eq!(read_session(&bytes[..]).unwrap(), session);
    session.reference_analysis.as_mut().unwrap().key_off_seconds = 0.05;
    assert!(write_session(Vec::new(), &session).is_err());
    session.reference_analysis.as_mut().unwrap().key_off_seconds = 0.6;
    session
        .reference_analysis
        .as_mut()
        .unwrap()
        .options
        .fft_size = 3;
    assert!(session.validate().is_err());
}

fn rich_session() -> Session {
    let mut session = Session {
        reference_path: Some("C:\\Timbres\\órgão \"FM\"\n参考.wav".into()),
        ..Session::default()
    };
    session.audition.channel = Channel::THREE;
    session.audition.note = 0;
    session.audition.velocity = Value::new(63).unwrap();
    session.audition.key_on_seconds = 0.04;
    session.audition.total_seconds = 0.1;
    session.instrument.channel3_special = Some([
        Frequency::new(0, 0).unwrap(),
        Frequency::new(987, 3).unwrap(),
        Frequency::new(2047, 7).unwrap(),
    ]);
    session.instrument.global.lfo_rate = Value::new(7).unwrap(); // Preserve dormant rate.
    session.instrument.channel.feedback = Value::new(7).unwrap();
    session.instrument.channel.ams = Value::new(3).unwrap();
    session.instrument.channel.fms = Value::new(7).unwrap();
    session.instrument.channel.left = false;
    session.instrument.channel.right = false;
    for (i, op) in session.instrument.operators.iter_mut().enumerate() {
        let i = i as u8;
        op.dt = Value::new(4 + i).unwrap();
        op.mul = Value::new(i * 5).unwrap();
        op.tl = Value::new(i * 42).unwrap();
        op.rs = Value::new(i).unwrap();
        op.ar = Value::new(i * 10).unwrap();
        op.d1r = Value::new(31 - i).unwrap();
        op.d2r = Value::new(i * 9).unwrap();
        op.sl = Value::new(i * 5).unwrap();
        op.rr = Value::new(15 - i).unwrap();
        op.am = !i.is_multiple_of(2);
        session.instrument.audition.velocity_tl_depth[i as usize] = Value::new(i * 40).unwrap();
    }
    session
}

#[test]
fn nondefault_sessions_preserve_every_field_all_shapes_algorithms_profiles_and_clocks() {
    for profile in [ChipProfile::Ym2612Discrete, ChipProfile::Ym3438] {
        for clock in [ClockConfig::Ntsc, ClockConfig::Pal] {
            for algorithm in Algorithm::ALL {
                for shape in SsgEg::ALL {
                    let mut session = rich_session();
                    session.audition.profile = profile;
                    session.audition.clock = clock;
                    session.audition.tuning_cents = -31.25;
                    session.instrument.channel.algorithm = algorithm;
                    for op in &mut session.instrument.operators {
                        op.ssg = shape;
                    }
                    let mut bytes = Vec::new();
                    write_session(&mut bytes, &session).unwrap();
                    assert_eq!(read_session(&bytes[..]).unwrap(), session);
                    assert_eq!(serde_json::from_slice::<Session>(&bytes).unwrap(), session);
                    assert!(schedule_note(&session.instrument, &session.audition).is_ok());
                }
            }
        }
    }
}

#[test]
fn original_v1_fixture_remains_compatible_including_optional_fields() {
    let original = include_bytes!("../../../examples/basic-session.json");
    let session = read_session(&original[..]).unwrap();
    assert_eq!(session, Session::default());
    let mut value: serde_json::Value = serde_json::from_slice(original).unwrap();
    value.as_object_mut().unwrap().remove("reference_path");
    value["instrument"]
        .as_object_mut()
        .unwrap()
        .remove("channel3_special");
    assert_eq!(
        read_session(&serde_json::to_vec(&value).unwrap()[..]).unwrap(),
        session
    );
    let mut encoded = Vec::new();
    write_session(&mut encoded, &session).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&encoded).unwrap(),
        serde_json::from_slice::<serde_json::Value>(original).unwrap()
    );
}

#[test]
fn nested_errors_name_the_invalid_field_and_reject_wrong_shapes() {
    let cases = [
        (
            "/instrument/operators/2/ar",
            json!(32),
            "instrument.operators[2].ar",
        ),
        (
            "/instrument/operators/0/ssg",
            json!(7),
            "instrument.operators[0].ssg",
        ),
        (
            "/instrument/channel/algorithm",
            json!(8),
            "instrument.channel.algorithm",
        ),
        (
            "/instrument/global/lfo_rate",
            json!(-1),
            "instrument.global.lfo_rate",
        ),
        (
            "/instrument/audition/velocity_tl_depth/3",
            json!(128),
            "instrument.audition.velocity_tl_depth[3]",
        ),
        ("/audition/channel", json!(0), "audition.channel"),
        ("/audition/velocity", json!(128), "audition.velocity"),
        ("/instrument/operators", json!([]), "instrument.operators"),
        (
            "/instrument/channel3_special",
            json!([[0, 0], [1, 0]]),
            "instrument.channel3_special",
        ),
        (
            "/instrument/channel3_special/1",
            json!([2048, 0]),
            "instrument.channel3_special[1]",
        ),
    ];
    for (pointer, bad, path) in cases {
        let mut value = serde_json::to_value(rich_session()).unwrap();
        *value.pointer_mut(pointer).unwrap() = bad;
        let bytes = serde_json::to_vec(&value).unwrap();
        let error = read_session(&bytes[..]).unwrap_err();
        assert!(error.to_string().contains(path), "{pointer}: {error}");
        assert!(serde_json::from_slice::<Session>(&bytes).is_err());
    }
}

#[test]
fn semantic_validation_is_consistent_for_reads_direct_serde_writes_and_engine() {
    for channel in 1..=6 {
        let mut session = rich_session();
        session.audition.channel = Channel::new(channel).unwrap();
        let bytes = serde_json::to_vec(&session).unwrap();
        let mut output = b"untouched".to_vec();
        assert_eq!(read_session(&bytes[..]).is_ok(), channel == 3);
        assert_eq!(
            serde_json::from_slice::<Session>(&bytes).is_ok(),
            channel == 3
        );
        assert_eq!(write_session(&mut output, &session).is_ok(), channel == 3);
        assert_eq!(
            schedule_note(&session.instrument, &session.audition).is_ok(),
            channel == 3
        );
        if channel != 3 {
            assert_eq!(output, b"untouched");
        }
    }
    for (pointer, bad) in [
        ("/version", json!(2)),
        ("/audition/note", json!(127)),
        ("/audition/total_seconds", json!(0)),
        ("/audition/total_seconds", json!(31)),
        ("/audition/key_on_seconds", json!(0)),
        ("/audition/key_on_seconds", json!(2)),
        ("/audition/profile", json!("RAW_DIAGNOSTIC")),
    ] {
        let mut value = serde_json::to_value(Session::default()).unwrap();
        *value.pointer_mut(pointer).unwrap() = bad;
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(read_session(&bytes[..]).is_err(), "{pointer}");
        assert!(
            serde_json::from_slice::<Session>(&bytes).is_err(),
            "{pointer}"
        );
    }
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut session = Session::default();
        session.audition.total_seconds = invalid;
        let mut output = Vec::new();
        assert!(write_session(&mut output, &session).is_err());
        assert!(output.is_empty());
    }
}

#[test]
fn malformed_unknown_duplicate_trailing_and_oversized_sessions_are_rejected() {
    let original = serde_json::to_string(&Session::default()).unwrap();
    for suffix in ["{}", " false", " garbage"] {
        assert!(read_session(format!("{original}{suffix}").as_bytes()).is_err());
    }
    for malformed in ["{", "{\"version\":1,\"version\":1}"] {
        assert!(read_session(malformed.as_bytes()).is_err());
    }
    for pointer in [
        "",
        "/instrument",
        "/instrument/operators/0",
        "/instrument/channel",
        "/instrument/global",
        "/instrument/audition",
        "/audition",
    ] {
        let mut value = serde_json::to_value(Session::default()).unwrap();
        value
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("typo".into(), json!(0));
        assert!(
            read_session(&serde_json::to_vec(&value).unwrap()[..]).is_err(),
            "{pointer}"
        );
    }
    assert!(matches!(
        read_session(&vec![b' '; MAX_SESSION_BYTES + 1][..]),
        Err(SessionError::TooLarge)
    ));
    let oversized = Session {
        reference_path: Some("\"".repeat(MAX_SESSION_BYTES / 2)),
        ..Session::default()
    };
    let mut output = b"preserve".to_vec();
    assert!(matches!(
        write_session(&mut output, &oversized),
        Err(SessionError::TooLarge)
    ));
    assert_eq!(output, b"preserve");
}

/// Eight retained runs at their largest (16 candidates with special
/// frequencies, 256 history points, maximal diagnostic strings) must always
/// be writable, and must read back unchanged.
#[test]
fn eight_maximal_fitting_runs_fit_the_session_limit() {
    use discofm_fit::*;
    let mut session = Session::default();
    session.audition.key_on_seconds = 0.05;
    session.audition.total_seconds = 0.08;
    let reference = discofm_audio::from_native(
        &discofm_engine::render_note(&session.instrument, &session.audition).expect("render"),
        24000,
    )
    .expect("resample");
    let config = FitConfig {
        evaluations: 2304,
        candidates: MAX_CANDIDATES,
        compute: ComputeMode::Cpu,
        ..Default::default()
    };
    let mut run = fit(
        FitInput {
            reference: std::sync::Arc::new(reference),
            options: discofm_analysis::AnalysisOptions {
                pitch_override_hz: Some(440.0),
                trim_silence: false,
                ..Default::default()
            },
            note_on_seconds: 0.0,
            key_off_seconds: 0.05,
            initial: session.instrument.clone(),
            audition: session.audition.clone(),
            config: config.clone(),
            locks: Default::default(),
            seeds: Vec::new(),
            refine: false,
        },
        || false,
        |_| {},
    )
    .expect("fit");
    // Thinning keeps a long curve below MAX_HISTORY; pad to the worst case.
    assert!(run.history.len() <= MAX_HISTORY);
    let first = run.history[0].clone();
    let last = run.history.last().expect("history").clone();
    run.history = (0..MAX_HISTORY)
        .map(|k| match k {
            0 => first.clone(),
            k if k + 1 == MAX_HISTORY => last.clone(),
            k => HistoryPoint {
                evaluation: 1 + k * (run.evaluations - 1) / (MAX_HISTORY - 1),
                stage: last.stage,
                best: last.best,
                current: first.current,
            },
        })
        .collect();
    assert_eq!(run.candidates.len(), MAX_CANDIDATES);
    assert_eq!(run.context.audition.channel, Channel::THREE);
    for (index, candidate) in run.candidates.iter_mut().enumerate() {
        let fnum = 600 + index as u16 * 37;
        candidate.instrument.channel3_special = Some([
            Frequency::new(fnum, 7).expect("fnum"),
            Frequency::new(fnum + 1, 6).expect("fnum"),
            Frequency::new(fnum + 2, 5).expect("fnum"),
        ]);
    }
    run.performance.backend = format!("GPU \"{}\"", "b".repeat(240));
    run.performance.adapter = Some(format!("NVIDIA \"{}\"", "a".repeat(1000)));
    run.performance.note = format!("CPU chosen \"{}\"", "n".repeat(4070));
    run.validate().expect("maximal run stays valid");
    session.audition.channel = Channel::THREE;
    session.reference_path = Some(format!("C:/{}/reference.wav", "folder/".repeat(30)));
    let mut state = FittingState {
        config,
        locks: all_parameters(true).into_iter().collect(),
        runs: Vec::new(),
    };
    for _ in 0..MAX_RUNS + 1 {
        state.push(run.clone());
    }
    assert_eq!(state.runs.len(), MAX_RUNS);
    session.fitting = Some(state);
    let mut bytes = Vec::new();
    write_session(&mut bytes, &session).expect("eight runs must be writable");
    assert_eq!(read_session(&bytes[..]).expect("read back"), session);
    let pretty = serde_json::to_vec_pretty(&session).expect("pretty").len();
    let compact = serde_json::to_vec(&session).expect("compact").len();
    println!(
        "pretty {} KiB, compact {} KiB",
        pretty / 1024,
        compact / 1024
    );
    // The compact fallback is the guarantee; leave room for long paths and
    // future optional fields.
    assert!(compact <= MAX_SESSION_BYTES * 3 / 4, "{compact} bytes");
    // When indentation alone would break the limit, the writer falls back to
    // compact JSON, which reads back identically.
    let padding = MAX_SESSION_BYTES - pretty + 4096;
    session.reference_path = Some(format!("C:/{}/reference.wav", "x".repeat(padding)));
    let mut bytes = Vec::new();
    write_session(&mut bytes, &session).expect("compact fallback");
    assert!(bytes.len() <= MAX_SESSION_BYTES && !bytes.contains(&b'\n'));
    assert_eq!(read_session(&bytes[..]).expect("read compact"), session);
}

#[test]
fn every_example_session_loads_and_rewrites_identically() {
    let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut count = 0;
    for entry in std::fs::read_dir(&examples).expect("examples folder") {
        let path = entry.expect("entry").path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let file = std::fs::File::open(&path).expect("open example");
        let session = read_session(file).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut bytes = Vec::new();
        write_session(&mut bytes, &session).expect("rewrite");
        assert_eq!(
            read_session(&bytes[..]).expect("reread"),
            session,
            "{}",
            path.display()
        );
        count += 1;
    }
    assert!(count >= 7, "found {count} example sessions");
}
