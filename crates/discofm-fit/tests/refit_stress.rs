//! Opt-in stress probe: repeated large refits in one process, as the GUI does
//! (refine the selected candidate, seeded with the whole previous run).
use discofm_analysis::AnalysisOptions;
use discofm_audio::from_native;
use discofm_chip::{Algorithm, Instrument, Value};
use discofm_engine::{RenderRequest, render_note};
use discofm_fit::*;
use std::sync::Arc;

fn env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[test]
#[ignore = "slow stress probe; run in release with --nocapture"]
fn repeated_large_refits_stay_valid() {
    let mut target = Instrument::default();
    target.channel.algorithm = Algorithm::new(4).expect("algorithm");
    target.channel.feedback = Value::new(5).expect("feedback");
    for (o, (mul, tl)) in target
        .operators
        .iter_mut()
        .zip([(2, 28), (1, 4), (3, 34), (1, 8)])
    {
        o.mul = Value::new(mul).expect("mul");
        o.tl = Value::new(tl).expect("tl");
        o.d1r = Value::new(6).expect("d1r");
        o.sl = Value::new(4).expect("sl");
    }
    let seconds = std::env::var("DISCOFM_STRESS_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.8);
    let request = RenderRequest {
        note: 57,
        key_on_seconds: seconds * 0.8,
        total_seconds: seconds,
        ..Default::default()
    };
    let reference = Arc::new(
        from_native(&render_note(&target, &request).expect("render"), 44100).expect("resample"),
    );
    let compute = match std::env::var("DISCOFM_STRESS_COMPUTE").as_deref() {
        Ok("cpu") => ComputeMode::Cpu,
        Ok("gpu") => ComputeMode::Gpu,
        _ => ComputeMode::Auto,
    };
    let mut state = FittingState {
        config: FitConfig {
            evaluations: env("DISCOFM_STRESS_EVALUATIONS", 6000),
            population: env("DISCOFM_STRESS_POPULATION", 64),
            candidates: 16,
            compute,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut initial = Instrument::default();
    let mut seeds = Vec::new();
    for round in 0..env("DISCOFM_STRESS_ROUNDS", 5) {
        let input = FitInput {
            reference: Arc::clone(&reference),
            options: AnalysisOptions {
                trim_silence: false,
                pitch_override_hz: Some(discofm_engine::midi_hz(57).expect("pitch")),
                ..Default::default()
            },
            note_on_seconds: 0.0,
            key_off_seconds: request.key_on_seconds,
            initial: initial.clone(),
            audition: request.clone(),
            config: state.config.clone(),
            locks: Default::default(),
            seeds: seeds.clone(),
            refine: round > 0,
        };
        let started = std::time::Instant::now();
        let worker = FitWorker::start(input).expect("valid input");
        let outcome = loop {
            if let Some(result) = worker.poll().result {
                break result.expect("fit");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        println!(
            "round {round}: {:.1} s, {} evaluations, {} renders, best {:.6}, {}",
            started.elapsed().as_secs_f64(),
            outcome.evaluations,
            outcome.renders,
            outcome.candidates[0].loss.total,
            outcome.performance.backend
        );
        initial = outcome.candidates[0].instrument.clone();
        seeds = outcome
            .candidates
            .iter()
            .map(|c| c.instrument.clone())
            .collect();
        state.push(outcome);
        state.validate().expect("valid fitting state");
        let json = serde_json::to_vec(&state).expect("serialize");
        let back: FittingState = serde_json::from_slice(&json).expect("deserialize");
        back.validate().expect("round trip");
        println!("   session fitting JSON {} KiB", json.len() / 1024);
    }
}
