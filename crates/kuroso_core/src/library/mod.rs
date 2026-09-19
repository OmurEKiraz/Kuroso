pub mod database;
pub mod queries;
pub mod scanner;
pub mod types;
pub mod watcher;

use crossbeam_channel::Receiver;
use database::LibraryDatabase;
use queries::{
    AlbumSortBy, AlbumWithTracks, ArtistSortBy, ArtistWithAlbums, LibraryQueries, SearchResult,
    SortDirection, TrackView,
};
use scanner::scan_directory;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use types::{Album, AlbumId, Artist, ArtistId};
pub use watcher::LibraryEvent;
use watcher::LibraryWatcher;

pub struct LibraryEngine {
    music_dir: PathBuf,
    cache_path: PathBuf,
    db: Arc<LibraryDatabase>,
    _watcher: Option<LibraryWatcher>,
    event_rx: Option<Receiver<LibraryEvent>>,
}

impl LibraryEngine {
    /// Opens the library engine.
    ///
    /// 1. Loads or cleanly recovers the binary database cache.
    /// 2. Performs an incremental background diff-scan against the music directory.
    /// 3. Starts the debounced filesystem watcher.
    pub fn open<P: AsRef<Path>, C: AsRef<Path>>(
        music_dir: P,
        cache_path: C,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let music_dir = music_dir.as_ref().to_path_buf();
        let cache_path = cache_path.as_ref().to_path_buf();

        // Load cache or create clean state
        let (db, _is_cache_hit) = LibraryDatabase::load_or_recover(&cache_path);
        let db = Arc::new(db);

        // Perform initial diff scan
        if music_dir.exists() {
            let _ = scan_directory(&music_dir, &db);
            let _ = db.save_to_file(&cache_path);
        }

        // Start filesystem watcher
        let (watcher, event_rx) = if music_dir.exists() {
            let (w, rx) = LibraryWatcher::start(
                &music_dir,
                Arc::clone(&db),
                Duration::from_millis(350),
            )?;
            (Some(w), Some(rx))
        } else {
            (None, None)
        };

        Ok(Self {
            music_dir,
            cache_path,
            db,
            _watcher: watcher,
            event_rx,
        })
    }

    /// Access the event stream for reactive UI updates (TrackAdded, TrackUpdated, TrackRemoved).
    pub fn events(&self) -> Option<&Receiver<LibraryEvent>> {
        self.event_rx.as_ref()
    }

    /// Query helper instance scoped to the current database state.
    pub fn queries(&self) -> LibraryQueries<'_> {
        LibraryQueries::new(&self.db)
    }

    /// Multi-field token & direct ID search ("id:5", "rock live", etc.)
    pub fn search(&self, query: &str) -> Vec<TrackView> {
        self.queries().search(query)
    }

    /// Comprehensive multi-entity search (Tracks, Albums, Artists)
    pub fn search_all(&self, query: &str) -> SearchResult {
        self.queries().search_all(query)
    }

    /// Fetch all artists with specified sorting
    pub fn get_all_artists(&self, sort_by: ArtistSortBy, direction: SortDirection) -> Vec<Artist> {
        self.queries().get_all_artists(sort_by, direction)
    }

    /// Fetch all albums with specified sorting
    pub fn get_all_albums(&self, sort_by: AlbumSortBy, direction: SortDirection) -> Vec<Album> {
        self.queries().get_all_albums(sort_by, direction)
    }

    /// Fetch album with tracks sorted by disc and track number
    pub fn get_album_with_tracks(&self, album_id: AlbumId) -> Option<AlbumWithTracks> {
        self.queries().get_album_with_tracks(album_id)
    }

    /// Fetch artist with albums and standalone tracks
    pub fn get_artist_with_albums(&self, artist_id: ArtistId) -> Option<ArtistWithAlbums> {
        self.queries().get_artist_with_albums(artist_id)
    }

    /// Total track count in RAM
    pub fn track_count(&self) -> usize {
        self.db.track_count()
    }

    /// Total album count in RAM
    pub fn album_count(&self) -> usize {
        self.db.album_count()
    }

    /// Total artist count in RAM
    pub fn artist_count(&self) -> usize {
        self.db.artist_count()
    }

    /// Force manual rescan of the directory and save state
    pub fn rescan(&self) -> usize {
        let report = scan_directory(&self.music_dir, &self.db);
        let _ = self.save();
        report.newly_added
    }

    /// Manually trigger atomic disk persistence
    pub fn save(&self) -> std::io::Result<()> {
        self.db.save_to_file(&self.cache_path)
    }

    /// Complete reinitialization of the database
    pub fn clear(&self) {
        self.db.clear();
        let _ = self.save();
    }

    /// Shared handle to the underlying database
    pub fn database(&self) -> Arc<LibraryDatabase> {
        Arc::clone(&self.db)
    }
}

impl Drop for LibraryEngine {
    fn drop(&mut self) {
        let _ = self.save();
    }
}