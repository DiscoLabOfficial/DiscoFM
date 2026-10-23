//! The copyrighted WAV is read from an existing FL Studio installation.
use discofm_analysis::{AnalysisOptions, read_wav};
use discofm_audio::from_native;
use discofm_chip::Instrument;
use discofm_engine::{RenderRequest, render_note_cancellable};
use discofm_fit::{ComputeMode, FitConfig, FitInput, fit};
use discofm_reference::compare_audio;
use serde::Deserialize;
use std::{fs::File, sync::Arc};

#[derive(Deserialize)]
struct Baseline {
    instrument: Instrument,
    audition: RenderRequest,
}

#[test]
#[ignore = "requires DISCOFM_TEST_STEELDRUM_WAV pointing to the installed FL Studio sample"]
fn steeldrum_auto_modes_improve_independent_spectrum_and_preserve_decay_at_256_evaluations() {
    let path = std::env::var("DISCOFM_TEST_STEELDRUM_WAV")
        .expect("set the installed PERC_SteelDrum_C5.wav path");
    let reference = Arc::new(read_wav(File::open(path).unwrap()).unwrap());
    assert_eq!(reference.sample_rate, 44100);
    assert_eq!(reference.frames.len(), 29696, "reference fixture changed");
    let duration = reference.frames.len() as f64 / f64::from(reference.sample_rate);
    let outcome = fit(
        FitInput {
            reference: Arc::clone(&reference),
            options: AnalysisOptions {
                normalize: true,
                end_seconds: Some(duration),
                ..Default::default()
            },
            note_on_seconds: 0.0,
            key_off_seconds: duration,
            initial: Instrument::default(),
            audition: RenderRequest::default(),
            config: FitConfig {
                evaluations: 256,
                workers: 4,
                compute: ComputeMode::Cpu,
                ..Default::default()
            },
            locks: Default::default(),
            seeds: Vec::new(),
            refine: false,
        },
        || false,
        |_| {},
    )
    .unwrap();
    outcome.validate().unwrap();
    let baseline: Baseline =
        serde_json::from_str(include_str!("fixtures/steeldrum-v3-baseline.json")).unwrap();
    let render = |patch: &Instrument, audition: &RenderRequest| {
        from_native(
            &render_note_cancellable(patch, audition, || false).unwrap(),
            24000,
        )
        .unwrap()
    };
    let old = compare_audio(
        &render(&baseline.instrument, &baseline.audition),
        &reference,
        0.0,
        true,
    )
    .unwrap();
    let new = compare_audio(
        &render(&outcome.candidates[0].instrument, &outcome.context.audition),
        &reference,
        0.0,
        true,
    )
    .unwrap();
    eprintln!(
        "v3: {old:?}\nv4: {new:?}\npatch: {:?}",
        outcome.candidates[0].instrument
    );
    assert!(1.0 - new.spectral_cosine_full < (1.0 - old.spectral_cosine_full) * 0.9);
    assert!(1.0 - new.spectral_cosine_low <= (1.0 - old.spectral_cosine_low) * 1.02);
    assert!(new.envelope_relative_error <= old.envelope_relative_error * 1.02);
    assert!(
        (new.dominant_actual_hz / new.dominant_reference_hz)
            .log2()
            .abs()
            * 1200.0
            < 20.0
    );
}
