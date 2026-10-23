//! Opt-in probe: chip-generated "hard" targets (square, buzz, noise drums) whose
//! exact patch exists, so a perfect search could reach the truth loss.
use discofm_analysis::AnalysisOptions;
use discofm_audio::from_native;
use discofm_chip::{Algorithm, Instrument, Value};
use discofm_engine::{RenderRequest, render_note};
use discofm_fit::*;
use std::sync::Arc;

struct Op {
    mul: u8,
    tl: u8,
    dt: u8,
    ar: u8,
    d1r: u8,
    sl: u8,
    d2r: u8,
}
const fn op(mul: u8, tl: u8, ar: u8, d1r: u8, sl: u8, d2r: u8) -> Op {
    Op {
        mul,
        tl,
        dt: 0,
        ar,
        d1r,
        sl,
        d2r,
    }
}
fn patch(algorithm: u8, feedback: u8, ops: [Op; 4]) -> Instrument {
    let mut p = Instrument::default();
    p.channel.algorithm = Algorithm::new(algorithm).unwrap();
    p.channel.feedback = Value::new(feedback).unwrap();
    for (o, s) in p.operators.iter_mut().zip(ops) {
        o.mul = Value::new(s.mul).unwrap();
        o.tl = Value::new(s.tl).unwrap();
        o.dt = Value::new(s.dt).unwrap();
        o.ar = Value::new(s.ar).unwrap();
        o.d1r = Value::new(s.d1r).unwrap();
        o.sl = Value::new(s.sl).unwrap();
        o.d2r = Value::new(s.d2r).unwrap();
    }
    p
}
fn targets() -> Vec<(&'static str, u8, Instrument)> {
    let mut tom = patch(
        4,
        3,
        [
            op(7, 30, 31, 18, 15, 0),
            op(1, 0, 31, 12, 15, 0),
            op(5, 34, 31, 20, 15, 0),
            op(2, 4, 31, 13, 15, 0),
        ],
    );
    tom.operators[0].dt = Value::new(3).unwrap();
    tom.operators[2].dt = Value::new(6).unwrap();
    vec![
        // Odd harmonics 1/3/5/7 at 1/n amplitude (0.75 dB per TL step).
        (
            "square_additive",
            57,
            patch(
                7,
                0,
                [
                    op(1, 0, 31, 0, 0, 0),
                    op(3, 13, 31, 0, 0, 0),
                    op(5, 19, 31, 0, 0, 0),
                    op(7, 23, 31, 0, 0, 0),
                ],
            ),
        ),
        (
            "buzz_fb7",
            45,
            patch(
                7,
                7,
                [
                    op(1, 0, 31, 0, 0, 0),
                    op(1, 127, 31, 0, 0, 0),
                    op(1, 127, 31, 0, 0, 0),
                    op(1, 127, 31, 0, 0, 0),
                ],
            ),
        ),
        // FB7 + high MUL modulator gives the chaotic "noise" used for snares.
        (
            "snare_noise",
            60,
            patch(
                4,
                7,
                [
                    op(15, 0, 31, 16, 15, 0),
                    op(1, 6, 31, 17, 15, 0),
                    op(1, 20, 31, 22, 15, 0),
                    op(1, 6, 31, 15, 15, 0),
                ],
            ),
        ),
        (
            "kick_thump",
            33,
            patch(
                0,
                5,
                [
                    op(1, 28, 31, 20, 15, 0),
                    op(1, 30, 31, 18, 15, 0),
                    op(1, 26, 31, 22, 15, 0),
                    op(1, 0, 31, 12, 15, 0),
                ],
            ),
        ),
        ("metal_tom", 50, tom),
    ]
}
fn input(target: &Instrument, note: u8, override_pitch: bool, budget: usize) -> FitInput {
    let request = RenderRequest {
        note,
        key_on_seconds: 0.45,
        total_seconds: 0.6,
        ..Default::default()
    };
    let audio = from_native(&render_note(target, &request).unwrap(), 24000).unwrap();
    FitInput {
        reference: Arc::new(audio),
        options: AnalysisOptions {
            trim_silence: false,
            pitch_override_hz: override_pitch.then(|| discofm_engine::midi_hz(note).unwrap()),
            ..Default::default()
        },
        note_on_seconds: 0.0,
        key_off_seconds: request.key_on_seconds,
        initial: Instrument::default(),
        audition: request,
        config: FitConfig {
            compute: ComputeMode::Cpu,
            evaluations: budget,
            ..Default::default()
        },
        locks: Default::default(),
        seeds: Vec::new(),
        refine: false,
    }
}
fn summary(p: &Instrument) -> String {
    let ops: Vec<_> = p
        .operators
        .iter()
        .map(|o| format!("{}/{}/{}", o.mul.get(), o.tl.get(), o.d1r.get()))
        .collect();
    format!(
        "alg{} fb{} ch3={} ssg={:?} ops(mul/tl/d1r)={}",
        p.channel.algorithm.get(),
        p.channel.feedback.get(),
        p.channel3_special.is_some(),
        p.operators.iter().map(|o| o.ssg.raw()).collect::<Vec<_>>(),
        ops.join(" ")
    )
}

/// Scores `patch` under the version 4 weights (bands = inharmonic = 0), which reproduce the
/// previous objective exactly, so runs with different weights stay comparable.
fn v4_score(target: &Instrument, note: u8, patch: &Instrument) -> f64 {
    let mut probe = input(target, note, true, 96);
    probe.initial = patch.clone();
    probe.config.mode = FitMode::Basic;
    probe.config.channel3_policy = ModePolicy::Preserve;
    probe.config.weights = LossWeights {
        bands: 0.0,
        inharmonic: Some(0.0),
        ..LossWeights::default()
    };
    probe.locks = all_parameters(true).into_iter().collect();
    if patch.channel3_special.is_some() {
        probe.audition.channel = discofm_chip::Channel::THREE;
    }
    fit(probe, || false, |_| {}).unwrap().initial_loss.total
}

#[test]
#[ignore = "slow probe; run in release with --nocapture"]
fn hard_targets_probe() {
    let budgets: Vec<usize> = std::env::var("DISCOFM_PROBE_BUDGETS")
        .map(|v| v.split(',').map(|s| s.trim().parse().unwrap()).collect())
        .unwrap_or_else(|_| vec![512, 4096]);
    for (name, note, target) in targets() {
        println!(
            "
== {name} (note {note})  truth {}",
            summary(&target)
        );
        for &budget in &budgets {
            for (label, unpitched, ch3) in [
                ("tonal  ch3=auto ", false, ModePolicy::Auto),
                ("tonal  ch3=keep ", false, ModePolicy::Preserve),
                ("unpitch ch3=keep", true, ModePolicy::Preserve),
            ] {
                let mut run = input(&target, note, !unpitched, budget);
                run.config.unpitched = unpitched;
                run.config.channel3_policy = ch3;
                if unpitched {
                    run.config.weights = LossWeights::unpitched();
                }
                let started = std::time::Instant::now();
                let o = match fit(run, || false, |_| {}) {
                    Ok(o) => o,
                    Err(e) => {
                        println!("   {budget:>5} {label}: ERROR {e}");
                        continue;
                    }
                };
                let best = &o.candidates[0];
                let l = &best.loss;
                println!(
                    "   {budget:>5} {label}: v4 {:.5} ({:.1}s) spec {:.3} conv {:.3} env {:.3} bands {:.3}
                 got {}",
                    v4_score(&target, note, &best.instrument),
                    started.elapsed().as_secs_f64(),
                    l.log_spectral,
                    l.convergence,
                    l.envelope,
                    l.bands,
                    summary(&best.instrument)
                );
            }
        }
    }
}

#[test]
#[ignore = "slow multi-seed spread; run in release with --nocapture"]
fn hard_targets_seed_spread() {
    let budget: usize = std::env::var("DISCOFM_PROBE_BUDGET").map_or(512, |v| v.parse().unwrap());
    let only = std::env::var("DISCOFM_PROBE_ONLY").ok();
    for (name, note, target) in targets()
        .into_iter()
        .skip(2)
        .filter(|t| only.as_deref().is_none_or(|o| o == t.0))
    {
        for (label, weights, unpitched) in [
            (
                "v4 weights",
                LossWeights {
                    bands: 0.0,
                    inharmonic: Some(0.0),
                    ..LossWeights::default()
                },
                false,
            ),
            ("v6 tonal  ", LossWeights::default(), false),
            ("v6 unpitch", LossWeights::unpitched(), true),
        ] {
            let mut independent = Vec::new();
            let scores: Vec<f64> = (1..=6)
                .map(|seed| {
                    let mut run = input(&target, note, !unpitched, budget);
                    run.config.seed = seed;
                    run.config.unpitched = unpitched;
                    run.config.weights = weights.clone();
                    run.config.channel3_policy = ModePolicy::Preserve;
                    let o = fit(run, || false, |_| {}).unwrap();
                    if std::env::var_os("DISCOFM_PROBE_VERBOSE").is_some() {
                        let first = |stage| o.history.iter().find(|h| h.stage == stage).map(|h| h.best);
                        println!(
                            "      seed {seed}: {} | best after coarse {:?} env {:?} mod {:?} refine {:?}",
                            summary(&o.candidates[0].instrument),
                            first(FitStage::Envelopes),
                            first(FitStage::Modulation),
                            first(FitStage::Refine),
                            o.history.last().map(|h| h.best)
                        );
                    }
                    let best = &o.candidates[0].instrument;
                    let reference = from_native(&render_note(&target, &o.context.audition).unwrap(), 24000).unwrap();
                    let audio = from_native(&render_note(best, &o.context.audition).unwrap(), 24000).unwrap();
                    let m = discofm_reference::compare_audio(&audio, &reference, 0.0, true).unwrap();
                    independent.push((m.spectral_cosine_full, m.envelope_relative_error));
                    v4_score(&target, note, best)
                })
                .collect();
            let mut sorted = scores.clone();
            sorted.sort_by(f64::total_cmp);
            let median = |mut v: Vec<f64>| {
                v.sort_by(f64::total_cmp);
                (v[2] + v[3]) / 2.0
            };
            println!(
                "{name:>12} {label}: indep. median cosine {:.3} envelope {:.3}",
                median(independent.iter().map(|m| m.0).collect()),
                median(independent.iter().map(|m| m.1).collect())
            );
            println!(
                "{name:>12} {label}: median {:.4} best {:.4} all {:?}",
                (sorted[2] + sorted[3]) / 2.0,
                sorted[0],
                scores
                    .iter()
                    .map(|v| (v * 1e4).round() / 1e4)
                    .collect::<Vec<_>>()
            );
        }
    }
}

/// Fixed F/CR against jDE self-adaptation on the same targets, budgets and
/// seeds. Same objective in both arms, so final losses compare directly.
#[test]
#[ignore = "slow A/B probe; run in release with --nocapture"]
fn adaptive_de_seed_spread() {
    let budget: usize = std::env::var("DISCOFM_PROBE_BUDGET").map_or(512, |v| v.parse().unwrap());
    let seeds: u64 = std::env::var("DISCOFM_PROBE_SEEDS").map_or(6, |v| v.parse().unwrap());
    let mut wins = [0_usize; 2];
    for (name, note, target) in targets() {
        let mut medians = Vec::new();
        let mut per_seed = Vec::new();
        for adaptive in [false, true] {
            let mut scores: Vec<f64> = (1..=seeds)
                .map(|seed| {
                    let mut run = input(&target, note, true, budget);
                    run.config.seed = seed;
                    run.config.adaptive = adaptive;
                    run.config.channel3_policy = ModePolicy::Preserve;
                    fit(run, || false, |_| {}).unwrap().candidates[0].loss.total
                })
                .collect();
            per_seed.push(scores.clone());
            scores.sort_by(f64::total_cmp);
            medians.push(scores[scores.len() / 2]);
        }
        for (fixed, adaptive) in per_seed[0].iter().zip(&per_seed[1]) {
            wins[usize::from(adaptive < fixed)] += 1;
        }
        println!(
            "{name:>16} @ {budget}: median fixed {:.5} jDE {:.5} ({:+.1}%)",
            medians[0],
            medians[1],
            100.0 * (medians[1] / medians[0] - 1.0)
        );
    }
    println!("seed-paired wins: fixed {} jDE {}", wins[0], wins[1]);
}

/// DE without and with ymfm screening at the same Nuked budget: final loss
/// and wall time. Same objective in both arms.
#[test]
#[ignore = "slow A/B probe; run in release with --nocapture"]
fn screening_seed_spread() {
    let budget: usize = std::env::var("DISCOFM_PROBE_BUDGET").map_or(512, |v| v.parse().unwrap());
    let seeds: u64 = std::env::var("DISCOFM_PROBE_SEEDS").map_or(6, |v| v.parse().unwrap());
    let screening: usize =
        std::env::var("DISCOFM_PROBE_SCREENING").map_or(4, |v| v.parse().unwrap());
    let mut wins = [0_usize; 3];
    let mut seconds = [0.0_f64; 2];
    let mut screened = 0;
    for (name, note, target) in targets() {
        let mut medians = Vec::new();
        let mut per_seed = Vec::new();
        for (arm, screen) in [0, screening].into_iter().enumerate() {
            let mut scores: Vec<f64> = (1..=seeds)
                .map(|seed| {
                    let mut run = input(&target, note, true, budget);
                    run.config.seed = seed;
                    run.config.screening = screen;
                    run.config.channel3_policy = ModePolicy::Preserve;
                    let started = std::time::Instant::now();
                    let outcome = fit(run, || false, |_| {}).unwrap();
                    seconds[arm] += started.elapsed().as_secs_f64();
                    screened += outcome.performance.screened;
                    outcome.candidates[0].loss.total
                })
                .collect();
            per_seed.push(scores.clone());
            scores.sort_by(f64::total_cmp);
            medians.push(scores[scores.len() / 2]);
        }
        for (off, on) in per_seed[0].iter().zip(&per_seed[1]) {
            wins[if on < off {
                1
            } else if off < on {
                0
            } else {
                2
            }] += 1;
        }
        println!(
            "{name:>16} @ {budget}: median off {:.5} screened {:.5} ({:+.1}%)",
            medians[0],
            medians[1],
            100.0 * (medians[1] / medians[0] - 1.0)
        );
    }
    println!(
        "seed-paired: off better {} screened better {} ties {} | wall off {:.1} s screened {:.1} s | {screened} ymfm screens",
        wins[0], wins[1], wins[2], seconds[0], seconds[1]
    );
}
