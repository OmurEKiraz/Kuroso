use kuroso_core::library::types::*;
use kuroso_core::library::LibraryEngine;
use std::collections::HashSet;
use std::env;
use std::path::PathBuf;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target_dir = env::args()
        .nth(1)
        .unwrap_or_else(|| "crates/testacdc".to_string());
    let music_path = PathBuf::from(&target_dir);
    let cache_path = std::env::temp_dir().join("kuroso_final_drill.bin");

    let _ = std::fs::remove_file(&cache_path);

    println!("============================================================");
    println!("KUROSO CORE ENGINE - FINAL INTEGRATION AUDIT");
    println!("============================================================\n");

    // [1] Multi-Folder Engine Init & Scan
    println!("[1/5] Initializing Engine & Parallel Scanning Root...");
    let scan_start = Instant::now();
    let engine = LibraryEngine::open(&music_path, &cache_path)?;
    let scan_duration = scan_start.elapsed();

    println!("-> Initial scan completed in: {:?}", scan_duration);
    println!("-> Tracked roots:             {:?}", engine.tracked_roots());
    println!("-> Total Tracks in DB:        {}", engine.track_count());
    println!("-> Total Albums in DB:        {}", engine.album_count());
    println!("-> Total Artists in DB:       {}\n", engine.artist_count());

    // [2] Tag Extraction & Audiophile Attribute Verification
    println!("[2/5] Inspecting Tag & Audiophile Extracted Metadata...");
    let db = engine.database();
    let mut sample_count = 0;

    db.for_each_track(|track, album, artist| {
        if sample_count < 2 {
            println!("------------------------------------------------------------");
            println!("Track ID:          {:?}", track.id);
            println!("Title:             {}", track.title);
            println!("Artist:            {}", artist.name);
            println!(
                "Album Artist:      {}",
                track
                    .album_artist_id
                    .and_then(|id| db.get_artist(id))
                    .map(|a| a.name.to_string())
                    .unwrap_or_else(|| "None (Inherited)".to_string())
            );
            println!(
                "Album:             {} (Compilation: {})",
                album.map(|a| a.title.as_str()).unwrap_or("None"),
                album.map(|a| a.is_compilation).unwrap_or(false)
            );
            println!("Format:            {:?}", track.format);
            println!("Sample Rate:       {:?} Hz", track.sample_rate);
            println!("Channels:          {:?}", track.channels);
            println!("Bit Depth:         {:?}", track.bit_depth);
            println!("ReplayGain Track:  {:?} dB", track.track_gain_db);
            sample_count += 1;
        }
    });
    println!("------------------------------------------------------------\n");

    // [3] Query Engine & Scrobble Payload Verification
    println!("[3/5] Testing UI Projections, Token Search & Scrobbler...");
    let query_start = Instant::now();
    let search_results = engine.search("hells");
    let query_duration = query_start.elapsed();

    println!(
        "-> Token search 'hells' returned {} matches in {:?}",
        search_results.len(),
        query_duration
    );
    if let Some(first) = search_results.first() {
        println!(
            "   Top Match: {} - {} [{}]",
            first.artist_name,
            first.title,
            first.formatted_duration()
        );

        let scrobble = engine.queries().get_scrobble_payload(first.id);
        assert!(scrobble.is_some(), "Track over 30s must generate scrobble payload");
        let payload = scrobble.unwrap();
        println!(
            "   Scrobble Payload: \"{}\" by {} ({}s)",
            payload.track_title, payload.artist_name, payload.duration_seconds
        );
    }
    println!();

    // [4] Removable Drive Resilience & Scoped Pruning
    println!("[4/5] Simulating Removable Drive & Scoped Pruning...");
    let usb_track = PathBuf::from("/mnt/external_usb_drive/audiophile/track_01.flac");
    db.insert_track(
        usb_track.clone(),
        1700000000,
        30_000_000,
        "Thunderstruck (Hi-Res)",
        "AC/DC",
        Some("AC/DC"),
        Some("The Razors Edge"),
        292_000,
        Some(1),
        Some(1),
        Some(1990),
        AudioFormat::Flac,
        Some(96_000),
        Some(2_500_000),
        Some(24),
        Some(2),
        Some(-6.0),
        Some(0.98),
        Some(-5.5),
        Some(1.0),
    );

    let count_before = engine.track_count();
    let unmounted_usb = PathBuf::from("/mnt/external_usb_drive");
    assert!(!unmounted_usb.exists(), "Simulated USB must be unmounted");

    // Retain all current local paths as live
    let mut live_paths = HashSet::new();
    db.for_each_track(|t, _, _| {
        if t.path.starts_with(&music_path) {
            live_paths.insert(t.path.clone());
        }
    });

    let pruned = db.prune_missing_files_scoped(
        &[&music_path, &unmounted_usb],
        &live_paths,
    );

    let usb_intact = db.get_track_by_path(&usb_track).is_some();
    println!("-> Pruned files from library:           {}", pruned);
    println!(
        "-> USB Track Preserved While Unmounted: {}",
        if usb_intact { "YES" } else { "NO" }
    );
    assert_eq!(pruned, 0, "No files should be pruned when local tracks are intact and USB is unmounted");
    assert!(usb_intact, "USB track must remain safely in the database");
    println!();

    // [5] Persistence & Clean Recovery
    println!("[5/5] Testing Atomic Cache Persistence & Re-open...");
    engine.save()?;
    assert!(cache_path.exists(), "Cache file must exist on disk");

    let reopened_engine = LibraryEngine::open(&music_path, &cache_path)?;
    println!("-> Reopened DB Track Count: {}", reopened_engine.track_count());
    assert_eq!(
        reopened_engine.track_count(),
        count_before,
        "Track count must match after cache reload"
    );

    let _ = std::fs::remove_file(&cache_path);

    println!("\n============================================================");
    println!("ALL VERIFICATIONS PASSED: Kuroso Core is Production Ready!");
    println!("============================================================");

    Ok(())
}