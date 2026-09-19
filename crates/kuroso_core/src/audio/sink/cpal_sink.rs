use crate::audio::sink::ring_buffer::AudioConsumer;
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use ringbuf::traits::Consumer;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

pub struct CpalAudioSink {
    _stream: Stream,
    is_paused: Arc<AtomicBool>,
    volume: Arc<AtomicU32>,
}

impl CpalAudioSink {
    pub fn start(
        device: &Device,
        config: &StreamConfig,
        sample_format: SampleFormat,
        mut consumer: AudioConsumer,
    ) -> Result<Self, String> {
        let is_paused = Arc::new(AtomicBool::new(false));
        let is_paused_cb = Arc::clone(&is_paused);

        let volume = Arc::new(AtomicU32::new(1.0f32.to_bits()));
        let volume_cb = Arc::clone(&volume);

        let err_fn = |err| {
            eprintln!("Audio sink hardware stream error: {err}");
        };

        let stream = match sample_format {
            SampleFormat::F32 => device
                .build_output_stream(
                    config,
                    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                        if is_paused_cb.load(Ordering::Relaxed) {
                            data.fill(0.0);
                            return;
                        }

                        let vol = f32::from_bits(volume_cb.load(Ordering::Relaxed));
                        let read = consumer.pop_slice(data);

                        for sample in &mut data[..read] {
                            *sample = (*sample * vol).clamp(-1.0, 1.0);
                        }

                        if read < data.len() {
                            data[read..].fill(0.0);
                        }
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| format!("Failed to build F32 stream: {e}"))?,

            SampleFormat::I16 => device
                .build_output_stream(
                    config,
                    move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                        if is_paused_cb.load(Ordering::Relaxed) {
                            data.fill(0);
                            return;
                        }

                        let vol = f32::from_bits(volume_cb.load(Ordering::Relaxed));

                        for sample in data.iter_mut() {
                            if let Some(val) = consumer.try_pop() {
                                let scaled = (val * vol).clamp(-1.0, 1.0);
                                *sample = (scaled * i16::MAX as f32) as i16;
                            } else {
                                *sample = 0;
                            }
                        }
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| format!("Failed to build I16 stream: {e}"))?,

            unsupported => return Err(format!("Hardware sample format {unsupported:?} is unsupported")),
        };

        stream.play().map_err(|e| format!("Failed to start stream playback: {e}"))?;

        Ok(Self {
            _stream: stream,
            is_paused,
            volume,
        })
    }

    pub fn pause(&self) {
        self.is_paused.store(true, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.is_paused.store(false, Ordering::Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.is_paused.load(Ordering::Relaxed)
    }

    pub fn set_volume(&self, vol: f32) {
        let clamped = vol.clamp(0.0, 2.0);
        self.volume.store(clamped.to_bits(), Ordering::Relaxed);
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }
}