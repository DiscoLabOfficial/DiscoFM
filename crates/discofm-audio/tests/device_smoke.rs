//! Opt-in OS-path acceptance. Plays short notes and captures only in-memory
//! WASAPI loopback frames. Other applications should be quiet during this test.
#![cfg(windows)]
use cpal::{
    FromSample, Sample, SampleFormat, SizedSample,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use discofm_audio::{DeviceOptions, Playback};
use discofm_chip::{Instrument, Value};
use discofm_engine::{ChipProfile, LiveConfig};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

fn capture<T: SizedSample>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut producer: rtrb::Producer<[f32; 2]>,
    failed: Arc<AtomicBool>,
    notifications: Arc<AtomicUsize>,
) -> cpal::Stream
where
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels);
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                for frame in data.chunks_exact(channels) {
                    let left = f32::from_sample(frame[0]);
                    let right = if channels > 1 {
                        f32::from_sample(frame[1])
                    } else {
                        left
                    };
                    let _ = producer.push([left, right]);
                }
            },
            move |error| match error.kind() {
                cpal::ErrorKind::Xrun
                | cpal::ErrorKind::DeviceChanged
                | cpal::ErrorKind::RealtimeDenied => {
                    notifications.fetch_add(1, Ordering::Relaxed);
                }
                _ => failed.store(true, Ordering::Release),
            },
            Some(Duration::from_secs(3)),
        )
        .unwrap()
}
fn drain(consumer: &mut rtrb::Consumer<[f32; 2]>) -> Vec<[f32; 2]> {
    let mut frames = Vec::new();
    while let Ok(frame) = consumer.pop() {
        frames.push(frame);
    }
    frames
}
fn tone(frames: &[[f32; 2]], hz: f64, rate: u32) -> f64 {
    assert!(frames.len() > 1000, "loopback must deliver real frames");
    let mean = frames.iter().map(|frame| f64::from(frame[0])).sum::<f64>() / frames.len() as f64;
    let (real, imaginary) = frames
        .iter()
        .enumerate()
        .fold((0.0, 0.0), |(re, im), (i, frame)| {
            let phase = std::f64::consts::TAU * hz * i as f64 / f64::from(rate);
            let value = f64::from(frame[0]) - mean;
            (re + value * phase.cos(), im + value * phase.sin())
        });
    2.0 * real.hypot(imaginary) / frames.len() as f64
}
fn window(consumer: &mut rtrb::Consumer<[f32; 2]>) -> Vec<[f32; 2]> {
    drain(consumer);
    thread::sleep(Duration::from_millis(160));
    drain(consumer)
}
fn report(label: &str, frames: &[[f32; 2]], rate: u32) {
    let peak = frames
        .iter()
        .map(|frame| frame[0].abs())
        .fold(0.0_f32, f32::max);
    println!(
        "{label}: frames={}, peak={peak}, A4={}, A5={}",
        frames.len(),
        tone(frames, 440.0, rate),
        tone(frames, 880.0, rate)
    );
}
fn ac_rms(frames: &[[f32; 2]]) -> f64 {
    let mean = frames.iter().map(|frame| f64::from(frame[0])).sum::<f64>() / frames.len() as f64;
    (frames
        .iter()
        .map(|frame| (f64::from(frame[0]) - mean).powi(2))
        .sum::<f64>()
        / frames.len() as f64)
        .sqrt()
}
#[test]
#[ignore = "requires an available Windows output endpoint; plays generated FM and reads WASAPI loopback"]
fn real_cpal_output_loopback_profiles_octave_release_and_stop() {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .expect("default output endpoint");
    let config = device.default_output_config().unwrap();
    let rate = config.sample_rate();
    let (producer, mut consumer) = rtrb::RingBuffer::new(rate as usize * 4);
    let failed = Arc::new(AtomicBool::new(false));
    let notifications = Arc::new(AtomicUsize::new(0));
    let stream = match config.sample_format() {
        SampleFormat::F32 => capture::<f32>(
            &device,
            config.config(),
            producer,
            Arc::clone(&failed),
            Arc::clone(&notifications),
        ),
        SampleFormat::I16 => capture::<i16>(
            &device,
            config.config(),
            producer,
            Arc::clone(&failed),
            Arc::clone(&notifications),
        ),
        SampleFormat::I32 => capture::<i32>(
            &device,
            config.config(),
            producer,
            Arc::clone(&failed),
            Arc::clone(&notifications),
        ),
        format => panic!("loopback test does not support endpoint format {format}"),
    };
    stream.play().unwrap();
    let mut patch = Instrument::default();
    patch.operators[3].rr = Value::new(15).unwrap();
    let mut playback =
        Playback::open(patch, LiveConfig::default(), DeviceOptions::default()).unwrap();
    println!("Opened device: {:?}", playback.info());
    let id = playback.controls().key_on(9).unwrap();
    thread::sleep(Duration::from_millis(160));
    let discrete = window(&mut consumer);
    report("discrete", &discrete, rate);
    let discrete_tone = tone(&discrete, 440.0, rate);
    assert!(
        discrete_tone > 1e-5 && discrete_tone > tone(&discrete, 700.0, rate) * 10.0,
        "discrete tone={discrete_tone}"
    );
    playback
        .controls()
        .set_profile(ChipProfile::Ym3438)
        .unwrap();
    thread::sleep(Duration::from_millis(100));
    let cmos = window(&mut consumer);
    report("cmos", &cmos, rate);
    let cmos_tone = tone(&cmos, 440.0, rate);
    assert!(
        cmos_tone > 1e-5 && cmos_tone > tone(&cmos, 700.0, rate) * 10.0,
        "CMOS tone={cmos_tone}"
    );
    playback.controls().note_off(id);
    assert_eq!(playback.controls().held_voice(), None);
    thread::sleep(Duration::from_millis(160));
    let release = window(&mut consumer);
    report("release", &release, rate);
    let release_tone = tone(&release, 440.0, rate);
    assert!(
        release_tone < cmos_tone * 0.1,
        "release must decay: held={cmos_tone}, released={release_tone}, stats={:?}",
        playback.stats()
    );
    playback.controls().set_octave(5).unwrap();
    playback.controls().key_on(9).unwrap();
    thread::sleep(Duration::from_millis(100));
    let octave = window(&mut consumer);
    let octave_tone = tone(&octave, 880.0, rate);
    assert!(
        octave_tone > 1e-5 && octave_tone > tone(&octave, 440.0, rate) * 10.0,
        "octave tone={octave_tone}"
    );
    playback.controls().stop();
    thread::sleep(Duration::from_millis(100));
    let stopped = window(&mut consumer);
    assert!(
        tone(&stopped, 880.0, rate) < octave_tone * 0.1,
        "Stop must mute queued old voice"
    );
    assert!(
        ac_rms(&stopped) < ac_rms(&octave) * 0.1,
        "Stop must suppress broadband audio as well as the target tone"
    );
    playback.check_health().unwrap();
    let stats = playback.stats();
    assert!(stats.callbacks > 10 && stats.note_frames > 1000);
    assert!(!failed.load(Ordering::Acquire), "loopback callback error");
    println!(
        "Device: {:?}; stats: {:?}; capture notifications: {}; tone amplitudes: discrete={discrete_tone:.8}, CMOS={cmos_tone:.8}, octave={octave_tone:.8}",
        playback.info(),
        stats,
        notifications.load(Ordering::Relaxed)
    );
}
