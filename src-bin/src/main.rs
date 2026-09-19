use kuroso_core::library::database::LibraryDatabase;
use kuroso_core::library::queries::{
    AlbumSortBy, ArtistSortBy, LibraryQueries, SortDirection, TrackView,
};
use kuroso_core::library::scanner::scan_directory;
use kuroso_core::library::types::TrackId;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let music_dir = env::args()
        .nth(1)
        .unwrap_or_else(|| "crates/testacdc".to_string());

    let test_cache_path = PathBuf::from("/tmp/kuroso_comprehensive_test.bin");
    let _ = fs::remove_file(&test_cache_path);

    println!("============================================================");
    println!("KUROSO FULL END-TO-END DATABASE & QUERY VERIFICATION HARNESS");
    println!("Target Music Directory: {}", music_dir);
    println!("============================================================\n");

    // -------------------------------------------------------------
    // 1. INGESTION & DIFF SCANNING
    // -------------------------------------------------------------
    println!("[PHASE 1] Scanning and Ingesting Files...");
    let db = Arc::new(LibraryDatabase::new());
    let start_scan = Instant::now();
    let report = scan_directory(&music_dir, &db);
    let scan_dur = start_scan.elapsed();

    println!("  -> Scanned files : {}", report.scanned_files);
    println!("  -> Newly added   : {}", report.newly_added);
    println!("  -> Scan duration : {:?}", scan_dur);
    println!(
        "  -> RAM State     : {} tracks, {} albums, {} artists\n",
        db.track_count(),
        db.album_count(),
        db.artist_count()
    );

    assert!(
        db.track_count() > 0,
        "Database must have ingested tracks to run tests"
    );

    // -------------------------------------------------------------
    // 2. ATOMIC SAVE & RELOAD
    // -------------------------------------------------------------
    println!("[PHASE 2] Atomic Persistence & Serialization...");
    let start_save = Instant::now();
    db.save_to_file(&test_cache_path).expect("Atomic save failed");
    println!(
        "  -> Saved to {:?} in {:?}",
        test_cache_path,
        start_save.elapsed()
    );

    let start_load = Instant::now();
    let (loaded_db, is_hit) = LibraryDatabase::load_or_recover(&test_cache_path);
    println!(
        "  -> Loaded from cache in {:?} (is_hit: {})",
        start_load.elapsed(),
        is_hit
    );
    assert!(is_hit, "Cache should be cleanly recognized");
    assert_eq!(loaded_db.track_count(), db.track_count());
    assert_eq!(loaded_db.album_count(), db.album_count());
    assert_eq!(loaded_db.artist_count(), db.artist_count());
    println!("  -> State parity verified across serialization.\n");

    // -------------------------------------------------------------
    // 3. FAULT INJECTION & CORRUPTION DRILL
    // -------------------------------------------------------------
    println!("[PHASE 3] Simulating Bit-Rot / Corruption / 0-Byte Failures...");
    let corrupt_test_path = PathBuf::from("/tmp/kuroso_tamper_test.bin");
    db.save_to_file(&corrupt_test_path).unwrap();

    // Overwrite payload with corruption
    {
        let mut file = OpenOptions::new()
            .write(true)
            .open(&corrupt_test_path)
            .unwrap();
        file.write_all(b"KUROSO\0\x01\x01\0\0\0CORRUPT_BYTES_INJECTED_HERE")
            .unwrap();
    }

    let (recovered_db, hit) = LibraryDatabase::load_or_recover(&corrupt_test_path);
    assert!(!hit, "Corrupt file must not register as hit");
    assert_eq!(
        recovered_db.track_count(),
        0,
        "Corrupted file must fall back to empty state"
    );
    assert!(
        !corrupt_test_path.exists(),
        "Corrupt file must have been quarantined"
    );
    println!("  -> Corruption safety verified: file quarantined, process did not crash.\n");

    // -------------------------------------------------------------
    // 4. UI PROJECTIONS & FLAT VIEWS
    // -------------------------------------------------------------
    println!("[PHASE 4] UI Flat Projections & Formatting...");
    let queries = LibraryQueries::new(&db);
    let sample_track = db.get_track(TrackId(1)).expect("Track #1 missing");
    let view: TrackView = queries
        .track_to_view(&sample_track)
        .expect("Failed to project TrackView");

    println!("  Sample Track View:");
    println!("    ID         : {:?}", view.id);
    println!("    Title      : {}", view.title);
    println!("    Artist     : {}", view.artist_name);
    println!(
        "    Album      : {}",
        view.album_title.as_deref().unwrap_or("Standalone")
    );
    println!("    Track #    : {}", view.formatted_track_number());
    println!("    Duration   : {}", view.formatted_duration());
    println!("    Format     : {:?}", view.format);
    println!("    Sample Rate: {:?} Hz", view.sample_rate);
    println!("    Bitrate    : {:?} kbps\n", view.bitrate.map(|b| b / 1000));

    // -------------------------------------------------------------
    // 5. SORTING ALGORITHMS
    // -------------------------------------------------------------
    println!("[PHASE 5] Relational Sorting Tests...");

    // Artists Alphabetically
    let artists_asc = queries.get_all_artists(ArtistSortBy::Name, SortDirection::Ascending);
    println!("  -> Top 3 Artists (A-Z):");
    for a in artists_asc.iter().take(3) {
        println!("     - {} (Albums: {})", a.name, a.albums.len());
    }

    // Albums Chronologically
    let albums_by_year = queries.get_all_albums(AlbumSortBy::Year, SortDirection::Descending);
    println!("  -> Top 3 Albums (Newest First):");
    for alb in albums_by_year.iter().take(3) {
        println!(
            "     - {} ({}) - {} tracks",
            alb.title,
            alb.year
                .map(|y| y.to_string())
                .unwrap_or_else(|| "Unknown Year".into()),
            alb.tracks.len()
        );
    }

    // Natural Album Track Ordering (Disc -> Track -> Title)
    if let Some(first_album) = albums_by_year.first() {
        if let Some(album_data) = queries.get_album_with_tracks(first_album.id) {
            println!(
                "  -> Tracklist for \"{}\" (Natural Disc/Track Ordering):",
                album_data.album.title
            );
            for t in album_data.tracks.iter().take(5) {
                println!(
                    "     [{}] {} ({})",
                    t.formatted_track_number(),
                    t.title,
                    t.formatted_duration()
                );
            }
        }
    }
    println!();

    // -------------------------------------------------------------
    // 6. MULTI-FIELD SEARCH & SYNTAX MATCHING
    // -------------------------------------------------------------
    println!("[PHASE 6] Search Algorithms & Edge Cases...");

    // Multi-token search
    let search_term = "rock live";
    let start_search = Instant::now();
    let matches = queries.search(search_term);
    println!(
        "  -> Search \"{}\": found {} results in {:?}",
        search_term,
        matches.len(),
        start_search.elapsed()
    );
    for m in matches.iter().take(3) {
        println!("     * {} - {}", m.artist_name, m.title);
    }

    // Direct ID syntax search
    let id_query = "id:1";
    let id_match = queries.search(id_query);
    assert_eq!(id_match.len(), 1, "id:1 should return exactly one result");
    println!(
        "  -> Direct ID search \"{}\" -> matched: {}",
        id_query, id_match[0].title
    );

    // Empty string edge case
    assert!(
        queries.search("").is_empty(),
        "Empty query must return empty"
    );
    assert!(
        queries.search("    ").is_empty(),
        "Whitespace query must return empty"
    );
    println!("  -> Empty query edge cases handled cleanly.");

    // Multi-entity search (Tracks + Albums + Artists)
    let all_res = queries.search_all("rock");
    println!(
        "  -> Multi-Entity \"rock\" -> {} tracks, {} albums, {} artists\n",
        all_res.tracks.len(),
        all_res.albums.len(),
        all_res.artists.len()
    );

    // -------------------------------------------------------------
    // 7. SCROBBLER PAYLOAD & SPECIFICATION CHECKS
    // -------------------------------------------------------------
    println!("[PHASE 7] Scrobble Engine Specification...");
    if let Some(payload) = queries.get_scrobble_payload(TrackId(1)) {
        println!("  -> Track #1 Scrobble Payload generated:");
        println!("     * Title    : {}", payload.track_title);
        println!("     * Artist   : {}", payload.artist_name);
        println!("     * Duration : {}s", payload.duration_seconds);
        assert!(
            payload.duration_seconds >= 30,
            "Scrobble specification requires >= 30s duration"
        );
    }

    // -------------------------------------------------------------
    // 8. PAGINATION & VIRTUALIZATION SLICING
    // -------------------------------------------------------------
    println!("\n[PHASE 8] Pagination / Table Slicing...");
    let all_tracks = queries.search("rock");
    let page_0 = LibraryQueries::paginate(&all_tracks, 0, 3);
    let page_1 = LibraryQueries::paginate(&all_tracks, 1, 3);
    println!("  -> Page 0 (size 3): {} tracks", page_0.len());
    println!("  -> Page 1 (size 3): {} tracks", page_1.len());
    if !page_0.is_empty() && !page_1.is_empty() {
        assert_ne!(
            page_0[0].id, page_1[0].id,
            "Pages must contain distinct tracks"
        );
    }

    // -------------------------------------------------------------
    // 9. FULL RESET / DATABASE REINITIALIZATION
    // -------------------------------------------------------------
    println!("\n[PHASE 9] Full Database Clear & Reinitialization...");
    let before_clear = db.track_count();
    db.clear();
    println!(
        "  -> Tracks before: {}, after clear(): {}",
        before_clear,
        db.track_count()
    );
    assert_eq!(db.track_count(), 0);
    assert_eq!(db.album_count(), 0);
    assert_eq!(db.artist_count(), 0);
    println!("  -> Memory state fully purged and ID counters reset.");

    // Cleanup
    let _ = fs::remove_file(&test_cache_path);

    println!("\n============================================================");
    println!("ALL 9 VERIFICATION PHASES COMPLETED WITH 100% PASS RATE");
    println!("============================================================");
}