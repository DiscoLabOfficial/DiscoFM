use discofm_analysis::AnalysisOptions;

use discofm_audio::{AudioBuffer, from_native};

use discofm_chip::{Instrument, Value};

use discofm_engine::{RenderRequest, render_note, render_note_cancellable};

use discofm_fit::*;

use std::{
    cell::Cell,
    sync::Arc,
    time::{Duration, Instant},
};

fn input() -> FitInput {
    let mut target = Instrument::default();

    target.operators[3].tl = Value::new(48).unwrap();

    let request = RenderRequest {
        key_on_seconds: 0.055,

        total_seconds: 0.09,

        ..Default::default()
    };

    let native = render_note(&target, &request).unwrap();

    let audio = from_native(&native, 24000).unwrap();

    let mut initial = target.clone();

    initial.operators[3].tl = Value::new(20).unwrap();

    let unlocked = Parameter::Operator {
        index: 3,

        field: OperatorField::Tl,
    };

    FitInput {
        reference: Arc::new(audio),

        options: AnalysisOptions {
            trim_silence: false,

            pitch_override_hz: Some(440.0),

            ..Default::default()
        },

        note_on_seconds: 0.0,

        key_off_seconds: request.key_on_seconds,

        initial,

        audition: request,

        config: FitConfig {
            match_level: false,

            compute: ComputeMode::Cpu,

            mode: FitMode::Basic,

            evaluations: 96,

            population: 8,

            ..Default::default()
        },

        locks: all_parameters(false)
            .into_iter()
            .filter(|p| *p != unlocked)
            .collect(),

        seeds: Vec::new(),

        refine: false,
    }
}

#[test]

fn recording_gain_is_global_optional_and_legacy_archives_preserve_their_objective() {
    let mut request = input();

    request.initial.operators[3].tl = Value::new(48).unwrap();

    request.locks = all_parameters(false).into_iter().collect();

    let reference = Arc::make_mut(&mut request.reference);

    for frame in &mut reference.frames {
        for sample in frame {
            *sample *= 4.0;
        }
    }

    let fixed = fit(request.clone(), || false, |_| {}).unwrap();

    request.config.match_level = true;

    let matched = fit(request, || false, |_| {}).unwrap();

    assert_eq!(matched.initial, matched.candidates[0].instrument);

    assert!((matched.initial_loss.recording_gain - 4.0).abs() < 0.04);

    assert!(
        matched.initial_loss.total < 0.005,
        "{}",
        matched.initial_loss.total
    );

    assert!(fixed.initial_loss.envelope > 0.5);

    assert_ne!(fixed.context.fingerprint, matched.context.fingerprint);

    let mut previous = fixed.clone();

    previous.context.loss_version = 2;

    let previous: FitOutcome =
        serde_json::from_slice(&serde_json::to_vec(&previous).unwrap()).unwrap();

    previous.validate().unwrap();

    assert_eq!(previous.initial_loss, fixed.initial_loss);

    let mut legacy = serde_json::to_value(&fixed).unwrap();

    legacy["context"]["loss_version"] = 1.into();

    legacy["context"]["audition"]
        .as_object_mut()
        .unwrap()
        .remove("tuning_cents");

    legacy["config"]
        .as_object_mut()
        .unwrap()
        .remove("match_level");

    legacy["config"].as_object_mut().unwrap().remove("compute");

    legacy.as_object_mut().unwrap().remove("performance");

    legacy["initial_loss"]
        .as_object_mut()
        .unwrap()
        .remove("recording_gain");

    for c in legacy["candidates"].as_array_mut().unwrap() {
        c["loss"].as_object_mut().unwrap().remove("recording_gain");
    }

    let legacy: FitOutcome = serde_json::from_value(legacy).unwrap();

    legacy.validate().unwrap();

    assert!(!legacy.config.match_level);

    assert_eq!(legacy.context.audition.tuning_cents, 0.0);

    assert_eq!(legacy.candidates[0].loss.recording_gain, 1.0);
}

#[test]

fn off_grid_pitch_reaches_registers_without_rounding_to_a_semitone() {
    let mut request = input();

    request.audition.note = 36;

    request.audition.tuning_cents = 31.0;

    request.audition.key_on_seconds = 0.16;

    request.audition.total_seconds = 0.24;

    request.key_off_seconds = 0.16;

    request.reference = Arc::new(
        from_native(
            &render_note(&request.initial, &request.audition).unwrap(),
            24000,
        )
        .unwrap(),
    );

    request.options.pitch_override_hz = Some(request.audition.pitch_hz().unwrap());

    request.locks = all_parameters(false).into_iter().collect();

    let fitted = fit(request, || false, |_| {}).unwrap();

    assert_eq!(fitted.context.audition.note, 36);

    assert!((fitted.context.audition.tuning_cents - 31.0).abs() < 1e-9);

    assert!(fitted.initial_loss.total < 0.005);

    let mut forged = fitted;

    forged.context.audition.tuning_cents = 0.0;

    assert!(forged.validate().is_err());
}

#[test]

fn chip_target_improves_levels_deterministically_and_refines_selected_candidates() {
    let input = input();

    let a = fit(input.clone(), || false, |_| {}).unwrap();

    let b = fit(input.clone(), || false, |_| {}).unwrap();

    assert_eq!(a.candidates, b.candidates);

    assert_eq!(a.history, b.history);

    assert_eq!(a.evaluations, b.evaluations);

    assert_eq!(a.renders, b.renders);

    assert!(
        a.candidates[0].loss.total < a.initial_loss.total * 0.05,
        "{} -> {}",
        a.initial_loss.total,
        a.candidates[0].loss.total
    );

    assert!((i32::from(a.candidates[0].instrument.operators[3].tl.get()) - 48).abs() <= 1);

    for c in &a.candidates {
        for p in &input.locks {
            assert_eq!(p.get(&c.instrument), p.get(&input.initial), "{p:?}");
        }
    }

    let mut refine = input;

    refine.initial = a.candidates[0].instrument.clone();

    refine.seeds = a.candidates.iter().map(|c| c.instrument.clone()).collect();

    refine.refine = true;

    let refined = fit(refine, || false, |_| {}).unwrap();

    assert!(refined.refined);

    assert!(refined.candidates[0].loss.total <= a.candidates[0].loss.total + 1e-12);

    a.validate().unwrap();

    let mut invalid = a.clone();

    invalid.candidates[0].instrument.channel.feedback = Value::new(7).unwrap();

    assert!(invalid.validate().is_err());
}

#[test]

fn all_locked_evaluates_once_and_cancellation_keeps_only_completed_candidates() {
    let mut request = input();

    request.locks = all_parameters(false).into_iter().collect();

    let outcome = fit(request.clone(), || false, |_| {}).unwrap();

    assert_eq!(outcome.evaluations, 1);

    assert_eq!(outcome.renders, 1);

    assert_eq!(outcome.candidates[0].instrument, request.initial);

    request.locks.clear();

    request.config.mode = FitMode::Advanced;

    let evaluations = Cell::new(0);

    let cancelled = fit(
        request.clone(),
        || evaluations.get() >= 12,
        |p| evaluations.set(p.evaluations),
    )
    .unwrap();

    assert_eq!(cancelled.status, FitStatus::Cancelled);

    assert_eq!(cancelled.evaluations, 12);

    assert!(!cancelled.candidates.is_empty());

    cancelled.validate().unwrap();

    assert!(matches!(
        fit(request, || true, |_| {}),
        Err(FitError::Cancelled)
    ));
}

#[test]

fn fitting_accepts_absolute_markers_in_long_references() {
    let mut request = input();

    let rate = request.reference.sample_rate;

    let mut frames = vec![[0.0; 2]; rate as usize * 32];

    frames.extend_from_slice(&request.reference.frames);

    request.reference = Arc::new(AudioBuffer {
        frames,

        sample_rate: rate,
    });

    request.options.start_seconds = 32.0;

    request.options.end_seconds = Some(32.0 + request.audition.total_seconds);

    request.note_on_seconds = 32.0;

    request.key_off_seconds += 32.0;

    request.locks = all_parameters(false).into_iter().collect();

    let outcome = fit(request, || false, |_| {}).unwrap();

    assert_eq!(outcome.context.reference_onset_seconds, 32.0);

    assert_eq!(outcome.context.reference_start_seconds, 32.0);

    assert!((outcome.context.audition.total_seconds - 0.09).abs() < 1e-10);

    outcome.validate().unwrap();
}

#[test]

fn temporal_loss_penalizes_wrong_release_and_shared_gain_preserves_level_information() {
    let mut request = input();

    request.locks = all_parameters(false).into_iter().collect();

    request.initial.operators[3].tl = Value::new(48).unwrap();

    let correct = fit(request.clone(), || false, |_| {}).unwrap();

    request.initial.operators[3].rr = Value::new(0).unwrap();

    let wrong = fit(request.clone(), || false, |_| {}).unwrap();

    assert!(wrong.initial_loss.envelope > correct.initial_loss.envelope + 0.05);

    assert!(wrong.initial_loss.tail > correct.initial_loss.tail + 0.05);

    request.initial.operators[3].rr = Value::new(10).unwrap();

    request.initial.operators[3].tl = Value::new(12).unwrap();

    request.options.normalize = true;

    let normalized = fit(request, || false, |_| {}).unwrap();

    assert!(
        normalized.initial_loss.envelope > 1.0,
        "normalization must not erase level differences"
    );
}

#[test]

fn one_shot_decay_cannot_hide_a_missing_tail_behind_global_gain_or_key_off_at_end() {
    let mut request = input();

    request.audition.total_seconds = 0.6;

    request.audition.key_on_seconds = 0.6;

    request.key_off_seconds = 0.6;

    request.config.match_level = true;

    request.initial.operators[3].tl = Value::new(48).unwrap();

    request.initial.operators[3].d1r = Value::new(9).unwrap();

    request.initial.operators[3].sl = Value::new(8).unwrap();

    request.initial.operators[3].d2r = Value::new(1).unwrap();

    request.reference = Arc::new(
        from_native(
            &render_note(&request.initial, &request.audition).unwrap(),
            24000,
        )
        .unwrap(),
    );

    request.locks = all_parameters(false).into_iter().collect();

    let correct = fit(request.clone(), || false, |_| {}).unwrap();

    request.initial.operators[3].d1r = Value::new(31).unwrap();

    request.initial.operators[3].d2r = Value::new(31).unwrap();

    let short = fit(request, || false, |_| {}).unwrap();

    assert!(
        correct.initial_loss.total < 0.01,
        "{}",
        correct.initial_loss.total
    );

    assert!(short.initial_loss.envelope > correct.initial_loss.envelope + 0.2);

    assert!(short.initial_loss.tail > correct.initial_loss.tail + 0.2);
}

#[test]

fn aligned_stereo_other_sample_rates_and_invalid_references_are_explicit() {
    let mut request = input();

    let signal = from_native(
        &render_note(&request.initial, &request.audition).unwrap(),
        44100,
    )
    .unwrap();

    let prefix = 4410;

    let mut frames = vec![[0.0; 2]; prefix];

    frames.extend(signal.frames.iter().map(|f| [f[0], 0.8]));

    request.reference = Arc::new(AudioBuffer {
        sample_rate: 44100,

        frames,
    });

    request.options.channel = discofm_analysis::ChannelMode::Left;

    request.note_on_seconds = 0.1;

    request.key_off_seconds += 0.1;

    request.locks = all_parameters(false).into_iter().collect();

    let outcome = fit(request.clone(), || false, |_| {}).unwrap();

    assert!((outcome.context.reference_onset_seconds - 0.1).abs() < 1e-8);

    assert!(
        outcome.initial_loss.total < 0.025,
        "aligned loss {}",
        outcome.initial_loss.total
    );

    request.reference = Arc::new(AudioBuffer {
        sample_rate: 8000,

        frames: vec![[0.0; 2]; 800],
    });

    request.note_on_seconds = 0.0;

    request.key_off_seconds = 0.05;

    assert!(matches!(
        fit(request.clone(), || false, |_| {}),
        Err(FitError::Silent)
    ));

    request.reference = Arc::new(AudioBuffer {
        sample_rate: 8000,

        frames: vec![[f32::NAN; 2]; 800],
    });

    assert!(fit(request.clone(), || false, |_| {}).is_err());

    request.locks.insert(Parameter::Operator {
        index: 4,

        field: OperatorField::Tl,
    });

    assert!(request.validate().is_err());
}

#[test]

fn cancellable_renderer_matches_offline_and_worker_drop_does_not_need_polling() {
    let mut input = input();

    let expected = render_note(&input.initial, &input.audition).unwrap();

    let bounded = render_note_cancellable(&input.initial, &input.audition, || false).unwrap();

    assert_eq!(expected.frames, bounded.frames);

    let calls = Cell::new(0);

    assert!(matches!(
        render_note_cancellable(&input.initial, &input.audition, || {
            calls.set(calls.get() + 1);

            calls.get() > 8
        }),
        Err(discofm_engine::EngineError::Cancelled)
    ));

    input.locks.clear();

    input.config.evaluations = 20000;

    let worker = FitWorker::start(input).unwrap();

    std::thread::sleep(Duration::from_millis(30));

    let start = Instant::now();

    worker.cancel();

    drop(worker);

    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]

fn complete_advanced_search_improves_envelopes_and_modulation_stages_run() {
    let mut request = input();

    let mut target = request.initial.clone();

    target.operators[3].ar = Value::new(17).unwrap();

    request.audition.total_seconds = 0.2;

    request.audition.key_on_seconds = 0.13;

    request.key_off_seconds = 0.13;

    request.reference =
        Arc::new(from_native(&render_note(&target, &request.audition).unwrap(), 24000).unwrap());

    let unlocked = [
        Parameter::Operator {
            index: 3,

            field: OperatorField::Ar,
        },
        Parameter::Operator {
            index: 3,

            field: OperatorField::Rr,
        },
        Parameter::Operator {
            index: 3,

            field: OperatorField::Ssg,
        },
        Parameter::Operator {
            index: 3,

            field: OperatorField::Am,
        },
        Parameter::LfoEnabled,
        Parameter::LfoRate,
        Parameter::Ams,
        Parameter::Fms,
    ];

    request.locks = all_parameters(false)
        .into_iter()
        .filter(|p| !unlocked.contains(p))
        .collect();

    request.config.mode = FitMode::Advanced;

    request.config.evaluations = 192;

    let mut stages = Vec::new();

    let outcome = fit(
        request,
        || false,
        |p| {
            if !stages.contains(&p.stage) {
                stages.push(p.stage);
            }
        },
    )
    .unwrap();

    assert_eq!(outcome.status, FitStatus::Completed);

    assert!(
        stages.contains(&FitStage::Envelopes)
            && stages.contains(&FitStage::Modulation)
            && stages.contains(&FitStage::Refine)
    );

    assert!(
        outcome.candidates[0].loss.total < outcome.initial_loss.total * 0.6,
        "{} -> {}",
        outcome.initial_loss.total,
        outcome.candidates[0].loss.total
    );
}

#[test]

fn silence_trim_preserves_slow_attack_and_selected_middle_region_uses_real_preroll() {
    let mut request = input();

    request.initial.operators[3].ar = Value::new(12).unwrap();

    request.audition.total_seconds = 0.5;

    request.audition.key_on_seconds = 0.3;

    request.key_off_seconds = 0.3;

    request.reference = Arc::new(
        from_native(
            &render_note(&request.initial, &request.audition).unwrap(),
            24000,
        )
        .unwrap(),
    );

    request.locks = all_parameters(false).into_iter().collect();

    request.options.trim_silence = true;

    let trimmed = fit(request.clone(), || false, |_| {}).unwrap();

    request.options.trim_silence = false;

    let full = fit(request.clone(), || false, |_| {}).unwrap();

    assert_eq!(trimmed.context.reference_onset_seconds, 0.0);

    assert!((trimmed.initial_loss.total - full.initial_loss.total).abs() < 1e-12);

    request.options.start_seconds = 0.05;

    let selected = fit(request, || false, |_| {}).unwrap();

    assert_eq!(selected.context.reference_onset_seconds, 0.0);

    assert_eq!(selected.context.reference_start_seconds, 0.05);

    assert_eq!(selected.context.audition.total_seconds, 0.5);

    assert!(
        selected.initial_loss.total < 0.025,
        "pre-roll loss {}",
        selected.initial_loss.total
    );
}

#[test]

fn parallel_cpu_fit_has_identical_candidates_history_and_cache_counts() {
    let mut request = input();
    request.config.workers = 1;

    let serial = fit(request.clone(), || false, |_| {}).unwrap();

    request.config.workers = 4;

    let parallel = fit(request, || false, |_| {}).unwrap();

    assert_eq!(serial.candidates, parallel.candidates);

    assert_eq!(serial.history, parallel.history);

    assert_eq!(serial.renders, parallel.renders);

    assert_eq!(
        serial.performance.cache_hits,
        parallel.performance.cache_hits
    );
}

#[test]

fn automatic_modulation_crosses_disabled_gates_for_tremolo_vibrato_and_both() {
    for (am, fm) in [(true, false), (false, true), (true, true)] {
        let mut request = input();

        request.config.mode = FitMode::Advanced;
        request.config.evaluations = 128;
        request.config.population = 8;

        request.config.match_level = true;
        request.config.workers = 4;

        request.audition.total_seconds = 0.65;
        request.audition.key_on_seconds = 0.65;
        request.key_off_seconds = 0.65;

        let mut target = request.initial.clone();

        target.global.lfo_enabled = true;
        target.global.lfo_rate = Value::new(4).unwrap();

        target.channel.ams = Value::new(if am { 2 } else { 0 }).unwrap();

        target.channel.fms = Value::new(if fm { 5 } else { 0 }).unwrap();
        target.operators[3].am = am;

        request.reference = Arc::new(
            from_native(&render_note(&target, &request.audition).unwrap(), 24000).unwrap(),
        );

        let controls = [
            Parameter::LfoEnabled,
            Parameter::LfoRate,
            Parameter::Ams,
            Parameter::Fms,
            Parameter::Operator {
                index: 3,
                field: OperatorField::Am,
            },
        ];

        request.locks = all_parameters(false)
            .into_iter()
            .filter(|p| !controls.contains(p))
            .collect();

        let result = fit(request, || false, |_| {}).unwrap();

        let best = &result.candidates[0];

        assert!(best.instrument.global.lfo_enabled, "AM{am} FM{fm}");

        assert_eq!(best.instrument.channel.fms.get() > 0, fm);

        assert_eq!(best.instrument.channel.ams.get() > 0, am);

        assert_eq!(best.instrument.operators[3].am, am);

        assert!(best.loss.total < 0.01, "AM{am} FM{fm}: {:?}", best.loss);

        assert!(best.loss.total < result.initial_loss.total * 0.1);
    }
}

#[test]

fn legacy_v3_sessions_keep_their_original_weighted_scores() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/mandolynn-fitted-session.json");

    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();

    let run: FitOutcome = serde_json::from_value(
        json["fitting"]["runs"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .clone(),
    )
    .unwrap();

    run.validate().unwrap();

    assert_eq!(run.context.loss_version, 3);

    assert_eq!(run.config.channel3_policy, ModePolicy::Preserve);

    assert_eq!(run.config.weights.pitch, 0.0);

    assert_eq!(run.candidates[0].loss.partials, 0.0);
}

#[test]
fn automatic_ch3_discovers_inharmonic_partial_frequencies_and_refinement_can_change_modes() {
    for refine in [false, true] {
        let mut request = input();
        request.initial.channel.algorithm = discofm_chip::Algorithm::new(7).unwrap();
        request.initial.operators[0].tl = Value::new(12).unwrap();
        request.initial.operators[3].tl = Value::new(40).unwrap();
        request.audition.channel = discofm_chip::Channel::THREE;
        request.audition.total_seconds = 0.24;
        request.audition.key_on_seconds = 0.24;
        request.key_off_seconds = 0.24;
        request.config.mode = FitMode::Advanced;
        request.config.evaluations = 128;
        request.config.population = 8;
        request.config.workers = 4;
        request.config.match_level = true;
        request.refine = refine;
        let mut target = request.initial.clone();
        target.channel3_special =
            Some([discofm_engine::frequency_for_hz(623.4, request.audition.clock).unwrap(); 3]);
        request.reference = Arc::new(
            from_native(&render_note(&target, &request.audition).unwrap(), 24000).unwrap(),
        );
        request.locks = all_parameters(true)
            .into_iter()
            .filter(|p| {
                !matches!(
                    p,
                    Parameter::Channel3Mode
                        | Parameter::SpecialFnum { .. }
                        | Parameter::SpecialBlock { .. }
                )
            })
            .collect();
        let result = fit(request, || false, |_| {}).unwrap();
        assert_eq!(
            result.context.audition.channel,
            discofm_chip::Channel::THREE
        );
        assert!(result.candidates[0].instrument.channel3_special.is_some());
        assert!(
            result.candidates[0].loss.total < result.initial_loss.total * 0.2,
            "refine{refine}: {} -> {}",
            result.initial_loss.total,
            result.candidates[0].loss.total
        );
        let restored: FitOutcome =
            serde_json::from_slice(&serde_json::to_vec(&result).unwrap()).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored, result);
    }
}

#[test]
fn modulation_refines_depths_outside_structured_starts_and_resolves_fast_lfo_sidebands() {
    for (rate, seconds) in [(2, 0.65), (7, 0.4)] {
        let mut request = input();
        request.config.mode = FitMode::Advanced;
        request.config.evaluations = 256;
        request.config.population = 8;
        request.config.workers = 4;
        request.config.match_level = true;
        request.audition.total_seconds = seconds;
        request.audition.key_on_seconds = seconds;
        request.key_off_seconds = seconds;
        let mut target = request.initial.clone();
        target.global.lfo_enabled = true;
        target.global.lfo_rate = Value::new(rate).unwrap();
        target.channel.ams = Value::new(1).unwrap();
        target.channel.fms = Value::new(3).unwrap();
        target.operators[3].am = true;
        request.reference = Arc::new(
            from_native(&render_note(&target, &request.audition).unwrap(), 24000).unwrap(),
        );
        let controls = [
            Parameter::LfoEnabled,
            Parameter::LfoRate,
            Parameter::Ams,
            Parameter::Fms,
            Parameter::Operator {
                index: 3,
                field: OperatorField::Am,
            },
        ];
        request.locks = all_parameters(false)
            .into_iter()
            .filter(|p| !controls.contains(p))
            .collect();
        let reference = request.reference.clone();
        let result = fit(request, || false, |_| {}).unwrap();
        let best = &result.candidates[0];
        assert_eq!(best.instrument.global.lfo_rate.get(), rate);
        assert_eq!(best.instrument.channel.ams.get(), 1);
        assert_eq!(best.instrument.channel.fms.get(), 3);
        let audio = from_native(
            &render_note(&best.instrument, &result.context.audition).unwrap(),
            24000,
        )
        .unwrap();
        let measured = discofm_reference::compare_audio(&audio, &reference, 0.0, true).unwrap();
        assert!(
            measured.spectral_cosine_full > 0.99,
            "rate{rate}: {measured:?}"
        );
        assert!(measured.envelope_relative_error < 0.02);
    }
}

#[test]
fn unpitched_mode_fits_feedback_noise_without_detected_pitch_and_archives_the_current_version() {
    let mut target = Instrument::default();
    target.channel.algorithm = discofm_chip::Algorithm::new(4).unwrap();
    target.channel.feedback = Value::new(7).unwrap();
    for (index, (mul, tl)) in [(15, 0), (1, 6), (1, 127), (1, 127)]
        .into_iter()
        .enumerate()
    {
        target.operators[index].mul = Value::new(mul).unwrap();
        target.operators[index].tl = Value::new(tl).unwrap();
        target.operators[index].d1r = Value::new(16).unwrap();
        target.operators[index].sl = Value::new(15).unwrap();
    }
    let mut request = input();
    request.audition.note = 60;
    request.reference =
        Arc::new(from_native(&render_note(&target, &request.audition).unwrap(), 24000).unwrap());
    request.options.pitch_override_hz = None;
    request.initial = target.clone();
    request.initial.operators[1].tl = Value::new(30).unwrap();
    request.locks = all_parameters(false)
        .into_iter()
        .filter(|p| {
            *p != Parameter::Operator {
                index: 1,
                field: OperatorField::Tl,
            }
        })
        .collect();
    assert!(matches!(
        fit(request.clone(), || false, |_| {}),
        Err(FitError::Pitch)
    ));
    request.config.unpitched = true;
    request.config.weights = LossWeights::unpitched();
    let result = fit(request.clone(), || false, |_| {}).unwrap();
    assert_eq!(result.context.loss_version, discofm_fit::LOSS_VERSION);
    // Off-grid energy is relative to an arbitrary playback note here.
    assert_eq!(result.config.weights.inharmonic, Some(0.0));
    // FB7 noise has no clear body, so the keyed note need not be the
    // generating one; with everything else locked the exact target is then out
    // of reach, but the free level is still recovered.
    assert!((20.0..=4000.0).contains(&result.context.pitch_hz));
    let best = &result.candidates[0];
    assert!(
        best.loss.total < result.initial_loss.total * 0.5,
        "{result:?}"
    );
    assert_eq!(best.loss.pitch * request.config.weights.pitch, 0.0);
    assert!((i32::from(best.instrument.operators[1].tl.get()) - 6).abs() <= 2);
    let restored: FitOutcome =
        serde_json::from_slice(&serde_json::to_vec(&result).unwrap()).unwrap();
    restored.validate().unwrap();
    assert!(restored.config.unpitched);
}
