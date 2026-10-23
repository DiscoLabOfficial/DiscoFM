use discofm_chip::{Channel, Frequency, Instrument, Value};
use discofm_engine::{
    ChipProfile, ClockConfig, LiveConfig, LiveRenderer, RenderRequest, render_note,
};

#[test]
fn streaming_native_frames_match_offline_across_blocks_profiles_clocks_and_channels() {
    for profile in [ChipProfile::Ym2612Discrete, ChipProfile::Ym3438] {
        for clock in [ClockConfig::Ntsc, ClockConfig::Pal] {
            for number in 1..=6 {
                let channel = Channel::new(number).unwrap();
                let mut patch = Instrument::default();
                if number == 3 {
                    patch.channel3_special = Some([
                        Frequency::new(700, 3).unwrap(),
                        Frequency::new(1100, 4).unwrap(),
                        Frequency::new(1500, 2).unwrap(),
                    ]);
                }
                let request = RenderRequest {
                    profile,
                    clock,
                    channel,
                    key_on_seconds: 96.0 / clock.native_hz(),
                    total_seconds: 0.025,
                    ..RenderRequest::default()
                };
                let expected = render_note(&patch, &request).unwrap();
                let mut renderer = LiveRenderer::new(LiveConfig {
                    channel,
                    profile,
                    clock,
                    tuning_cents: request.tuning_cents,
                })
                .unwrap();
                renderer
                    .note_on(&patch, request.note, request.velocity)
                    .unwrap();
                let mut actual = vec![[0.0; 2]; expected.frames.len()];
                let mut offset = 0;
                for chunk in [1, 23, 24, 25, 23] {
                    renderer
                        .render_into(&mut actual[offset..offset + chunk])
                        .unwrap();
                    offset += chunk;
                }
                assert_eq!(offset, 96);
                renderer.release().unwrap();
                for chunk in actual[offset..].chunks_mut(137) {
                    renderer.render_into(chunk).unwrap();
                }
                let mismatch = actual
                    .iter()
                    .zip(&expected.frames)
                    .position(|(a, b)| a != b);
                assert_eq!(
                    mismatch, None,
                    "{profile:?}, {clock:?}, channel {number}: first different native frame"
                );
            }
        }
    }
}

#[test]
fn release_stop_retrigger_and_invalid_commands_preserve_their_contracts() {
    let patch = Instrument::default();
    let mut live = LiveRenderer::new(LiveConfig::default()).unwrap();
    let mut initial = [[0.0; 2]; 256];
    live.note_on(&patch, 69, Value::new(127).unwrap()).unwrap();
    live.render_into(&mut initial).unwrap();
    assert!(live.note_on(&patch, 127, Value::new(127).unwrap()).is_err());
    assert!(live.is_held());
    assert!(live.set_profile(ChipProfile::RawDiagnostic).is_err());
    assert!(live.is_held());
    live.release().unwrap();
    live.release().unwrap();
    assert!(!live.is_held());
    let mut tail = [[0.0; 2]; 256];
    live.render_into(&mut tail).unwrap();
    assert!(tail.iter().flatten().any(|v| *v != 0.0));
    live.stop().unwrap();
    live.render_into(&mut tail).unwrap();
    assert_eq!(tail, [[0.0; 2]; 256]);
    live.note_on(&patch, 69, Value::new(127).unwrap()).unwrap();
    live.render_into(&mut tail).unwrap();
    assert_eq!(tail, initial);
    live.note_on(&patch, 69, Value::new(127).unwrap()).unwrap();
    live.release().unwrap(); // Before any note frames.
    live.render_into(&mut tail).unwrap();
    assert!(!live.is_held());
}

#[test]
fn release_before_or_after_one_frame_preserves_minimum_key_on_bus_spacing() {
    for profile in [ChipProfile::Ym2612Discrete, ChipProfile::Ym3438] {
        for clock in [ClockConfig::Ntsc, ClockConfig::Pal] {
            for before_release in [0, 1] {
                let patch = Instrument::default();
                let request = RenderRequest {
                    profile,
                    clock,
                    key_on_seconds: 48.0 / clock.tick_hz(),
                    total_seconds: 0.02,
                    ..RenderRequest::default()
                };
                let expected = render_note(&patch, &request).unwrap();
                let mut renderer = LiveRenderer::new(LiveConfig {
                    profile,
                    clock,
                    ..LiveConfig::default()
                })
                .unwrap();
                renderer
                    .note_on(&patch, request.note, request.velocity)
                    .unwrap();
                let mut actual = vec![[0.0; 2]; expected.frames.len()];
                renderer.render_into(&mut actual[..before_release]).unwrap();
                renderer.release().unwrap();
                renderer.render_into(&mut actual[before_release..]).unwrap();
                assert_eq!(
                    actual, expected.frames,
                    "{profile:?}/{clock:?}, release after {before_release} frames"
                );
            }
        }
    }
}
