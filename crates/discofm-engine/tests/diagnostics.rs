use discofm_chip::{Instrument, SsgEg, Value};
use discofm_engine::{
    ChipBackend, ChipProfile, ClockConfig, NukedChip, RenderRequest, render_diagnostics,
    render_note,
};

#[test]
fn envelope_traces_use_actual_chip_state_and_preserve_offline_frames_and_bus_ticks() {
    for profile in [ChipProfile::Ym2612Discrete, ChipProfile::Ym3438] {
        for clock in [ClockConfig::Ntsc, ClockConfig::Pal] {
            let mut patch = Instrument::default();
            patch.operators[0].ar = Value::new(15).unwrap();
            patch.operators[1].ssg = SsgEg::try_from(10).unwrap();
            let request = RenderRequest {
                profile,
                clock,
                key_on_seconds: 0.020123,
                total_seconds: 0.05,
                ..RenderRequest::default()
            };
            let expected = render_note(&patch, &request).unwrap();
            let preview = render_diagnostics(&patch, &request, 71, || false).unwrap();
            assert_eq!(preview.audio.frames, expected.frames);
            assert!(preview.envelopes.len() <= 71);
            for point in preview.envelopes.iter().step_by(13) {
                let end = expected.schedule.capture_start + point.frame as u64 * 24;
                let events = expected
                    .schedule
                    .events
                    .iter()
                    .filter(|event| event.tick < end)
                    .copied()
                    .collect::<Vec<_>>();
                let mut chip = NukedChip::new(profile).unwrap();
                // render_trace cannot capture zero ticks: an independent bus driver
                // clocks precisely to the snapshot boundary instead.
                chip.reset().unwrap();
                let mut tick = 0;
                let mut scratch = [[0; 2]; 1024];
                for event in events {
                    while tick < event.tick {
                        let count = (event.tick - tick).min(1024) as usize;
                        chip.clock_into(&mut scratch[..count]).unwrap();
                        tick += count as u64;
                    }
                    chip.write_port(event.port as u8, event.data).unwrap();
                }
                while tick < end {
                    let count = (end - tick).min(1024) as usize;
                    chip.clock_into(&mut scratch[..count]).unwrap();
                    tick += count as u64;
                }
                assert_eq!(
                    point.operators,
                    chip.snapshot(request.channel).unwrap().operators
                );
            }
            assert!(render_diagnostics(&patch, &request, 0, || false).is_err());
            assert!(render_diagnostics(&patch, &request, 71, || true).is_err());
        }
    }
}
