use audiopus::coder::{Decoder as OpusDecoder, GenericCtl};
use audiopus::{Channels as OpusChannels, SampleRate as OpusSampleRate};
use std::fs::File;
use std::path::Path;
use std::time::Duration;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder as SymphoniaDecoder, DecoderOptions, CODEC_TYPE_OPUS};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSpec {
    pub sample_rate: u32,
    pub channels: u16,
    pub duration: Option<Duration>,
}

enum DecoderBackend {
    Symphonia(Box<dyn SymphoniaDecoder>),
    Opus(OpusDecoder),
}

pub struct AudioDecoder {
    format_reader: Box<dyn FormatReader>,
    backend: DecoderBackend,
    track_id: u32,
    sample_buf: Option<SampleBuffer<f32>>,
    opus_pcm_buf: Vec<f32>,
    spec: AudioSpec,
    consecutive_errors: usize,
}

const MAX_CONSECUTIVE_DECODE_ERRORS: usize = 20;
// Maximum frame size per RFC 6716 is 120ms (5760 samples @ 48kHz per channel)
const OPUS_MAX_FRAME_SAMPLES: usize = 5760;

impl AudioDecoder {
    /// Opens and probes any audio file, dynamically binding the optimal codec engine.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let path_ref = path.as_ref();
        if !path_ref.exists() {
            return Err(format!("Audio file does not exist: {}", path_ref.display()));
        }

        let file = File::open(path_ref).map_err(|e| format!("IO open failure for {}: {e}", path_ref.display()))?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path_ref.extension().and_then(|s| s.to_str()) {
            hint.with_extension(ext);
        }

        let format_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let metadata_opts: MetadataOptions = Default::default();

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &metadata_opts)
            .map_err(|e| format!("Failed to identify audio container for {}: {e}", path_ref.display()))?;

        let format_reader = probed.format;

        let track = format_reader
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
            .ok_or_else(|| format!("No supported audio streams found in {}", path_ref.display()))?;

        let track_id = track.id;
        let codec_params = track.codec_params.clone();

        let sample_rate = codec_params.sample_rate.unwrap_or(48_000);
        let channels = codec_params
            .channels
            .map(|c| c.count() as u16)
            .unwrap_or(2)
            .max(1);

        let duration = codec_params.n_frames.map(|frames| {
            let secs = frames as f64 / sample_rate as f64;
            Duration::from_secs_f64(secs)
        });

        let spec = AudioSpec {
            sample_rate,
            channels,
            duration,
        };

        if codec_params.codec == CODEC_TYPE_OPUS {
            let opus_channels = if channels == 1 {
                OpusChannels::Mono
            } else {
                OpusChannels::Stereo
            };

            let opus_sample_rate = match sample_rate {
                8_000 => OpusSampleRate::Hz8000,
                12_000 => OpusSampleRate::Hz12000,
                16_000 => OpusSampleRate::Hz16000,
                24_000 => OpusSampleRate::Hz24000,
                _ => OpusSampleRate::Hz48000,
            };

            let opus_decoder = OpusDecoder::new(opus_sample_rate, opus_channels)
                .map_err(|e| format!("Failed to initialize libopus decoder: {e:?}"))?;

            let opus_pcm_buf = vec![0.0f32; OPUS_MAX_FRAME_SAMPLES * (channels as usize)];

            Ok(Self {
                format_reader,
                backend: DecoderBackend::Opus(opus_decoder),
                track_id,
                sample_buf: None,
                opus_pcm_buf,
                spec,
                consecutive_errors: 0,
            })
        } else {
            let decoder_opts: DecoderOptions = Default::default();
            let registry = symphonia::default::get_codecs();
            let symphonia_dec = registry
                .make(&codec_params, &decoder_opts)
                .map_err(|e| format!("Unsupported audio codec: {e}"))?;

            Ok(Self {
                format_reader,
                backend: DecoderBackend::Symphonia(symphonia_dec),
                track_id,
                sample_buf: None,
                opus_pcm_buf: Vec::new(),
                spec,
                consecutive_errors: 0,
            })
        }
    }

    pub fn spec(&self) -> &AudioSpec {
        &self.spec
    }

    /// Accurate seek to timestamp with complete codec state flush
    pub fn seek(&mut self, time: Duration) -> Result<(), String> {
        let seek_to = SeekTo::Time {
            time: symphonia::core::units::Time::from(time.as_secs_f64()),
            track_id: Some(self.track_id),
        };

        self.format_reader
            .seek(SeekMode::Accurate, seek_to)
            .map_err(|e| format!("Accurate seek operation failed: {e}"))?;

        match &mut self.backend {
            DecoderBackend::Symphonia(dec) => dec.reset(),
            DecoderBackend::Opus(dec) => {
                let _ = dec.reset_state();
            }
        }

        self.consecutive_errors = 0;
        Ok(())
    }

    /// Decodes next audio packet into normalized, interleaved f32 samples.
    /// Returns `Ok(None)` on stream termination.
    pub fn next_packet(&mut self) -> Result<Option<&[f32]>, String> {
        loop {
            let packet = match self.format_reader.next_packet() {
                Ok(packet) => {
                    self.consecutive_errors = 0;
                    packet
                }
                Err(SymphoniaError::IoError(ref err))
                    if err.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(None);
                }
                Err(SymphoniaError::ResetRequired) => {
                    match &mut self.backend {
                        DecoderBackend::Symphonia(dec) => dec.reset(),
                        DecoderBackend::Opus(dec) => {
                            let _ = dec.reset_state();
                        }
                    }
                    continue;
                }
                Err(err) => {
                    self.consecutive_errors += 1;
                    if self.consecutive_errors > MAX_CONSECUTIVE_DECODE_ERRORS {
                        return Err(format!("Exceeded decode failure threshold: {err}"));
                    }
                    continue;
                }
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            match &mut self.backend {
                DecoderBackend::Opus(opus_dec) => {
                    let raw_bytes: &[u8] = &packet.data[..];

                    // Discard Ogg Opus non-audio encapsulation headers
                    if raw_bytes.starts_with(b"OpusHead") || raw_bytes.starts_with(b"OpusTags") {
                        continue;
                    }

                    match opus_dec.decode_float(Some(raw_bytes), &mut self.opus_pcm_buf, false) {
                        Ok(frames_decoded) => {
                            let total_samples = frames_decoded * (self.spec.channels as usize);
                            return Ok(Some(&self.opus_pcm_buf[..total_samples]));
                        }
                        Err(err) => {
                            self.consecutive_errors += 1;
                            if self.consecutive_errors > MAX_CONSECUTIVE_DECODE_ERRORS {
                                return Err(format!("Opus bitstream decode error: {err:?}"));
                            }
                            continue;
                        }
                    }
                }
                DecoderBackend::Symphonia(symphonia_dec) => {
                    let decoded_buffer = match symphonia_dec.decode(&packet) {
                        Ok(buf) => buf,
                        Err(SymphoniaError::DecodeError(_)) => {
                            self.consecutive_errors += 1;
                            if self.consecutive_errors > MAX_CONSECUTIVE_DECODE_ERRORS {
                                return Err("Excessive corrupt packets encountered".into());
                            }
                            continue;
                        }
                        Err(err) => return Err(format!("Fatal decoding failure: {err}")),
                    };

                    if self.sample_buf.is_none() {
                        let spec = *decoded_buffer.spec();
                        let capacity = decoded_buffer.capacity() as u64;
                        self.sample_buf = Some(SampleBuffer::new(capacity, spec));
                    }

                    if let Some(ref mut sample_buf) = self.sample_buf {
                        sample_buf.copy_interleaved_ref(decoded_buffer);
                        return Ok(Some(sample_buf.samples()));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decoder_non_existent_file() {
        match AudioDecoder::open("crates/does_not_exist.flac") {
            Ok(_) => panic!("Expected error for missing file"),
            Err(err) => assert!(err.contains("does not exist")),
        }
    }
}