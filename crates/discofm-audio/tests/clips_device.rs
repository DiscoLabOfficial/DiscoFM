use discofm_audio::{AudioBuffer, DeviceOptions, Playback};
use discofm_chip::Instrument;
use discofm_engine::LiveConfig;
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
fn tone(rate: u32, hz: f32, seconds: f32) -> Arc<AudioBuffer> {
    Arc::new(AudioBuffer {
        sample_rate: rate,
        frames: (0..(rate as f32 * seconds).round() as usize)
            .map(|i| {
                let v = (std::f32::consts::TAU * hz * i as f32 / rate as f32).sin() * 0.08;
                [v; 2]
            })
            .collect(),
    })
}
fn finish(playback: &mut Playback) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !playback.controls().clip_finished() && Instant::now() < deadline {
        playback.check_health().unwrap();
        thread::sleep(Duration::from_millis(5));
    }
    assert!(playback.controls().clip_finished());
}
#[test]
#[ignore = "requires an available output endpoint; briefly plays two quiet test tones"]
fn real_cpal_ab_clips_complete_with_exact_frame_counts_and_switch_and_stop() {
    let mut playback = Playback::open(
        Instrument::default(),
        LiveConfig::default(),
        DeviceOptions::default(),
    )
    .unwrap();
    let rate = playback.info().sample_rate;
    for clip in [tone(48000, 440.0, 0.08), tone(44100, 660.0, 0.09)] {
        let expected = (clip.frames.len() as f64 * f64::from(rate) / f64::from(clip.sample_rate))
            .round() as usize;
        let before = playback.stats().note_frames;
        playback.controls().play_clip(clip).unwrap();
        finish(&mut playback);
        assert_eq!(playback.stats().note_frames - before, expected);
        let underruns = playback.stats().underrun_frames;
        thread::sleep(Duration::from_millis(30));
        assert_eq!(
            playback.stats().underrun_frames,
            underruns,
            "normal EOF must stay silent without accumulating underruns"
        );
    }
    playback
        .controls()
        .play_clip(tone(8000, 220.0, 0.5))
        .unwrap();
    thread::sleep(Duration::from_millis(20));
    playback
        .controls()
        .play_clip(tone(96000, 880.0, 0.05))
        .unwrap();
    finish(&mut playback);
    playback
        .controls()
        .play_clip(tone(48000, 440.0, 0.5))
        .unwrap();
    playback.controls().stop();
    thread::sleep(Duration::from_millis(50));
    let frames = playback.stats().note_frames;
    thread::sleep(Duration::from_millis(30));
    assert_eq!(playback.stats().note_frames, frames);
    playback.check_health().unwrap();
    println!("{} Hz, final stats {:?}", rate, playback.stats());
}
