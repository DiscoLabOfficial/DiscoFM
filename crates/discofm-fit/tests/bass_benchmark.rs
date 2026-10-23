//! The copyrighted FL Studio WAV is supplied locally, never checked in.
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
#[ignore = "requires DISCOFM_TEST_BASS_WAV pointing to the installed FL Studio sample"]
fn fl_studio_bass_improves_independent_spectrum_and_envelope_at_the_same_512_budget() {
    let path =
        std::env::var("DISCOFM_TEST_BASS_WAV").expect("set the installed BASS_EfEm_C2.wav path");
    let reference = Arc::new(read_wav(File::open(path).unwrap()).unwrap());
    assert_eq!(reference.sample_rate, 44100);
    assert_eq!(
        reference.frames.len(),
        10510,
        "fixture changed: remeasure the M7 baseline first"
    );
    let duration = reference.frames.len() as f64 / f64::from(reference.sample_rate);
    let input = FitInput {
        reference: Arc::clone(&reference),
        options: AnalysisOptions {
            end_seconds: Some(duration),
            ..Default::default()
        },
        note_on_seconds: 0.0,
        key_off_seconds: 0.2,
        initial: Instrument::default(),
        audition: RenderRequest::default(),
        config: FitConfig {
            compute: ComputeMode::Cpu,
            ..Default::default()
        },
        locks: Default::default(),
        seeds: Vec::new(),
        refine: false,
    };
    let outcome = fit(input, || false, |_| {}).unwrap();
    let baseline: Baseline =
        serde_json::from_str(include_str!("fixtures/bass-m7-baseline.json")).unwrap();
    let old = from_native(
        &render_note_cancellable(&baseline.instrument, &baseline.audition, || false).unwrap(),
        24000,
    )
    .unwrap();
    let new = from_native(
        &render_note_cancellable(
            &outcome.candidates[0].instrument,
            &outcome.context.audition,
            || false,
        )
        .unwrap(),
        24000,
    )
    .unwrap();
    let old = compare_audio(&old, &reference, 0.0, true).unwrap();
    let new = compare_audio(&new, &reference, 0.0, true).unwrap();
    eprintln!(
        "M7 independent: {old:?}\nM8 independent: {new:?}\nM8 loss {} -> {}, time {}s, renders {}",
        outcome.initial_loss.total,
        outcome.candidates[0].loss.total,
        outcome.elapsed_seconds,
        outcome.renders
    );
    eprintln!(
        "best patch: {:?}\nloss: {:?}",
        outcome.candidates[0].instrument, outcome.candidates[0].loss
    );
    let old_distances = [
        1.0 - old.spectral_cosine_low,
        1.0 - old.spectral_cosine_full,
        old.envelope_relative_error,
    ];
    let new_distances = [
        1.0 - new.spectral_cosine_low,
        1.0 - new.spectral_cosine_full,
        new.envelope_relative_error,
    ];
    assert!(
        old_distances
            .iter()
            .zip(new_distances)
            .all(|(old, new)| new <= old * 1.02)
    );
    assert!(
        old_distances
            .iter()
            .zip(new_distances)
            .any(|(old, new)| new < old * 0.9)
    );
    assert!(
        (new.dominant_actual_hz / new.dominant_reference_hz)
            .log2()
            .abs()
            * 1200.0
            < 20.0
    );
}
