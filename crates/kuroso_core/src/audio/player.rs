use crate::audio::decoder::AudioDecoder;
use crate::audio::dsp::AudioResampler;
use crate::audio::hardware::{HardwareProber, PlaybackStrategy};
use crate::audio::queue::PlaybackQueue;
use crate::audio::sink::{create_audio_ring_buffer, CpalAudioSink};
use crate::library::database::LibraryDatabase;
use crate::library::queries::TrackView;
use crate::library::types::TrackId;
use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{Device, SampleFormat, StreamConfig};
use crossbeam_channel::{unbounded, Receiver, Sender};
use ringbuf::traits::Producer;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerState {
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone)]
pub struct PlayerSnapshot {
    pub state: PlayerState,
    pub current_track: Option<TrackView>,
    pub elapsed: Duration,
    pub duration: Option<Duration>,
    pub volume: f32,
    pub playlist_len: usize,
    pub playlist_index: Option<usize>,
    pub up_next_count: usize,
}

pub enum PlayerCommand {
    Play,
    Pause,
    TogglePause,
    Next,
    Prev,
    Seek(Duration),
    SetVolume(f32),
    Stop,
}

pub struct Player {
    cmd_tx: Sender<PlayerCommand>,
    state: Arc<AtomicU8>, // 0: Stopped, 1: Playing, 2: Paused
    elapsed_ms: Arc<AtomicU64>,
    duration_ms: Arc<AtomicU64>,
    volume_bits: Arc<AtomicU32>,
    queue: Arc<parking_lot::RwLock<PlaybackQueue>>,
    db: Arc<LibraryDatabase>,
    _worker: Option<JoinHandle<()>>,
}

impl Player {
    pub fn new(queue: PlaybackQueue, db: Arc<LibraryDatabase>) -> Result<Self, String> {
        let (cmd_tx, cmd_rx) = unbounded();

        let state = Arc::new(AtomicU8::new(0));
        let elapsed_ms = Arc::new(AtomicU64::new(0));
        let duration_ms = Arc::new(AtomicU64::new(0));
        let volume_bits = Arc::new(AtomicU32::new(1.0f32.to_bits()));
        let queue_lock = Arc::new(parking_lot::RwLock::new(queue));

        let s_worker = Arc::clone(&state);
        let e_worker = Arc::clone(&elapsed_ms);
        let d_worker = Arc::clone(&duration_ms);
        let v_worker = Arc::clone(&volume_bits);
        let q_worker = Arc::clone(&queue_lock);
        let db_worker = Arc::clone(&db);

        let worker = thread::spawn(move || {
            Self::run_audio_thread(
                cmd_rx, s_worker, e_worker, d_worker, v_worker, q_worker, db_worker,
            );
        });

        Ok(Self {
            cmd_tx,
            state,
            elapsed_ms,
            duration_ms,
            volume_bits,
            queue: queue_lock,
            db,
            _worker: Some(worker),
        })
    }

    pub fn snapshot(&self) -> PlayerSnapshot {
        let st_code = self.state.load(Ordering::Relaxed);
        let state = match st_code {
            1 => PlayerState::Playing,
            2 => PlayerState::Paused,
            _ => PlayerState::Stopped,
        };

        let q = self.queue.read();
        let current_track = q.current_track().and_then(|id| self.resolve_track_view(id));
        let playlist_len = q.playlist().len();
        let playlist_index = q.playlist_index();
        let up_next_count = q.up_next().len();

        let dur_val = self.duration_ms.load(Ordering::Relaxed);
        let duration = if dur_val == 0 {
            None
        } else {
            Some(Duration::from_millis(dur_val))
        };

        PlayerSnapshot {
            state,
            current_track,
            elapsed: Duration::from_millis(self.elapsed_ms.load(Ordering::Relaxed)),
            duration,
            volume: f32::from_bits(self.volume_bits.load(Ordering::Relaxed)),
            playlist_len,
            playlist_index,
            up_next_count,
        }
    }

    pub fn play(&self) {
        let _ = self.cmd_tx.send(PlayerCommand::Play);
    }

    pub fn pause(&self) {
        let _ = self.cmd_tx.send(PlayerCommand::Pause);
    }

    pub fn toggle_pause(&self) {
        let _ = self.cmd_tx.send(PlayerCommand::TogglePause);
    }

    pub fn next(&self) {
        let _ = self.cmd_tx.send(PlayerCommand::Next);
    }

    pub fn prev(&self) {
        let _ = self.cmd_tx.send(PlayerCommand::Prev);
    }

    pub fn seek(&self, time: Duration) {
        let _ = self.cmd_tx.send(PlayerCommand::Seek(time));
    }

    pub fn set_volume(&self, vol: f32) {
        let clamped = vol.clamp(0.0, 2.0);
        self.volume_bits.store(clamped.to_bits(), Ordering::Relaxed);
        let _ = self.cmd_tx.send(PlayerCommand::SetVolume(clamped));
    }

    pub fn stop(&self) {
        let _ = self.cmd_tx.send(PlayerCommand::Stop);
    }

    pub fn queue_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut PlaybackQueue) -> R,
    {
        let mut q = self.queue.write();
        f(&mut q)
    }

    fn resolve_track_view(&self, id: TrackId) -> Option<TrackView> {
        let track = self.db.get_track(id)?;
        let artist = self.db.get_artist(track.artist_id)?;
        let album = track.album_id.and_then(|a_id| self.db.get_album(a_id));
        let album_artist = track
            .album_artist_id
            .and_then(|a_id| self.db.get_artist(a_id));

        Some(TrackView {
            id: track.id,
            path: track.path,
            title: track.title,
            artist_id: track.artist_id,
            artist_name: artist.name,
            album_artist_id: track.album_artist_id,
            album_artist_name: album_artist.map(|a| a.name),
            album_id: track.album_id,
            album_title: album.map(|a| a.title),
            duration_ms: track.duration_ms,
            track_number: track.track_number,
            disc_number: track.disc_number,
            year: track.year,
            format: track.format,
            bitrate: track.bitrate,
            sample_rate: track.sample_rate,
            bit_depth: track.bit_depth,
            channels: track.channels,
            track_gain_db: track.track_gain_db,
            track_peak: track.track_peak,
            album_gain_db: track.album_gain_db,
            album_peak: track.album_peak,
        })
    }

    fn resolve_output_device() -> Option<Device> {
        let prober = HardwareProber::new();
        let host = cpal::default_host();
        let output_devices: Vec<_> = host.output_devices().ok()?.collect();

        output_devices
            .iter()
            .find(|d| {
                d.name()
                    .map(|n| n.to_lowercase() == "pipewire")
                    .unwrap_or(false)
            })
            .or_else(|| {
                output_devices
                    .iter()
                    .find(|d| d.name().map(|n| n.to_lowercase() == "pulse").unwrap_or(false))
            })
            .cloned()
            .or_else(|| prober.get_default_device())
    }

    fn run_audio_thread(
        cmd_rx: Receiver<PlayerCommand>,
        state: Arc<AtomicU8>,
        elapsed_ms: Arc<AtomicU64>,
        duration_ms: Arc<AtomicU64>,
        volume_bits: Arc<AtomicU32>,
        queue: Arc<parking_lot::RwLock<PlaybackQueue>>,
        db: Arc<LibraryDatabase>,
    ) {
        let device = match Self::resolve_output_device() {
            Some(d) => d,
            None => {
                eprintln!("Audio thread error: No audio output device found");
                return;
            }
        };

        loop {
            let next_track_path: Option<PathBuf> = {
                let q = queue.read();
                q.current_track()
                    .and_then(|id| db.get_track(id))
                    .map(|t| t.path.clone())
            };

            let track_path = match next_track_path {
                Some(p) => p,
                None => {
                    state.store(0, Ordering::Relaxed);
                    match cmd_rx.recv() {
                        Ok(PlayerCommand::Play) => continue,
                        Ok(PlayerCommand::Stop) => break,
                        Ok(_) => continue,
                        Err(_) => break,
                    }
                }
            };

            let mut decoder = match AudioDecoder::open(&track_path) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Skipping invalid track {:?}: {e}", track_path);
                    let mut q = queue.write();
                    let _ = q.next();
                    continue;
                }
            };

            let spec = decoder.spec().clone();
            if let Some(dur) = spec.duration {
                duration_ms.store(dur.as_millis() as u64, Ordering::Relaxed);
            } else {
                duration_ms.store(0, Ordering::Relaxed);
            }

            let strategy = match HardwareProber::negotiate_strategy(
                &device,
                spec.sample_rate,
                spec.channels,
            ) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Hardware negotiation error: {e}");
                    break;
                }
            };

            let (target_sample_rate, mut resampler) = match strategy {
                PlaybackStrategy::BitPerfect { sample_rate, .. } => (sample_rate, None),
                PlaybackStrategy::ResampleRequired {
                    target_sample_rate,
                    source_sample_rate,
                    channels,
                } => {
                    let r = AudioResampler::new(
                        source_sample_rate,
                        target_sample_rate,
                        channels as usize,
                    )
                    .ok();
                    (target_sample_rate, r)
                }
            };

            let stream_config = StreamConfig {
                channels: spec.channels,
                sample_rate: cpal::SampleRate(target_sample_rate),
                buffer_size: cpal::BufferSize::Default,
            };

            let buffer_frames = (target_sample_rate as usize) / 2;
            let (mut producer, consumer) =
                create_audio_ring_buffer(buffer_frames, usize::from(spec.channels));

            let sink = match CpalAudioSink::start(
                &device,
                &stream_config,
                SampleFormat::F32,
                consumer,
            ) {
                Ok(s) => Arc::new(s),
                Err(e) => {
                    eprintln!("Hardware sink launch error: {e}");
                    break;
                }
            };

            sink.set_volume(f32::from_bits(volume_bits.load(Ordering::Relaxed)));
            state.store(1, Ordering::Relaxed);

            let mut samples_played: u64 = 0;
            let mut track_finished = false;
            let mut manual_track_change = false;

            while !track_finished && !manual_track_change {
                while let Ok(cmd) = cmd_rx.try_recv() {
                    match cmd {
                        PlayerCommand::Play => {
                            sink.resume();
                            state.store(1, Ordering::Relaxed);
                        }
                        PlayerCommand::Pause => {
                            sink.pause();
                            state.store(2, Ordering::Relaxed);
                        }
                        PlayerCommand::TogglePause => {
                            if sink.is_paused() {
                                sink.resume();
                                state.store(1, Ordering::Relaxed);
                            } else {
                                sink.pause();
                                state.store(2, Ordering::Relaxed);
                            }
                        }
                        PlayerCommand::Next => {
                            let mut q = queue.write();
                            let _ = q.next();
                            manual_track_change = true;
                            break;
                        }
                        PlayerCommand::Prev => {
                            let mut q = queue.write();
                            let _ = q.previous();
                            manual_track_change = true;
                            break;
                        }
                        PlayerCommand::Seek(target_time) => {
                            if let Ok(()) = decoder.seek(target_time) {
                                samples_played = target_time.as_secs_f64() as u64
                                    * target_sample_rate as u64
                                    * spec.channels as u64;
                            }
                        }
                        PlayerCommand::SetVolume(vol) => {
                            sink.set_volume(vol);
                        }
                        PlayerCommand::Stop => {
                            state.store(0, Ordering::Relaxed);
                            return;
                        }
                    }
                }

                if manual_track_change {
                    break;
                }

                if sink.is_paused() {
                    thread::sleep(Duration::from_millis(15));
                    continue;
                }

                match decoder.next_packet() {
                    Ok(Some(samples)) => {
                        let samples_to_push = if let Some(ref mut r) = resampler {
                            let ch = spec.channels as usize;
                            let frames = samples.len() / ch;
                            let mut planar = vec![vec![0.0f32; frames]; ch];
                            for i in 0..frames {
                                for c in 0..ch {
                                    planar[c][i] = samples[i * ch + c];
                                }
                            }
                            if let Ok(resampled_planar) = r.process(&planar) {
                                let out_frames = resampled_planar[0].len();
                                let mut interleaved = Vec::with_capacity(out_frames * ch);
                                for i in 0..out_frames {
                                    for c in 0..ch {
                                        interleaved.push(resampled_planar[c][i]);
                                    }
                                }
                                interleaved
                            } else {
                                samples.to_vec()
                            }
                        } else {
                            samples.to_vec()
                        };

                        let mut offset = 0;
                        while offset < samples_to_push.len() {
                            let pushed = producer.push_slice(&samples_to_push[offset..]);
                            offset += pushed;
                            samples_played += pushed as u64;

                            let current_sec = samples_played as f64
                                / (target_sample_rate as f64 * spec.channels as f64);
                            elapsed_ms.store((current_sec * 1000.0) as u64, Ordering::Relaxed);

                            if pushed == 0 {
                                thread::sleep(Duration::from_millis(4));
                            }
                        }
                    }
                    Ok(None) => {
                        track_finished = true;
                    }
                    Err(e) => {
                        eprintln!("Stream decode error: {e}");
                        track_finished = true;
                    }
                }
            }

            if track_finished && !manual_track_change {
                thread::sleep(Duration::from_millis(250));
                let mut q = queue.write();
                if q.next().is_none() {
                    state.store(0, Ordering::Relaxed);
                    break;
                }
            }
        }
    }
}