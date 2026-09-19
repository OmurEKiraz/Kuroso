pub mod database;
pub mod queries;
pub mod scanner;
pub mod types;
pub mod watcher;

use crossbeam_channel::Receiver;
use database::LibraryDatabase;
use parking_lot::RwLock;
use queries::{
    AlbumSortBy, AlbumWithTracks, ArtistSortBy, ArtistWithAlbums, LibraryQueries, SearchResult,
    SortDirection, TrackView,
};
use scanner::scan_directories;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use types::{Album, AlbumId, Artist, ArtistId};
pub use watcher::LibraryEvent;
use watcher::LibraryWatcher;

pub struct LibraryEngine {
    roots: RwLock<Vec<PathBuf>>,
    cache_path: PathBuf,
    db: Arc<LibraryDatabase>,
    watcher: Option<RwLock<LibraryWatcher>>,
    event_rx: Option<Receiver<LibraryEvent>>,
}

impl LibraryEngine {
    /// Opens the library engine across multiple root directories.
    ///
    /// 1. Loads or cleanly recovers the binary database cache.
    /// 2. Performs parallel diff-scan across all valid/mounted roots.
    /// 3. Spawns debounced filesystem watcher over all valid roots.
    pub fn open_roots<P: AsRef<Path>, C: AsRef<Path>>(
        roots: &[P],
        cache_path: C,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let root_paths: Vec<PathBuf> = roots.iter().map(|r| r.as_ref().to_path_buf()).collect();
        let cache_path = cache_path.as_ref().to_path_buf();

        // 1. Load cache or recover
        let (db, _is_cache_hit) = LibraryDatabase::load_or_recover(&cache_path);
        let db = Arc::new(db);

        // 2. Initial diff scan across all roots
        let _ = scan_directories(&root_paths, &db);
        let _ = db.save_to_file(&cache_path);

        // 3. Start filesystem watcher for all roots
        let (watcher, event_rx) = {
            let existing_roots: Vec<&Path> = root_paths
                .iter()
                .filter(|p| p.exists() && p.is_dir())
                .map(|p| p.as_path())
                .collect();

            if !existing_roots.is_empty() {
                let (w, rx) = LibraryWatcher::start_multiple(
                    &existing_roots,
                    Arc::clone(&db),
                    Duration::from_millis(350),
                )?;
                (Some(RwLock::new(w)), Some(rx))
            } else {
                (None, None)
            }
        };

        Ok(Self {
            roots: RwLock::new(root_paths),
            cache_path,
            db,
            watcher,
            event_rx,
        })
    }

    /// Single directory constructor for simple usage / backward compatibility.
    pub fn open<P: AsRef<Path>, C: AsRef<Path>>(
        music_dir: P,
        cache_path: C,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Self::open_roots(&[music_dir.as_ref()], cache_path)
    }

    /// Dynamically add a root directory (e.g. an external drive plugged in).
    /// Performs incremental diff-scan and attaches watcher.
    pub fn add_root<P: AsRef<Path>>(&self, path: P) -> Result<(), Box<dyn std::error::Error>> {
        let path_buf = path.as_ref().to_path_buf();

        {
            let mut roots = self.roots.write();
            if !roots.contains(&path_buf) {
                roots.push(path_buf.clone());
            }
        }

        if path_buf.exists() && path_buf.is_dir() {
            let _ = scan_directories(&[path_buf.as_path()], &self.db);
            let _ = self.save();

            if let Some(watcher_lock) = &self.watcher {
                let mut w = watcher_lock.write();
                let _ = w.watch_directory(&path_buf);
            }
        }

        Ok(())
    }

    /// Dynamically remove a root directory from active management.
    /// Unwatches the path and prunes missing tracks scoped to remaining roots.
    pub fn remove_root<P: AsRef<Path>>(&self, path: P) -> Result<(), Box<dyn std::error::Error>> {
        let path_buf = path.as_ref().to_path_buf();

        {
            let mut roots = self.roots.write();
            roots.retain(|r| r != &path_buf);
        }

        if let Some(watcher_lock) = &self.watcher {
            let mut w = watcher_lock.write();
            let _ = w.unwatch_directory(&path_buf);
        }

        let _ = self.rescan();
        Ok(())
    }

    /// List of tracked roots.
    pub fn tracked_roots(&self) -> Vec<PathBuf> {
        self.roots.read().clone()
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

    /// Rescan across all active roots and save binary snapshot
    pub fn rescan(&self) -> usize {
        let roots = self.roots.read().clone();
        let report = scan_directories(&roots, &self.db);
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