use discofm_chip::{Channel, Frequency, Instrument, SsgEg, Value};
use discofm_engine::{ChipBackend, ChipProfile, NukedChip, RenderRequest, render_note};
#[test]
fn concurrent_instances_match_serial_across_profiles_ch3_lfo_ssg_and_reset() {
    let jobs: Vec<_> = (0..8)
        .map(|i| {
            let mut patch = Instrument::default();
            patch.global.lfo_enabled = true;
            patch.global.lfo_rate = Value::new(i).unwrap();
            patch.channel.fms = Value::new(7).unwrap();
            patch.channel.ams = Value::new(2).unwrap();
            patch.operators[3].am = true;
            patch.operators[3].ssg = SsgEg::try_from(8 + i).unwrap();
            if i % 2 == 0 {
                patch.channel3_special = Some([
                    Frequency::new(700, 3).unwrap(),
                    Frequency::new(1100, 4).unwrap(),
                    Frequency::new(1500, 2).unwrap(),
                ]);
            }
            let request = RenderRequest {
                profile: if i % 3 == 0 {
                    ChipProfile::Ym3438
                } else {
                    ChipProfile::Ym2612Discrete
                },
                channel: Channel::THREE,
                key_on_seconds: 0.04,
                total_seconds: 0.07,
                ..Default::default()
            };
            (patch, request)
        })
        .collect();
    let serial: Vec<_> = jobs
        .iter()
        .map(|(p, r)| render_note(p, r).unwrap().frames)
        .collect();
    let concurrent = std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .iter()
            .map(|(p, r)| {
                scope.spawn(move || {
                    let mut chip = NukedChip::new(r.profile).unwrap();
                    chip.write_port(0, 0x22).unwrap();
                    chip.clock_into(&mut [[0; 2]; 48]).unwrap();
                    chip.reset().unwrap();
                    let mut fresh = NukedChip::new(r.profile).unwrap();
                    assert_eq!(chip.status().unwrap(), fresh.status().unwrap());
                    let mut a = [[0; 2]; 96];
                    let mut b = a;
                    chip.clock_into(&mut a).unwrap();
                    fresh.clock_into(&mut b).unwrap();
                    assert_eq!(a, b);
                    render_note(p, r).unwrap().frames
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(concurrent, serial);
}
