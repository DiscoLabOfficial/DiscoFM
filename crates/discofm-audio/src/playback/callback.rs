use super::{
    PlaybackError,
    transport::{Frame, Shared},
};
use cpal::{FromSample, SampleFormat, SizedSample, SupportedStreamConfig, traits::DeviceTrait};
use rtrb::Consumer;
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

/// No allocation, locks, I/O or synthesis. The stale-pop budget is shared across
/// the entire callback, so a concurrent producer cannot make draining unbounded.
pub(super) struct Callback {
    input: Consumer<Frame>,
    shared: Arc<Shared>,
    channels: usize,
    capacity: usize,
}
impl Callback {
    pub fn new(
        input: Consumer<Frame>,
        shared: Arc<Shared>,
        channels: usize,
        capacity: usize,
    ) -> Self {
        Self {
            input,
            shared,
            channels,
            capacity,
        }
    }
    pub fn fill<T: SizedSample + FromSample<f32>>(&mut self, output: &mut [T]) {
        output.fill(T::EQUILIBRIUM);
        let mut stale_budget = self.capacity;
        let mut underruns = 0;
        let mut played = 0;
        for destination in output.chunks_exact_mut(self.channels) {
            let generation = self.shared.generation.load(Ordering::Acquire);
            let muted = self.shared.muted.load(Ordering::Acquire);
            let mut frame = None;
            loop {
                if stale_budget == 0 {
                    break;
                }
                let Ok(next) = self.input.pop() else {
                    break;
                };
                if next.generation() == generation {
                    match next {
                        Frame::Sample { samples, last, .. } => {
                            if !muted {
                                frame = Some(samples);
                            }
                            if last {
                                self.shared.completed.store(generation, Ordering::Release);
                            }
                        }
                        Frame::End { .. } => {
                            self.shared.completed.store(generation, Ordering::Release)
                        }
                    }
                    break;
                }
                stale_budget -= 1;
            }
            if let Some(frame) = frame {
                played += 1;
                if self.channels == 1 {
                    destination[0] = T::from_sample(((frame[0] + frame[1]) * 0.5).clamp(-1.0, 1.0));
                } else {
                    destination[0] = T::from_sample(frame[0].clamp(-1.0, 1.0));
                    destination[1] = T::from_sample(frame[1].clamp(-1.0, 1.0));
                }
            } else if !muted && self.shared.completed.load(Ordering::Acquire) != generation {
                underruns += 1;
            }
        }
        self.shared.callbacks.fetch_add(1, Ordering::Relaxed);
        self.shared
            .device_frames
            .fetch_add(output.len() / self.channels, Ordering::Relaxed);
        self.shared.note_frames.fetch_add(played, Ordering::Relaxed);
        self.shared
            .underruns
            .fetch_add(underruns, Ordering::Relaxed);
    }
}
pub(super) fn publish_error(shared: &Shared, kind: cpal::ErrorKind) {
    let code = match kind {
        cpal::ErrorKind::DeviceChanged
        | cpal::ErrorKind::Xrun
        | cpal::ErrorKind::RealtimeDenied => {
            shared.notifications.fetch_add(1, Ordering::Relaxed);
            return;
        }
        cpal::ErrorKind::DeviceNotAvailable => 1,
        cpal::ErrorKind::StreamInvalidated => 2,
        _ => 3,
    };
    shared.failure.store(code, Ordering::Release);
    shared.muted.store(true, Ordering::Release);
}
fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &SupportedStreamConfig,
    mut callback: Callback,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, PlaybackError> {
    Ok(device.build_output_stream(
        config.config(),
        move |data: &mut [T], _| callback.fill(data),
        move |error| publish_error(&shared, error.kind()),
        Some(Duration::from_secs(3)),
    )?)
}
pub(super) fn build_stream(
    device: &cpal::Device,
    config: &SupportedStreamConfig,
    callback: Callback,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, PlaybackError> {
    match config.sample_format() {
        SampleFormat::F32 => build::<f32>(device, config, callback, shared),
        SampleFormat::F64 => build::<f64>(device, config, callback, shared),
        SampleFormat::I8 => build::<i8>(device, config, callback, shared),
        SampleFormat::I16 => build::<i16>(device, config, callback, shared),
        SampleFormat::I24 => build::<cpal::I24>(device, config, callback, shared),
        SampleFormat::I32 => build::<i32>(device, config, callback, shared),
        SampleFormat::I64 => build::<i64>(device, config, callback, shared),
        SampleFormat::U8 => build::<u8>(device, config, callback, shared),
        SampleFormat::U16 => build::<u16>(device, config, callback, shared),
        SampleFormat::U32 => build::<u32>(device, config, callback, shared),
        SampleFormat::U64 => build::<u64>(device, config, callback, shared),
        _ => Err(PlaybackError::Config("unsupported device sample format")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn callback(channels: usize) -> (rtrb::Producer<Frame>, Callback, Arc<Shared>) {
        let (producer, consumer) = rtrb::RingBuffer::new(4);
        let shared = Arc::new(Shared::default());
        shared.generation.store(1, Ordering::Release);
        shared.muted.store(false, Ordering::Release);
        (
            producer,
            Callback::new(consumer, Arc::clone(&shared), channels, 4),
            shared,
        )
    }
    #[test]
    fn sample_formats_channel_mapping_and_underruns_initialize_every_sample() {
        let (mut producer, mut render, shared) = callback(2);
        producer
            .push(Frame::Sample {
                generation: 1,
                samples: [0.25, -0.5],
                last: false,
            })
            .unwrap();
        let mut output = [99.0_f32; 5];
        render.fill(&mut output);
        assert_eq!(output, [0.25, -0.5, 0.0, 0.0, 0.0]);
        assert_eq!(shared.stats().underrun_frames, 1);
        let (mut producer, mut render, _) = callback(1);
        producer
            .push(Frame::Sample {
                generation: 1,
                samples: [0.25, -0.5],
                last: false,
            })
            .unwrap();
        let mut mono = [99.0_f64; 2];
        render.fill(&mut mono);
        assert_eq!(mono, [-0.125, 0.0]);
        let (mut producer, mut render, _) = callback(6);
        producer
            .push(Frame::Sample {
                generation: 1,
                samples: [-2.0, 2.0],
                last: false,
            })
            .unwrap();
        let mut surround = [99.0_f32; 6];
        render.fill(&mut surround);
        assert_eq!(surround, [-1.0, 1.0, 0.0, 0.0, 0.0, 0.0]);
        let (mut producer, mut render, _) = callback(2);
        producer
            .push(Frame::Sample {
                generation: 1,
                samples: [0.5, -0.5],
                last: false,
            })
            .unwrap();
        let mut signed = [99_i16; 4];
        render.fill(&mut signed);
        assert_eq!(signed, [16384, -16384, 0, 0]);
        let (_, mut render, _) = callback(2);
        let mut unsigned = [0_u16; 4];
        render.fill(&mut unsigned);
        assert_eq!(unsigned, [32768; 4]);
    }
    #[test]
    fn stop_and_generation_changes_discard_full_and_partially_consumed_old_audio() {
        let (mut producer, mut render, shared) = callback(2);
        for _ in 0..4 {
            producer
                .push(Frame::Sample {
                    generation: 1,
                    samples: [0.5; 2],
                    last: false,
                })
                .unwrap();
        }
        let mut output = [0.0_f32; 2];
        render.fill(&mut output);
        assert_eq!(output, [0.5; 2]);
        shared.muted.store(true, Ordering::Release);
        shared.generation.store(2, Ordering::Release);
        render.fill(&mut output);
        assert_eq!(output, [0.0; 2]);
        assert_eq!(producer.slots(), 4);
        producer
            .push(Frame::Sample {
                generation: 2,
                samples: [0.8; 2],
                last: false,
            })
            .unwrap();
        producer
            .push(Frame::Sample {
                generation: 3,
                samples: [0.2; 2],
                last: false,
            })
            .unwrap();
        shared.generation.store(3, Ordering::Release);
        shared.muted.store(false, Ordering::Release);
        render.fill(&mut output);
        assert_eq!(output, [0.2; 2]);
        // Exhaust the callback-wide stale budget, then recover next callback.
        for _ in 0..4 {
            producer
                .push(Frame::Sample {
                    generation: 2,
                    samples: [0.9; 2],
                    last: false,
                })
                .unwrap();
        }
        render.fill(&mut output);
        assert_eq!(output, [0.0; 2]);
        assert_eq!(producer.slots(), 4);
        producer
            .push(Frame::Sample {
                generation: 3,
                samples: [0.3; 2],
                last: false,
            })
            .unwrap();
        render.fill(&mut output);
        assert_eq!(output, [0.3; 2]);
        producer
            .push(Frame::Sample {
                generation: 3,
                samples: [0.7; 2],
                last: false,
            })
            .unwrap();
        publish_error(&shared, cpal::ErrorKind::StreamInvalidated);
        render.fill(&mut output);
        assert_eq!(output, [0.0; 2]);
    }
    #[test]
    fn asynchronous_device_errors_are_published_without_logging_or_rebuilding() {
        let (_, _, shared) = callback(2);
        for kind in [
            cpal::ErrorKind::DeviceChanged,
            cpal::ErrorKind::Xrun,
            cpal::ErrorKind::RealtimeDenied,
        ] {
            publish_error(&shared, kind);
        }
        assert_eq!(shared.failure.load(Ordering::Acquire), 0);
        assert_eq!(shared.stats().notifications, 3);
        publish_error(&shared, cpal::ErrorKind::DeviceNotAvailable);
        assert_eq!(shared.failure.load(Ordering::Acquire), 1);
        assert!(shared.muted.load(Ordering::Acquire));
        publish_error(&shared, cpal::ErrorKind::StreamInvalidated);
        assert_eq!(shared.failure.load(Ordering::Acquire), 2);
    }
    #[test]
    fn final_clip_sample_is_played_and_eof_silence_is_not_an_underrun_or_a_stale_completion() {
        let (mut producer, mut render, shared) = callback(2);
        producer
            .push(Frame::Sample {
                generation: 1,
                samples: [0.75, -0.5],
                last: true,
            })
            .unwrap();
        let mut output = [9.0_f32; 8];
        render.fill(&mut output);
        assert_eq!(output, [0.75, -0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(shared.completed.load(Ordering::Acquire), 1);
        assert_eq!(shared.stats().underrun_frames, 0);
        shared.generation.store(2, Ordering::Release);
        producer.push(Frame::End { generation: 1 }).unwrap();
        producer
            .push(Frame::Sample {
                generation: 2,
                samples: [0.2; 2],
                last: false,
            })
            .unwrap();
        render.fill(&mut output[..2]);
        assert_eq!(&output[..2], &[0.2; 2]);
        assert_eq!(shared.completed.load(Ordering::Acquire), 1);
        producer.push(Frame::End { generation: 2 }).unwrap();
        render.fill(&mut output);
        assert_eq!(output, [0.0; 8]);
        assert_eq!(shared.completed.load(Ordering::Acquire), 2);
        assert_eq!(shared.stats().underrun_frames, 0);
    }
}
