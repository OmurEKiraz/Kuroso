use kuroso_core::library::database::LibraryDatabase;
use kuroso_core::library::queries::LibraryQueries;
use kuroso_core::library::scanner::scan_directory;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let music_dir = env::args()
        .nth(1)
        .expect("Usage: cargo run --release -p kuroso -- <music_directory>");

    let cache_file = PathBuf::from("/tmp/kuroso_library.bin");

    // 1. Cold Scan or Instant Binary Load
    let db = if cache_file.exists() {
        println!("Found binary cache at {:?}, loading...", cache_file);
        let start = Instant::now();
        let loaded = LibraryDatabase::load_from_file(&cache_file).expect("Failed to load cache");
        println!("Loaded binary cache in {:?}", start.elapsed());
        Arc::new(loaded)
    } else {
        Arc::new(LibraryDatabase::new())
    };

    println!("\nRunning Incremental Diff Sync against: {}", music_dir);
    let start_scan = Instant::now();
    let report = scan_directory(&music_dir, &db);
    let scan_duration = start_scan.elapsed();

    println!("--------------------------------------------------");
    println!("Scan Time        : {:?}", scan_duration);
    println!("Files On Disk    : {}", report.scanned_files);
    println!("New Ingested     : {}", report.newly_added);
    println!("Updated (mtime)  : {}", report.updated);
    println!("Pruned Vanished  : {}", report.pruned);
    println!(
        "Total In RAM     : {} tracks, {} albums, {} artists",
        db.track_count(),
        db.album_count(),
        db.artist_count()
    );
    println!("--------------------------------------------------");

    // 2. Binary Serialization
    let start_save = Instant::now();
    db.save_to_file(&cache_file).expect("Failed to save cache");
    println!("Serialized database to disk in {:?}\n", start_save.elapsed());

    // 3. Multi-Field Search Benchmark & Track Number Verification
    let queries = LibraryQueries::new(&db);
    let search_term = "rock";
    let start_search = Instant::now();
    let results = queries.search(search_term);
    let search_time = start_search.elapsed();

    println!("Multi-field search for \"{}\":", search_term);
    println!("  Found {} matches in {:?}", results.len(), search_time);
    for track in results.iter().take(5) {
        println!(
            "    -> Track #{:02}: {} (Album: {:?})",
            track.track_number.unwrap_or(0),
            track.title,
            track.album_id
        );
    }
}