use kuroso_core::library::database::LibraryDatabase;
use kuroso_core::library::scanner::scan_directory;
use kuroso_core::library::queries::LibraryQueries;
use std::env;
use std::path::PathBuf;

fn main() {
    let target_dir = env::args()
        .nth(1)
        .unwrap_or_else(|| "crates/testacdc".to_string());

    println!("============================================================");
    println!("KUROSO AUDIOPHILE & TAG ENRICHMENT DRILL");
    println!("============================================================\n");

    let db = LibraryDatabase::new();

    println!("[1/2] Scanning directory: {}", target_dir);
    let report = scan_directory(&PathBuf::from(&target_dir), &db);
    println!(
        "Scan complete: {} files scanned, {} newly added, {} pruned\n",
        report.scanned_files, report.newly_added, report.pruned
    );

    println!("[2/2] Validating Extracted Wave 1 Tags:");
    println!("------------------------------------------------------------");

    let queries = LibraryQueries::new(&db);
    let tracks = queries.search(""); // returns empty, let's fetch by artist or query all
    
    // Iterate every track in the DB to inspect fields directly
    db.for_each_track(|track, album, artist| {
        println!("Title:             {}", track.title);
        println!("Track Artist:      {}", artist.name);
        println!(
            "Album Artist:      {}",
            track
                .album_artist_id
                .and_then(|id| db.get_artist(id))
                .map(|a| a.name.to_string())
                .unwrap_or_else(|| "None (Defaults to Track Artist)".to_string())
        );
        println!(
            "Album:             {}",
            album.map(|a| a.title.as_str()).unwrap_or("None")
        );
        println!(
            "Is Compilation:    {}",
            album.map(|a| a.is_compilation).unwrap_or(false)
        );
        println!("Format:            {:?}", track.format);
        println!("Sample Rate:       {:?} Hz", track.sample_rate);
        println!("Bitrate:           {:?} bps", track.bitrate);
        println!("Bit Depth:         {:?} bits", track.bit_depth);
        println!("Channels:          {:?}", track.channels);
        println!("Track Gain:        {:?} dB", track.track_gain_db);
        println!("Track Peak:        {:?}", track.track_peak);
        println!("Album Gain:        {:?} dB", track.album_gain_db);
        println!("Album Peak:        {:?}", track.album_peak);
        println!("------------------------------------------------------------");
    });

    println!("\nTotal Tracks In DB:  {}", db.track_count());
    println!("Total Albums In DB:  {}", db.album_count());
    println!("Total Artists In DB: {}", db.artist_count());
}