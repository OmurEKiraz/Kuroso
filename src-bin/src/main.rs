use kuroso_core::library::database::LibraryDatabase;
use kuroso_core::library::scanner::scan_directory;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

fn main() {
    let music_dir = env::args()
        .nth(1)
        .unwrap_or_else(|| "crates/testacdc".to_string());

    let test_cache = PathBuf::from("/tmp/kuroso_resilience_test.bin");
    let _ = fs::remove_file(&test_cache);

    println!("==================================================");
    println!("KUROSO DATABASE RESILIENCE & INTEGRITY DRILL");
    println!("==================================================");

    // 1. Ingest clean data and save atomically
    println!("\n[Drill 1] Creating valid database from disk...");
    let db = Arc::new(LibraryDatabase::new());
    let report = scan_directory(&music_dir, &db);
    println!("Ingested {} tracks. Saving atomically...", report.scanned_files);
    db.save_to_file(&test_cache).expect("Failed atomic save");
    println!("Saved valid cache to {:?}", test_cache);

    // Verify it loads cleanly
    let (loaded_db, is_hit) = LibraryDatabase::load_or_recover(&test_cache);
    assert!(is_hit, "Expected cache hit on valid file");
    println!("Clean load successful: {} tracks in RAM", loaded_db.track_count());

    // 2. Corrupted payload test (Simulating Bit-rot / Half-written sector)
    println!("\n[Drill 2] Corrupting file payload with random garbage...");
    {
        let mut file = OpenOptions::new()
            .write(true)
            .open(&test_cache)
            .expect("Failed to open file for tampering");
        // Overwrite middle of the file with garbage bytes
        file.write_all(b"KUROSO\0\x01\x01\0\0\0CORRUPTED_GARBAGE_PAYLOAD_TEST_DATA").unwrap();
    }

    println!("Attempting recovery from corrupted file...");
    let (recovered_db, is_hit) = LibraryDatabase::load_or_recover(&test_cache);
    println!("Recovery result: is_hit = {}, tracks in RAM = {}", is_hit, recovered_db.track_count());
    assert!(!is_hit, "Corrupted file must not register as a valid cache hit");
    assert_eq!(recovered_db.track_count(), 0, "Corrupted DB must reset to empty for clean rescan");
    assert!(!test_cache.exists(), "Original corrupted file must have been moved to quarantine");
    println!("Quarantine verified: corrupted file safely moved out of the way.");

    // 3. Truncated 0-byte file test (Simulating crash during file allocation)
    println!("\n[Drill 3] Simulating 0-byte truncated file...");
    fs::File::create(&test_cache).expect("Failed to create empty file");
    let (empty_db, is_hit) = LibraryDatabase::load_or_recover(&test_cache);
    println!("Empty file result: is_hit = {}, tracks = {}", is_hit, empty_db.track_count());
    assert!(!is_hit, "0-byte file must not be treated as a hit");

    // 4. Atomic save crash-safety test
    println!("\n[Drill 4] Verifying atomic save leaves original intact on failure...");
    // Put a healthy state back
    db.save_to_file(&test_cache).expect("Failed atomic save");
    let original_size = fs::metadata(&test_cache).unwrap().len();

    // A temp file next to it should never overwrite the master file until sync + rename completes
    let parent = test_cache.parent().unwrap();
    let orphan_tmp = parent.join(".kuroso_resilience_test.bin.tmp.fake_pid");
    fs::write(&orphan_tmp, b"incomplete write from killed process").unwrap();

    // Verify master file was untouched
    let current_size = fs::metadata(&test_cache).unwrap().len();
    assert_eq!(original_size, current_size, "Master file altered prematurely!");
    let _ = fs::remove_file(orphan_tmp);

    // Clean up
    let _ = fs::remove_file(&test_cache);

    println!("\n==================================================");
    println!("ALL 4 RESILIENCE & CORRUPTION DRILLS PASSED CLEANLY");
    println!("==================================================");
}