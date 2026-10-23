//! Opt-in timing probe for long references: a chip-generated sustained note so
//! no copyrighted audio is needed. Reports wall time and the per-stage sums.
use discofm_analysis::AnalysisOptions;
use discofm_audio::from_native;
use discofm_chip::{Algorithm, Instrument, Value};
use discofm_engine::{RenderRequest, render_note};
use discofm_fit::*;
use std::sync::Arc;

fn target() -> Instrument {
    let mut p = Instrument::default();
    p.channel.algorithm = Algorithm::new(4).unwrap();
    p.channel.feedback = Value::new(5).unwrap();
    for (o, (mul, tl, d1r, sl, d2r)) in p.operators.iter_mut().zip([
        (2, 28, 6, 4, 2),
        (1, 4, 4, 3, 1),
        (3, 34, 8, 5, 3),
        (1, 8, 5, 4, 1),
    ]) {
        o.mul = Value::new(mul).unwrap();
        o.tl = Value::new(tl).unwrap();
        o.ar = Value::new(31).unwrap();
        o.d1r = Value::new(d1r).unwrap();
        o.sl = Value::new(sl).unwrap();
        o.d2r = Value::new(d2r).unwrap();
        o.rr = Value::new(6).unwrap();
    }
    p
}

fn input(seconds: f64, compute: ComputeMode, evaluations: usize) -> FitInput {
    let request = RenderRequest {
        note: 57,
        key_on_seconds: seconds * 0.8,
        total_seconds: seconds,
        ..Default::default()
    };
    let audio = from_native(&render_note(&target(), &request).unwrap(), 44100).unwrap();
    FitInput {
        reference: Arc::new(audio),
        options: AnalysisOptions {
            trim_silence: false,
            pitch_override_hz: Some(discofm_engine::midi_hz(57).unwrap()),
            ..Default::default()
        },
        note_on_seconds: 0.0,
        key_off_seconds: request.key_on_seconds,
        initial: Instrument::default(),
        audition: request,
        config: FitConfig {
            compute,
            evaluations,
            max_seconds: 30.0,
            ..Default::default()
        },
        locks: Default::default(),
        seeds: Vec::new(),
        refine: false,
    }
}

#[test]
#[ignore = "slow timing probe; run in release with --nocapture"]
fn long_reference_timing() {
    let seconds: f64 = std::env::var("DISCOFM_LONG_SECONDS").map_or(6.0, |v| v.parse().unwrap());
    let evaluations: usize =
        std::env::var("DISCOFM_LONG_EVALUATIONS").map_or(128, |v| v.parse().unwrap());
    let request = input(seconds, ComputeMode::Cpu, evaluations).audition;
    let started = std::time::Instant::now();
    let native = render_note(&target(), &request).unwrap();
    let render = started.elapsed().as_secs_f64();
    let started = std::time::Instant::now();
    let _ = from_native(&native, 24000).unwrap();
    println!(
        "{seconds} s note: one render {render:.3} s, one 24 kHz resample {:.3} s",
        started.elapsed().as_secs_f64()
    );
    let modes: Vec<ComputeMode> = match std::env::var("DISCOFM_LONG_COMPUTE").as_deref() {
        Ok("cpu") => vec![ComputeMode::Cpu],
        Ok("gpu") => vec![ComputeMode::Gpu],
        Ok("auto") => vec![ComputeMode::Auto],
        _ => vec![ComputeMode::Gpu, ComputeMode::Cpu, ComputeMode::Auto],
    };
    for compute in modes {
        let mut run = input(seconds, compute, evaluations);
        run.config.workers =
            std::env::var("DISCOFM_LONG_WORKERS").map_or(0, |v| v.parse().unwrap());
        let started = std::time::Instant::now();
        let o = fit(run, || false, |_| {}).unwrap();
        let p = &o.performance;
        println!(
            "{compute:?}: wall {:.2} s, {} renders, best {:.6} | sums: render {:.2} s, resample {:.2} s, features {:.2} s | workers {} backend {} | cpu {:.3} ms gpu {:?} ms | {}",
            started.elapsed().as_secs_f64(),
            o.renders,
            o.candidates[0].loss.total,
            p.render_seconds,
            p.resample_seconds,
            p.features_seconds,
            p.workers,
            p.backend,
            p.cpu_fft_ms,
            p.gpu_fft_ms,
            p.note
        );
    }
}

#[test]
#[ignore = "slow timing probe; run in release with --nocapture"]
fn parallel_render_throughput() {
    let request = input(6.0, ComputeMode::Cpu, 128).audition;
    for threads in [1, 2, 4, 8, 16] {
        let started = std::time::Instant::now();
        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| {
                    discofm_engine::render_note_cancellable(&target(), &request, || false).unwrap()
                });
            }
        });
        let wall = started.elapsed().as_secs_f64();
        println!(
            "{threads:>2} threads: {wall:.3} s wall, {:.2} renders/s",
            threads as f64 / wall
        );
    }
}
