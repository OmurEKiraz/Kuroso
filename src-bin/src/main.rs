use kuroso_core::library::database::LibraryDatabase;
use kuroso_core::library::queries::LibraryQueries;
use kuroso_core::library::scanner::scan_directory;
use kuroso_core::library::types::*;
use std::env;
use std::sync::Arc;
use std::time::Instant;

fn format_duration(ms: u32) -> String {
    let total_secs = ms / 1000;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{}:{:02}", mins, secs)
}

fn main() {
    let music_dir = env::args()
        .nth(1)
        .expect("Usage: cargo run --release -p kuroso -- <music_directory>");

    let db = Arc::new(LibraryDatabase::new());

    println!("============================================================");
    println!("KUROSO LIBRARY BENCHMARK & QUERY TEST");
    println!("Target Path: {}", music_dir);
    println!("============================================================\n");

    // 1. Filesystem Traversal + Metadata Ingestion Benchmark
    let start_scan = Instant::now();
    let scanned_count = scan_directory(&music_dir, &db);
    let scan_time = start_scan.elapsed();

    println!("[1] INGESTION METRICS");
    println!("  Total Files Ingested : {}", scanned_count);
    println!("  Tracks Stored (RAM)  : {}", db.track_count());
    println!("  Albums Stored (RAM)  : {}", db.album_count());
    println!("  Artists Stored (RAM) : {}", db.artist_count());
    println!("  Ingestion Duration   : {:?}", scan_time);
    if scanned_count > 0 {
        let per_file = scan_time / scanned_count as u32;
        println!("  Average Cost / Track : {:?}", per_file);
    }
    println!();

    let queries = LibraryQueries::new(&db);

    // 2. Relational Hierarchy Query (Artist -> Albums -> Tracks)
    println!("[2] RELATIONAL HIERARCHY TEST");
    let start_relational = Instant::now();
    let artist_data = queries.get_artist_with_albums(ArtistId(1));
    let relational_time = start_relational.elapsed();

    match artist_data {
        Some(data) => {
            println!("  Artist Name : {}", data.artist.name);
            println!("  Total Albums: {}", data.albums.len());
            println!("  Query Latency: {:?}", relational_time);
            println!();

            // Print the first 2 albums with their sorted tracks
            for album_data in data.albums.iter().take(2) {
                println!("  Album: \"{}\" ({} tracks, Year: {:?})", 
                    album_data.album.title, 
                    album_data.tracks.len(),
                    album_data.album.year
                );
                for track in album_data.tracks.iter().take(4) {
                    println!(
                        "    [{:02}] {:<30} | {} | {:?} | {} Hz",
                        track.track_number.unwrap_or(0),
                        track.title,
                        format_duration(track.duration_ms),
                        track.format,
                        track.sample_rate.unwrap_or(0)
                    );
                }
                if album_data.tracks.len() > 4 {
                    println!("    ... and {} more tracks", album_data.tracks.len() - 4);
                }
                println!();
            }
        }
        None => println!("  No artist found with ArtistId(1)"),
    }

    // 3. Search Queries Benchmark
    println!("[3] SEARCH BENCHMARKS");
    let search_terms = ["Rock", "Highway", "Black", "Live", "a"];

    for term in search_terms {
        let start_search = Instant::now();
        let results = queries.search_tracks(term);
        let search_time = start_search.elapsed();

        println!(
            "  Query \"{:<8}\" -> {:>3} matches | Latency: {:?}",
            term,
            results.len(),
            search_time
        );
    }
    println!();

    // 4. Random Access Microbenchmark
    println!("[4] IN-MEMORY RANDOM ACCESS BENCHMARK");
    let iterations = 100_000;
    let track_count = db.track_count() as u32;

    if track_count > 0 {
        let start_random = Instant::now();
        let mut sample_checksum: u64 = 0;

        // Pseudo-random linear congruential generator for zero-overhead iteration
        let mut state: u32 = 1337;
        for _ in 0..iterations {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let target_id = (state % track_count) + 1;
            
            if let Some(track) = db.get_track(TrackId(target_id)) {
                sample_checksum = sample_checksum.wrapping_add(track.duration_ms as u64);
            }
        }
        let total_lookup_time = start_random.elapsed();
        let per_lookup = total_lookup_time / iterations as u32;

        println!("  Lookups Executed     : {} queries", iterations);
        println!("  Total Lookup Time    : {:?}", total_lookup_time);
        println!("  Latency Per Lookup   : {:?}", per_lookup);
        println!("  Throughput           : {:.2} million queries/sec", 
            (iterations as f64 / total_lookup_time.as_secs_f64()) / 1_000_000.0
        );
        println!("  Checksum Verification: {}", sample_checksum);
    }
    println!("============================================================");
}