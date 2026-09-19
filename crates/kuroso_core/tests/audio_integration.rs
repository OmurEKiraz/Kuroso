use kuroso_core::audio::decoder::AudioDecoder;
use kuroso_core::audio::dsp::{AudioResampler, GainProcessor};
use kuroso_core::audio::hardware::PlaybackStrategy;
use kuroso_core::audio::player::{Player, PlayerState};
use kuroso_core::audio::queue::PlaybackQueue;
use kuroso_core::audio::sink::create_audio_ring_buffer;
use kuroso_core::audio::state::{RepeatMode, ShuffleMode};
use kuroso_core::library::database::LibraryDatabase;
use kuroso_core::library::types::{AudioFormat, TrackId};
use ringbuf::traits::{Consumer, Producer};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Robust multi-context path resolver for integration tests run either from
/// the crate root or workspace root.
fn find_test_file(rel: &str) -> Option<PathBuf> {
    let direct = PathBuf::from(rel);
    if direct.exists() {
        return Some(direct);
    }
    let from_workspace = PathBuf::from("../../").join(rel);
    if from_workspace.exists() {
        return Some(from_workspace);
    }
    let from_crate = PathBuf::from("../").join(rel);
    if from_crate.exists() {
        return Some(from_crate);
    }
    None
}

/// Helper: Builds an in-memory database with populated audio specs.
fn create_populated_audio_db() -> (Arc<LibraryDatabase>, [TrackId; 4]) {
    let db = LibraryDatabase::new();

    let p1 = PathBuf::from("/music/camellia/crystallized/01_crystallized.opus");
    let p2 = PathBuf::from("/music/camellia/crystallized/02_first_town.opus");
    let p3 = PathBuf::from("/music/camellia/singles/spin_eternally.opus");
    let p4 = PathBuf::from("/music/taku_inoue/aliens.flac");

    let t1 = db.insert_track(
        p1, 1700000000, 15_000_000, "Crystallized", "Camellia", None,
        Some("Crystallized"), 284_000, Some(1), Some(1), Some(2015),
        AudioFormat::Opus, Some(48_000), Some(160_000), Some(16), Some(2),
        Some(-6.0), Some(0.95), Some(-5.5), Some(0.98),
    );

    let t2 = db.insert_track(
        p2, 1700000001, 12_000_000, "First Town", "Camellia", None,
        Some("Crystallized"), 210_000, Some(2), Some(1), Some(2015),
        AudioFormat::Opus, Some(48_000), Some(160_000), Some(16), Some(2),
        Some(-6.2), Some(0.94), Some(-5.5), Some(0.98),
    );

    let t3 = db.insert_track(
        p3, 1700000002, 18_000_000, "Spin Eternally", "Camellia", None,
        None, 290_000, None, None, Some(2021),
        AudioFormat::Opus, Some(48_000), Some(192_000), Some(16), Some(2),
        Some(-7.0), Some(0.99), None, None,
    );

    let t4 = db.insert_track(
        p4, 1700000003, 35_000_000, "Aliens", "Taku Inoue", None,
        None, 245_000, None, None, Some(2022),
        AudioFormat::Flac, Some(96_000), Some(900_000), Some(24), Some(2),
        Some(-5.0), Some(0.92), None, None,
    );

    (Arc::new(db), [t1, t2, t3, t4])
}

// ============================================================================
// 1. PHYSICAL FILE DECODER & OPUS ENGINE TESTS
// ============================================================================

#[test]
fn test_real_acdc_opus_file_decoding() {
    let opus_path = match find_test_file("crates/testacdc/Back In Black/Back In Black.opus") {
        Some(p) => p,
        None => {
            eprintln!("Skipping real file test: Back In Black.opus not found");
            return;
        }
    };

    let mut decoder = AudioDecoder::open(&opus_path).expect("Failed to initialize decoder on physical Opus file");
    let spec = decoder.spec().clone();

    assert_eq!(spec.sample_rate, 48_000, "Native Opus sample rate must be 48kHz");
    assert_eq!(spec.channels, 2, "Back In Black must be stereo");
    assert!(spec.duration.is_some(), "Duration must be resolved from container header");

    let mut packets_read = 0;
    let mut total_samples = 0;

    while packets_read < 50 {
        match decoder.next_packet() {
            Ok(Some(samples)) => {
                assert!(!samples.is_empty(), "Decoded packet delivered 0 samples");
                assert_eq!(samples.len() % 2, 0, "Stereo packets must have even sample counts");
                for &sample in samples {
                    assert!(sample >= -1.05 && sample <= 1.05, "PCM sample exceeded normalized bounds: {}", sample);
                }
                total_samples += samples.len();
                packets_read += 1;
            }
            Ok(None) => break,
            Err(e) => panic!("Decoder error while decoding real Opus file: {e}"),
        }
    }

    assert!(packets_read >= 50, "Failed to decode minimum frame threshold");
    assert!(total_samples > 0);
}

#[test]
fn test_real_file_accurate_seeking() {
    let opus_path = match find_test_file("crates/testacdc/Back In Black/Hells Bells.opus") {
        Some(p) => p,
        None => {
            eprintln!("Skipping seek test: Hells Bells.opus not found");
            return;
        }
    };

    let mut decoder = AudioDecoder::open(&opus_path).expect("Failed to open audio file");
    let seek_target = Duration::from_secs(30);

    decoder.seek(seek_target).expect("Accurate seek failed");
    let next_chunk = decoder.next_packet().expect("Post-seek decode error");
    assert!(next_chunk.is_some(), "Seek yielded empty packet");
}

#[test]
fn test_decoder_invalid_path_fails_gracefully() {
    let invalid_path = Path::new("crates/testacdc/NonExistentTrack.opus");
    let result = AudioDecoder::open(invalid_path);
    assert!(result.is_err(), "Non-existent path must return Err");
}

// ============================================================================
// 2. SPSC LOCK-FREE RING BUFFER TESTS
// ============================================================================

#[test]
fn test_ring_buffer_boundaries_and_underrun() {
    let frames = 512;
    let channels = 2;
    let (mut producer, mut consumer) = create_audio_ring_buffer(frames, channels);

    let mut sink_read_buffer = vec![0.0f32; 128];
    let popped = consumer.pop_slice(&mut sink_read_buffer);
    assert_eq!(popped, 0, "Empty buffer must yield 0 read samples");

    let pcm_in = vec![0.5f32; 256];
    let pushed = producer.push_slice(&pcm_in);
    assert_eq!(pushed, 256, "Must push all available samples under capacity");

    let read_1 = consumer.pop_slice(&mut sink_read_buffer);
    assert_eq!(read_1, 128);
    assert!(sink_read_buffer.iter().all(|&s| (s - 0.5).abs() < 1e-6));

    let read_2 = consumer.pop_slice(&mut sink_read_buffer);
    assert_eq!(read_2, 128);

    let read_dry = consumer.pop_slice(&mut sink_read_buffer);
    assert_eq!(read_dry, 0, "Dry buffer must not generate phantom samples");
}

#[test]
fn test_ring_buffer_multithreaded_stress_streaming() {
    let frames = 1024;
    let channels = 2;
    let total_samples = 48_000 * 2; // 1 second of stereo audio
    let (mut producer, mut consumer) = create_audio_ring_buffer(frames, channels);

    let writer_done = Arc::new(AtomicBool::new(false));
    let writer_done_clone = Arc::clone(&writer_done);

    let producer_thread = thread::spawn(move || {
        let mut generated = 0usize;
        while generated < total_samples {
            let chunk_size = 256.min(total_samples - generated);
            let chunk: Vec<f32> = (0..chunk_size).map(|i| (generated + i) as f32).collect();
            let pushed = producer.push_slice(&chunk);
            generated += pushed;
            if pushed == 0 {
                thread::yield_now();
            }
        }
        writer_done_clone.store(true, Ordering::Release);
    });

    let consumer_thread = thread::spawn(move || {
        let mut received = Vec::with_capacity(total_samples);
        let mut read_scratch = [0.0f32; 128];

        while received.len() < total_samples {
            let read = consumer.pop_slice(&mut read_scratch);
            if read > 0 {
                received.extend_from_slice(&read_scratch[..read]);
            } else if writer_done.load(Ordering::Acquire) {
                let rem = consumer.pop_slice(&mut read_scratch);
                if rem > 0 {
                    received.extend_from_slice(&read_scratch[..rem]);
                } else {
                    break;
                }
            } else {
                thread::yield_now();
            }
        }
        received
    });

    producer_thread.join().expect("Producer crashed");
    let received_data = consumer_thread.join().expect("Consumer crashed");

    assert_eq!(received_data.len(), total_samples);
    for (idx, &val) in received_data.iter().enumerate() {
        assert_eq!(val, idx as f32, "Bit-perfect sample stream order corruption at {}", idx);
    }
}

// ============================================================================
// 3. DSP & RESAMPLING TESTS
// ============================================================================

#[test]
fn test_gain_processor_attenuation_and_clipping() {
    let mut buffer = vec![0.5f32, -0.5f32, 1.0f32, -1.0f32];

    let unity_gain = GainProcessor::new(0.0);
    unity_gain.process_interleaved(&mut buffer);
    assert_eq!(buffer, vec![0.5f32, -0.5f32, 1.0f32, -1.0f32]);

    let half_gain = GainProcessor::new(-6.0205999);
    half_gain.process_interleaved(&mut buffer);
    assert!((buffer[0] - 0.25).abs() < 1e-3);
    assert!((buffer[1] - (-0.25)).abs() < 1e-3);

    let mut hot_signal = vec![0.8f32, -0.9f32];
    let boost_gain = GainProcessor::new(20.0);
    boost_gain.process_interleaved(&mut hot_signal);
    assert_eq!(hot_signal[0], 1.0f32);
    assert_eq!(hot_signal[1], -1.0f32);
}

#[test]
fn test_audio_resampler_rate_conversion_ratios() {
    let in_rate = 44_100;
    let out_rate = 48_000;
    let channels = 2;
    let mut resampler = AudioResampler::new(in_rate, out_rate, channels)
        .expect("Failed to initialize 44.1k -> 48k Rubato sinc resampler");

    let in_frames = 1024;
    let chunks = 4;
    let mut total_out_frames = 0;

    for chunk_idx in 0..chunks {
        let mut left_in = vec![0.0f32; in_frames];
        let right_in = vec![0.0f32; in_frames];

        for i in 0..in_frames {
            let total_sample_idx = chunk_idx * in_frames + i;
            let t = total_sample_idx as f32 / in_rate as f32;
            left_in[i] = (2.0 * std::f32::consts::PI * 440.0 * t).sin();
        }

        let input_planar = vec![left_in, right_in];
        let resampled = resampler
            .process(&input_planar)
            .expect("Resampling transformation failed");

        assert_eq!(resampled.len(), 2, "Channel count must be preserved");
        total_out_frames += resampled[0].len();

        assert!(resampled[1].iter().all(|&s| s.abs() < 1e-6), "Crosstalk detected on silent channel");
    }

    let total_in_frames = in_frames * chunks;
    let expected_total_frames = (total_in_frames as f64 * (out_rate as f64 / in_rate as f64)).round() as usize;
    let delta = (total_out_frames as isize - expected_total_frames as isize).abs();
    assert!(delta <= 150, "Resampler output frames out of tolerance: got {}, expected {}", total_out_frames, expected_total_frames);
}

#[test]
fn test_hardware_strategy_negotiator() {
    let direct_strategy = PlaybackStrategy::BitPerfect {
        sample_rate: 48_000,
        channels: 2,
    };
    match direct_strategy {
        PlaybackStrategy::BitPerfect { sample_rate, channels } => {
            assert_eq!(sample_rate, 48_000);
            assert_eq!(channels, 2);
        }
        _ => panic!("Expected BitPerfect strategy"),
    }

    let resample_strategy = PlaybackStrategy::ResampleRequired {
        source_sample_rate: 96_000,
        target_sample_rate: 48_000,
        channels: 2,
    };
    match resample_strategy {
        PlaybackStrategy::ResampleRequired { source_sample_rate, target_sample_rate, channels } => {
            assert_eq!(source_sample_rate, 96_000);
            assert_eq!(target_sample_rate, 48_000);
            assert_eq!(channels, 2);
        }
        _ => panic!("Expected ResampleRequired strategy"),
    }
}

// ============================================================================
// 4. PLAYBACK QUEUE CORE ARCHITECTURE TESTS
// ============================================================================

#[test]
fn test_queue_linear_step_and_boundaries() {
    let mut q = PlaybackQueue::new(10);
    let tracks = vec![TrackId(101), TrackId(102), TrackId(103)];

    q.load_tracks(tracks, Some(0));
    assert_eq!(q.current_track(), Some(TrackId(101)));
    assert_eq!(q.playlist_index(), Some(0));

    assert_eq!(q.next(), Some(TrackId(102)));
    assert_eq!(q.playlist_index(), Some(1));
    assert_eq!(q.next(), Some(TrackId(103)));
    assert_eq!(q.playlist_index(), Some(2));

    assert_eq!(q.next(), None);
    assert_eq!(q.current_track(), None);
    assert_eq!(q.playlist_index(), None);
}

#[test]
fn test_queue_repeat_modes() {
    let mut q = PlaybackQueue::new(10);
    q.load_tracks(vec![TrackId(1), TrackId(2)], Some(0));

    // 1. RepeatMode::Track
    q.set_repeat(RepeatMode::Track);
    assert_eq!(q.current_track(), Some(TrackId(1)));
    assert_eq!(q.next(), Some(TrackId(1)), "Repeat Track must loop current ID indefinitely");
    assert_eq!(q.next(), Some(TrackId(1)));

    // 2. RepeatMode::Queue
    q.set_repeat(RepeatMode::Queue);
    assert_eq!(q.next(), Some(TrackId(2)));
    assert_eq!(q.next(), Some(TrackId(1)));
    assert_eq!(q.playlist_index(), Some(0));
}

#[test]
fn test_queue_up_next_interleaving_and_reordering() {
    let mut q = PlaybackQueue::new(10);
    q.load_tracks(vec![TrackId(1), TrackId(2), TrackId(3)], Some(0));

    q.append_to_queue(TrackId(99));
    q.play_next(TrackId(88));

    assert_eq!(q.up_next().len(), 2);
    assert_eq!(q.up_next()[0], TrackId(88));
    assert_eq!(q.up_next()[1], TrackId(99));

    assert_eq!(q.next(), Some(TrackId(88)));
    assert_eq!(q.next(), Some(TrackId(99)));

    assert_eq!(q.next(), Some(TrackId(2)));
    assert_eq!(q.next(), Some(TrackId(3)));
}

#[test]
fn test_queue_history_navigation_and_truncation() {
    let mut q = PlaybackQueue::new(2);
    q.load_tracks(vec![TrackId(1), TrackId(2), TrackId(3), TrackId(4)], Some(0));

    q.next(); // current: 2, history: [1]
    q.next(); // current: 3, history: [1, 2]
    q.next(); // current: 4, history: [2, 3] (1 dropped due to limit = 2)

    assert_eq!(q.history().len(), 2);
    assert_eq!(q.history()[0], TrackId(2));
    assert_eq!(q.history()[1], TrackId(3));

    let prev = q.previous();
    assert_eq!(prev, Some(TrackId(3)));
    assert_eq!(q.current_track(), Some(TrackId(3)));

    let prev2 = q.previous();
    assert_eq!(prev2, Some(TrackId(2)));
    assert_eq!(q.current_track(), Some(TrackId(2)));
}

#[test]
fn test_queue_shuffle_uniformity_and_jump() {
    let mut q = PlaybackQueue::new(10);
    let ids: Vec<TrackId> = (1..=20).map(TrackId).collect();

    // With RepeatMode::Off and start_index = None, shuffle_cursor begins cleanly
    // at index 0 without mid-stream reshuffle loops.
    q.set_repeat(RepeatMode::Off);
    q.set_shuffle(ShuffleMode::Tracks);
    q.load_tracks(ids, None);

    let mut visited = HashSet::new();

    // Traverse all items in the permutation
    while let Some(t) = q.next() {
        assert!(visited.insert(t), "Shuffle produced duplicate track within single cycle: {:?}", t);
    }

    assert_eq!(visited.len(), 20, "Shuffle must visit all 20 unique tracks exactly once");

    let jumped = q.jump_to_playlist_index(5);
    assert_eq!(jumped, Some(TrackId(6)));
    assert_eq!(q.current_track(), Some(TrackId(6)));
}

#[test]
fn test_queue_database_sanitization() {
    let (db, [t1, t2, t3, _]) = create_populated_audio_db();
    let mut q = PlaybackQueue::new(10);
    q.load_tracks(vec![t1, t2, t3], Some(0));

    let p2 = PathBuf::from("/music/camellia/crystallized/02_first_town.opus");
    db.remove_track_by_path(&p2);

    q.sanitize(&db);

    assert_eq!(q.playlist().len(), 2);
    assert_eq!(q.playlist()[0], t1);
    assert_eq!(q.playlist()[1], t3);
    assert_eq!(q.current_track(), Some(t1));
}

// ============================================================================
// 5. PLAYER CONCURRENCY & ATOMIC EXPORTS
// ============================================================================

#[test]
fn test_ieee754_volume_atomic_roundtrip() {
    let vol_atomic = AtomicU32::new(1.0f32.to_bits());

    let test_volumes = [0.0f32, 0.25f32, 0.5f32, 0.85f32, 1.0f32, 1.5f32, 2.0f32];
    for &expected_vol in &test_volumes {
        vol_atomic.store(expected_vol.to_bits(), Ordering::Relaxed);
        let actual = f32::from_bits(vol_atomic.load(Ordering::Relaxed));
        assert_eq!(actual, expected_vol, "Float-to-u32 atomic bitcast failed");
    }
}

#[test]
fn test_player_instantiation_and_snapshot_reporting() {
    let (db, [t1, t2, _, _]) = create_populated_audio_db();
    let mut queue = PlaybackQueue::default();
    queue.load_tracks(vec![t1, t2], Some(0));

    let player = Player::new(queue, Arc::clone(&db))
        .expect("Player actor thread failed to launch");

    let snap = player.snapshot();
    assert_eq!(snap.state, PlayerState::Stopped);
    assert_eq!(snap.volume, 1.0);
    assert_eq!(snap.playlist_len, 2);
    assert_eq!(snap.playlist_index, Some(0));

    let current = snap.current_track.expect("Current track must be resolved");
    assert_eq!(current.title.as_str(), "Crystallized");
    assert_eq!(current.artist_name.as_str(), "Camellia");
    assert_eq!(current.sample_rate, Some(48_000));
    assert_eq!(current.channels, Some(2));

    player.set_volume(0.65);
    let updated_snap = player.snapshot();
    assert!((updated_snap.volume - 0.65).abs() < 1e-6);

    player.stop();
}

#[test]
fn test_player_queue_mutation_via_closure() {
    let (db, [t1, t2, t3, _]) = create_populated_audio_db();
    let mut queue = PlaybackQueue::default();
    queue.load_tracks(vec![t1, t2], Some(0));

    let player = Player::new(queue, Arc::clone(&db)).unwrap();

    player.queue_mut(|q| {
        q.append_to_queue(t3);
    });

    let snap = player.snapshot();
    assert_eq!(snap.up_next_count, 1);

    player.stop();
}