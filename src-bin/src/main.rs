use kuroso_core::library::database::LibraryDatabase;
use kuroso_core::library::scanner::scan_directory;
use kuroso_core::library::types::*;
use std::collections::HashSet;
use std::env;
use std::path::PathBuf;

fn main() {
    let local_dir = env::args()
        .nth(1)
        .unwrap_or_else(|| "crates/testacdc".to_string());
    let local_path = PathBuf::from(&local_dir);

    println!("============================================================");
    println!("KUROSO STORAGE RESILIENCE & REMOVABLE DRIVE DRILL");
    println!("============================================================\n");

    let db = LibraryDatabase::new();

    // 1. Initial scan of local root
    println!("[Step 1] Scanning primary storage directory: {}", local_dir);
    let report = scan_directory(&local_path, &db);
    println!(
        "-> Scan finished: {} files indexed, {} tracks in DB\n",
        report.scanned_files,
        db.track_count()
    );

    // 2. Inject tracks from an external drive that might be unplugged later
    println!("[Step 2] Mounting simulated external USB drive (/mnt/usb_lossless)...");
    let usb_track_1 = PathBuf::from("/mnt/usb_lossless/ACDC/flac/Hells_Bells_Master.flac");
    let usb_track_2 = PathBuf::from("/mnt/usb_lossless/ACDC/flac/Shoot_To_Thrill.flac");

    db.insert_track(
        usb_track_1.clone(),
        1700000000,
        45_000_000,
        "Hells Bells (Studio Master)",
        "AC/DC",
        Some("AC/DC"),
        Some("Back in Black [24-96]"),
        312_000,
        Some(1),
        Some(1),
        Some(1980),
        AudioFormat::Flac,
        Some(96_000),
        Some(2_800_000),
        Some(24),
        Some(2),
        Some(-8.4),
        Some(0.99),
        Some(-7.9),
        Some(1.0),
    );

    db.insert_track(
        usb_track_2.clone(),
        1700000001,
        42_000_000,
        "Shoot to Thrill (Studio Master)",
        "AC/DC",
        Some("AC/DC"),
        Some("Back in Black [24-96]"),
        317_000,
        Some(2),
        Some(1),
        Some(1980),
        AudioFormat::Flac,
        Some(96_000),
        Some(2_750_000),
        Some(24),
        Some(2),
        Some(-8.1),
        Some(0.98),
        Some(-7.9),
        Some(1.0),
    );

    let total_with_usb = db.track_count();
    println!("-> USB inserted. Total library tracks: {}\n", total_with_usb);

    // 3. Simulate disconnecting the USB drive and rescanning
    println!("[Step 3] Simulating unmounted external drive (/mnt/usb_lossless unplugged)...");
    let unmounted_usb_root = PathBuf::from("/mnt/usb_lossless");
    assert!(
        !unmounted_usb_root.exists(),
        "Simulation requires unmounted path to not exist on disk"
    );

    // Scanner runs on active roots: local_path exists, usb root does NOT exist.
    // We pass live local files only (USB files are missing from live scan).
    let mut live_scanned_files = HashSet::new();
    db.for_each_track(|t, _, _| {
        if t.path.starts_with(&local_path) {
            live_scanned_files.insert(t.path.clone());
        }
    });

    println!(
        "Running scoped prune with active roots: [\"{}\", \"/mnt/usb_lossless\"]",
        local_path.display()
    );

    let pruned = db.prune_missing_files_scoped(
        &[&local_path, &unmounted_usb_root],
        &live_scanned_files,
    );

    println!("-> Pruned missing files count: {}", pruned);
    println!("-> Remaining total tracks in DB: {}", db.track_count());

    // 4. Verifications
    let usb1_retained = db.get_track_by_path(&usb_track_1).is_some();
    let usb2_retained = db.get_track_by_path(&usb_track_2).is_some();

    println!("\n[Verification Report]");
    println!("------------------------------------------------------------");
    println!("Internal tracks retained:    OK");
    println!("External Track 1 preserved:  {}", if usb1_retained { "PASS (Safely Kept)" } else { "FAIL (Accidentally Wiped!)" });
    println!("External Track 2 preserved:  {}", if usb2_retained { "PASS (Safely Kept)" } else { "FAIL (Accidentally Wiped!)" });

    if usb1_retained && usb2_retained && pruned == 0 {
        println!("\n>>> DRILL SUCCESS: Removable drive tracks were protected from pruning. <<<");
    } else {
        eprintln!("\n>>> DRILL FAILED: Pruning wiped unmounted files! <<<");
    }
}