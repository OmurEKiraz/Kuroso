use kuroso_core::audio::{PlaybackQueue, Player};
use kuroso_core::library::LibraryEngine;
use kuroso_ui::KurosoTui;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target_dir = env::args()
        .nth(1)
        .unwrap_or_else(|| "crates/testacdc".to_string());
    let music_path = PathBuf::from(&target_dir);
    let cache_path = std::env::temp_dir().join("kuroso_tui_cache.bin");

    let engine = LibraryEngine::open(&music_path, &cache_path)?;
    let db = Arc::new(engine.database().clone());

    let mut track_ids = Vec::new();
    db.for_each_track(|t, _album, _artist| {
        track_ids.push(t.id);
    });

    if track_ids.is_empty() {
        eprintln!("No tracks found in: {}", music_path.display());
        return Ok(());
    }

    let mut queue = PlaybackQueue::default();
    queue.load_tracks(track_ids, Some(0));

    let player = Player::new(queue, Arc::clone(&db))
        .map_err(|e| format!("Player failed to initialize: {e}"))?;

    let tui = KurosoTui::new(player);
    tui.run_loop();

    let _ = std::fs::remove_file(&cache_path);
    Ok(())
}