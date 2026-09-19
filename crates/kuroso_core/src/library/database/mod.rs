use crate::library::types::*;
use bincode::Options;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const DB_MAGIC: &[u8; 8] = b"KUROSO\0\x01";
const DB_VERSION: u32 = 1;
const HEADER_SIZE: usize = 8 + 4; // 8 bytes magic + 4 bytes version

#[derive(Debug)]
pub enum DatabaseLoadError {
    Io(std::io::Error),
    NotFound,
    EmptyFile,
    FileTooShort,
    InvalidMagic,
    IncompatibleVersion { found: u32, expected: u32 },
    CorruptedData(bincode::Error),
}

impl std::fmt::Display for DatabaseLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::NotFound => write!(f, "Database cache file not found"),
            Self::EmptyFile => write!(f, "Database cache file is empty (0 bytes)"),
            Self::FileTooShort => write!(f, "Database cache file is shorter than valid header"),
            Self::InvalidMagic => write!(f, "Invalid magic bytes (not a Kuroso database)"),
            Self::IncompatibleVersion { found, expected } => {
                write!(f, "Incompatible schema version: found {found}, expected {expected}")
            }
            Self::CorruptedData(e) => write!(f, "Corrupted database payload: {e}"),
        }
    }
}

impl std::error::Error for DatabaseLoadError {}

#[derive(Default, Serialize, Deserialize)]
pub struct DatabaseState {
    pub tracks: HashMap<TrackId, Track>,
    pub albums: HashMap<AlbumId, Album>,
    pub artists: HashMap<ArtistId, Artist>,

    pub path_to_track: HashMap<PathBuf, TrackId>,
    pub artist_name_to_id: HashMap<SmolStr, ArtistId>,
    pub album_lookup_to_id: HashMap<(ArtistId, SmolStr), AlbumId>,
}

pub struct LibraryDatabase {
    state: RwLock<DatabaseState>,
    track_id_counter: AtomicU32,
    album_id_counter: AtomicU32,
    artist_id_counter: AtomicU32,
}

impl Default for LibraryDatabase {
    fn default() -> Self {
        Self::new()
    }
}

impl LibraryDatabase {
    pub fn new() -> Self {
        Self {
            state: RwLock::new(DatabaseState::default()),
            track_id_counter: AtomicU32::new(1),
            album_id_counter: AtomicU32::new(1),
            artist_id_counter: AtomicU32::new(1),
        }
    }

    /// Full reinitialization: clears all in-memory tracks, albums, artists, and resets counters.
    pub fn clear(&self) {
        let mut state = self.state.write();
        *state = DatabaseState::default();
        self.track_id_counter.store(1, Ordering::SeqCst);
        self.album_id_counter.store(1, Ordering::SeqCst);
        self.artist_id_counter.store(1, Ordering::SeqCst);
    }

    /// Atomic persistence with crash-safety:
    /// Writes to a sibling `.tmp` file, calls fsync, then atomically renames over the destination.
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> std::io::Result<()> {
        let target_path = path.as_ref();
        let parent_dir = target_path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent_dir)?;

        let tmp_path = parent_dir.join(format!(
            ".{}.tmp.{}",
            target_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("kuroso_cache"),
            std::process::id()
        ));

        {
            let file = File::create(&tmp_path)?;
            let mut writer = BufWriter::new(file);

            // Write header: 8 bytes magic + 4 bytes version
            writer.write_all(DB_MAGIC)?;
            writer.write_all(&DB_VERSION.to_le_bytes())?;

            // Serialize payload with bincode options
            let state = self.state.read();
            bincode::DefaultOptions::new()
                .with_fixint_encoding()
                .serialize_into(&mut writer, &*state)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

            writer.flush()?;
            writer.get_ref().sync_all()?;
        }

        fs::rename(&tmp_path, target_path)?;
        Ok(())
    }

    /// Loads the binary database from disk with comprehensive validation checks.
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, DatabaseLoadError> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(DatabaseLoadError::NotFound);
        }

        let metadata = fs::metadata(path).map_err(DatabaseLoadError::Io)?;
        let file_len = metadata.len();

        if file_len == 0 {
            return Err(DatabaseLoadError::EmptyFile);
        }

        if (file_len as usize) < HEADER_SIZE {
            return Err(DatabaseLoadError::FileTooShort);
        }

        let file = File::open(path).map_err(DatabaseLoadError::Io)?;
        let mut reader = BufReader::new(file);

        let mut magic_buf = [0u8; 8];
        reader
            .read_exact(&mut magic_buf)
            .map_err(DatabaseLoadError::Io)?;
        if &magic_buf != DB_MAGIC {
            return Err(DatabaseLoadError::InvalidMagic);
        }

        let mut version_buf = [0u8; 4];
        reader
            .read_exact(&mut version_buf)
            .map_err(DatabaseLoadError::Io)?;
        let version = u32::from_le_bytes(version_buf);
        if version != DB_VERSION {
            return Err(DatabaseLoadError::IncompatibleVersion {
                found: version,
                expected: DB_VERSION,
            });
        }

        let state: DatabaseState = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .with_limit(file_len)
            .deserialize_from(reader)
            .map_err(DatabaseLoadError::CorruptedData)?;

        let max_track = state.tracks.keys().map(|k| k.0).max().unwrap_or(0);
        let max_album = state.albums.keys().map(|k| k.0).max().unwrap_or(0);
        let max_artist = state.artists.keys().map(|k| k.0).max().unwrap_or(0);

        Ok(Self {
            state: RwLock::new(state),
            track_id_counter: AtomicU32::new(max_track + 1),
            album_id_counter: AtomicU32::new(max_album + 1),
            artist_id_counter: AtomicU32::new(max_artist + 1),
        })
    }

    /// Load database or recover cleanly:
    /// If missing, returns a clean new DB.
    /// If corrupted/invalid, renames broken file to `.corrupt.<timestamp>` and returns a clean DB.
    pub fn load_or_recover<P: AsRef<Path>>(path: P) -> (Self, bool) {
        let p = path.as_ref();
        match Self::load_from_file(p) {
            Ok(db) => {
                let is_blank = db.track_count() == 0;
                (db, !is_blank)
            }
            Err(DatabaseLoadError::NotFound) => (Self::new(), false),
            Err(err) => {
                eprintln!("[WARN] Kuroso cache validation failed ({err}). Quarantining corrupted cache.");
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let backup_path = p.with_extension(format!("corrupt.{now}"));
                let _ = fs::rename(p, backup_path);
                (Self::new(), false)
            }
        }
    }

    pub fn should_rescan(&self, path: &Path, mtime: u64, file_size: u64) -> bool {
        let state = self.state.read();
        if let Some(&track_id) = state.path_to_track.get(path) {
            if let Some(track) = state.tracks.get(&track_id) {
                return track.mtime != mtime || track.file_size != file_size;
            }
        }
        true
    }

    /// Prunes files missing from `live_paths`, but ONLY if the file resides
    /// under an accessible/active root. If a removable drive root is currently
    /// missing or unmounted, its tracks are preserved.
    pub fn prune_missing_files_scoped<P: AsRef<Path>>(
        &self,
        active_roots: &[P],
        live_paths: &HashSet<PathBuf>,
    ) -> usize {
        let valid_roots: Vec<&Path> = active_roots
            .iter()
            .map(|r| r.as_ref())
            .filter(|r| r.exists() && r.is_dir())
            .collect();

        let dead_paths: Vec<PathBuf> = {
            let state = self.state.read();
            state
                .path_to_track
                .keys()
                .filter(|p| {
                    // Check if file belongs to any currently active root
                    let under_active_root = valid_roots.iter().any(|root| p.starts_with(root));
                    // Only prune if it was supposed to be scanned and is missing
                    under_active_root && !live_paths.contains(*p)
                })
                .cloned()
                .collect()
        };

        let count = dead_paths.len();
        for path in dead_paths {
            self.remove_track_by_path(&path);
        }
        count
    }

    /// Backwards-compatible wrapper: prunes across a single root directory.
    pub fn prune_missing_files(&self, live_paths: &HashSet<PathBuf>) -> usize {
        let dead_paths: Vec<PathBuf> = {
            let state = self.state.read();
            state
                .path_to_track
                .keys()
                .filter(|p| !live_paths.contains(*p))
                .cloned()
                .collect()
        };

        let count = dead_paths.len();
        for path in dead_paths {
            self.remove_track_by_path(&path);
        }
        count
    }

    pub fn next_track_id(&self) -> TrackId {
        TrackId(self.track_id_counter.fetch_add(1, Ordering::Relaxed))
    }

    pub fn next_album_id(&self) -> AlbumId {
        AlbumId(self.album_id_counter.fetch_add(1, Ordering::Relaxed))
    }

    pub fn next_artist_id(&self) -> ArtistId {
        ArtistId(self.artist_id_counter.fetch_add(1, Ordering::Relaxed))
    }

    pub fn resolve_or_create_artist(&self, state: &mut DatabaseState, name: &str) -> ArtistId {
        let name_str = SmolStr::new(name);
        if let Some(&id) = state.artist_name_to_id.get(&name_str) {
            return id;
        }

        let id = self.next_artist_id();
        let artist = Artist {
            id,
            name: name_str.clone(),
            albums: Vec::new(),
            standalone_tracks: Vec::new(),
        };

        state.artist_name_to_id.insert(name_str, id);
        state.artists.insert(id, artist);
        id
    }

    pub fn resolve_or_create_album(
        &self,
        state: &mut DatabaseState,
        artist_id: ArtistId,
        title: &str,
        year: Option<u16>,
        is_compilation: bool,
    ) -> AlbumId {
        let title_str = SmolStr::new(title);
        let key = (artist_id, title_str.clone());

        if let Some(&id) = state.album_lookup_to_id.get(&key) {
            return id;
        }

        let id = self.next_album_id();
        let album = Album {
            id,
            title: title_str,
            artist_id,
            year,
            is_compilation,
            tracks: Vec::new(),
        };

        state.album_lookup_to_id.insert(key, id);
        state.albums.insert(id, album);

        if let Some(artist) = state.artists.get_mut(&artist_id) {
            artist.albums.push(id);
        }

        id
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_track(
        &self,
        path: PathBuf,
        mtime: u64,
        file_size: u64,
        title: &str,
        artist_name: &str,
        album_artist_name: Option<&str>,
        album_name: Option<&str>,
        duration_ms: u32,
        track_number: Option<u16>,
        disc_number: Option<u8>,
        year: Option<u16>,
        format: AudioFormat,
        sample_rate: Option<u32>,
        bitrate: Option<u32>,
        bit_depth: Option<u8>,
        channels: Option<u8>,
        track_gain_db: Option<f32>,
        track_peak: Option<f32>,
        album_gain_db: Option<f32>,
        album_peak: Option<f32>,
    ) -> TrackId {
        let mut state = self.state.write();

        if let Some(&existing_id) = state.path_to_track.get(&path) {
            return existing_id;
        }

        let artist_id = self.resolve_or_create_artist(&mut state, artist_name);
        let album_artist_id = album_artist_name.map(|name| self.resolve_or_create_artist(&mut state, name));

        // Use album artist if present, otherwise default to track artist for the album's primary anchor
        let album_owner_id = album_artist_id.unwrap_or(artist_id);
        let is_compilation = album_artist_name
            .map(|a| a.eq_ignore_ascii_case("various artists"))
            .unwrap_or(false);

        let album_id = album_name.map(|name| {
            self.resolve_or_create_album(&mut state, album_owner_id, name, year, is_compilation)
        });
        let track_id = self.next_track_id();

        let track = Track {
            id: track_id,
            path: path.clone(),
            mtime,
            file_size,
            title: SmolStr::new(title),
            artist_id,
            album_artist_id,
            album_id,
            duration_ms,
            track_number,
            disc_number,
            year,
            format,
            sample_rate,
            bitrate,
            bit_depth,
            channels,
            track_gain_db,
            track_peak,
            album_gain_db,
            album_peak,
        };

        state.path_to_track.insert(path, track_id);
        state.tracks.insert(track_id, track);

        if let Some(aid) = album_id {
            if let Some(album) = state.albums.get_mut(&aid) {
                album.tracks.push(track_id);
            }
        } else if let Some(artist) = state.artists.get_mut(&artist_id) {
            artist.standalone_tracks.push(track_id);
        }

        track_id
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_track_metadata(
        &self,
        path: &Path,
        mtime: u64,
        file_size: u64,
        title: &str,
        artist_name: &str,
        album_artist_name: Option<&str>,
        album_name: Option<&str>,
        duration_ms: u32,
        track_number: Option<u16>,
        disc_number: Option<u8>,
        year: Option<u16>,
        sample_rate: Option<u32>,
        bitrate: Option<u32>,
        bit_depth: Option<u8>,
        channels: Option<u8>,
        track_gain_db: Option<f32>,
        track_peak: Option<f32>,
        album_gain_db: Option<f32>,
        album_peak: Option<f32>,
    ) -> Option<TrackId> {
        let mut state = self.state.write();
        let track_id = *state.path_to_track.get(path)?;

        let old_track = state.tracks.get(&track_id)?.clone();
        let new_artist_id = self.resolve_or_create_artist(&mut state, artist_name);
        let new_album_artist_id =
            album_artist_name.map(|name| self.resolve_or_create_artist(&mut state, name));

        let new_album_owner_id = new_album_artist_id.unwrap_or(new_artist_id);
        let is_compilation = album_artist_name
            .map(|a| a.eq_ignore_ascii_case("various artists"))
            .unwrap_or(false);

        let new_album_id = album_name.map(|name| {
            self.resolve_or_create_album(
                &mut state,
                new_album_owner_id,
                name,
                year,
                is_compilation,
            )
        });

        if old_track.album_id != new_album_id {
            if let Some(old_aid) = old_track.album_id {
                let mut album_empty = false;
                if let Some(album) = state.albums.get_mut(&old_aid) {
                    album.tracks.retain(|&id| id != track_id);
                    album_empty = album.tracks.is_empty();
                }
                if album_empty {
                    if let Some(removed) = state.albums.remove(&old_aid) {
                        state
                            .album_lookup_to_id
                            .remove(&(removed.artist_id, removed.title));
                        if let Some(art) = state.artists.get_mut(&removed.artist_id) {
                            art.albums.retain(|&id| id != old_aid);
                        }
                    }
                }
            }
        }

        if old_track.artist_id != new_artist_id && old_track.album_id.is_none() {
            if let Some(old_art) = state.artists.get_mut(&old_track.artist_id) {
                old_art.standalone_tracks.retain(|&id| id != track_id);
            }
        }

        if old_track.artist_id != new_artist_id {
            let mut artist_empty = false;
            if let Some(art) = state.artists.get(&old_track.artist_id) {
                if art.albums.is_empty() && art.standalone_tracks.is_empty() {
                    artist_empty = true;
                }
            }
            if artist_empty {
                if let Some(removed) = state.artists.remove(&old_track.artist_id) {
                    state.artist_name_to_id.remove(&removed.name);
                }
            }
        }

        if old_track.album_id != new_album_id {
            if let Some(aid) = new_album_id {
                if let Some(album) = state.albums.get_mut(&aid) {
                    if !album.tracks.contains(&track_id) {
                        album.tracks.push(track_id);
                    }
                }
            } else if let Some(art) = state.artists.get_mut(&new_artist_id) {
                if !art.standalone_tracks.contains(&track_id) {
                    art.standalone_tracks.push(track_id);
                }
            }
        }

        if let Some(track) = state.tracks.get_mut(&track_id) {
            track.mtime = mtime;
            track.file_size = file_size;
            track.title = SmolStr::new(title);
            track.artist_id = new_artist_id;
            track.album_artist_id = new_album_artist_id;
            track.album_id = new_album_id;
            track.duration_ms = duration_ms;
            track.track_number = track_number;
            track.disc_number = disc_number;
            track.year = year;
            track.sample_rate = sample_rate;
            track.bitrate = bitrate;
            track.bit_depth = bit_depth;
            track.channels = channels;
            track.track_gain_db = track_gain_db;
            track.track_peak = track_peak;
            track.album_gain_db = album_gain_db;
            track.album_peak = album_peak;
        }

        Some(track_id)
    }

    pub fn remove_track_by_path(&self, path: &Path) -> Option<TrackId> {
        let mut state = self.state.write();
        let track_id = state.path_to_track.remove(path)?;
        let track = state.tracks.remove(&track_id)?;

        if let Some(album_id) = track.album_id {
            let mut album_empty = false;
            if let Some(album) = state.albums.get_mut(&album_id) {
                album.tracks.retain(|&id| id != track_id);
                album_empty = album.tracks.is_empty();
            }

            if album_empty {
                if let Some(removed_album) = state.albums.remove(&album_id) {
                    state
                        .album_lookup_to_id
                        .remove(&(removed_album.artist_id, removed_album.title));

                    if let Some(artist) = state.artists.get_mut(&removed_album.artist_id) {
                        artist.albums.retain(|&id| id != album_id);
                    }
                }
            }
        } else if let Some(artist) = state.artists.get_mut(&track.artist_id) {
            artist.standalone_tracks.retain(|&id| id != track_id);
        }

        let mut artist_empty = false;
        if let Some(artist) = state.artists.get(&track.artist_id) {
            if artist.albums.is_empty() && artist.standalone_tracks.is_empty() {
                artist_empty = true;
            }
        }

        if artist_empty {
            if let Some(removed_artist) = state.artists.remove(&track.artist_id) {
                state.artist_name_to_id.remove(&removed_artist.name);
            }
        }

        Some(track_id)
    }

    pub fn get_track_by_path(&self, path: &Path) -> Option<Track> {
        let state = self.state.read();
        let track_id = state.path_to_track.get(path)?;
        state.tracks.get(track_id).cloned()
    }

    pub fn get_track(&self, id: TrackId) -> Option<Track> {
        self.state.read().tracks.get(&id).cloned()
    }

    pub fn get_album(&self, id: AlbumId) -> Option<Album> {
        self.state.read().albums.get(&id).cloned()
    }

    pub fn get_artist(&self, id: ArtistId) -> Option<Artist> {
        self.state.read().artists.get(&id).cloned()
    }

    pub fn track_count(&self) -> usize {
        self.state.read().tracks.len()
    }

    pub fn album_count(&self) -> usize {
        self.state.read().albums.len()
    }

    pub fn artist_count(&self) -> usize {
        self.state.read().artists.len()
    }

    pub fn for_each_track<F>(&self, mut f: F)
    where
        F: FnMut(&Track, Option<&Album>, &Artist),
    {
        let state = self.state.read();
        for track in state.tracks.values() {
            if let Some(artist) = state.artists.get(&track.artist_id) {
                let album = track.album_id.and_then(|aid| state.albums.get(&aid));
                f(track, album, artist);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bincode_save_and_load_roundtrip() {
        let db = LibraryDatabase::new();
        let p = PathBuf::from("/music/roundtrip.opus");

        let id = db.insert_track(
            p.clone(),
            500,
            1024,
            "Binary Persistence",
            "Speed Arch",
            None,
            Some("Zero Alloc"),
            180000,
            Some(1),
            Some(1),
            Some(2026),
            AudioFormat::Opus,
            Some(48000),
            Some(128000),
            Some(16),
            Some(2),
            Some(-5.0),
            Some(0.95),
            Some(-4.5),
            Some(0.98),
        );

        let temp_dir = std::env::temp_dir();
        let cache_path = temp_dir.join("kuroso_test_cache_roundtrip.bin");

        db.save_to_file(&cache_path).expect("Failed to save binary cache");
        let loaded_db =
            LibraryDatabase::load_from_file(&cache_path).expect("Failed to load binary cache");

        assert_eq!(loaded_db.track_count(), 1);
        assert_eq!(loaded_db.album_count(), 1);
        assert_eq!(loaded_db.artist_count(), 1);

        let t = loaded_db.get_track(id).expect("Track missing");
        assert_eq!(t.title.as_str(), "Binary Persistence");
        assert_eq!(t.bit_depth, Some(16));
        assert_eq!(t.channels, Some(2));
        assert_eq!(t.track_gain_db, Some(-5.0));

        let _ = fs::remove_file(cache_path);
    }

    #[test]
    fn test_empty_file_fails_cleanly() {
        let temp_dir = std::env::temp_dir();
        let cache_path = temp_dir.join("kuroso_test_empty.bin");
        File::create(&cache_path).expect("Failed to create empty file");

        let result = LibraryDatabase::load_from_file(&cache_path);
        assert!(matches!(result, Err(DatabaseLoadError::EmptyFile)));

        let (recovered_db, is_hit) = LibraryDatabase::load_or_recover(&cache_path);
        assert!(!is_hit);
        assert_eq!(recovered_db.track_count(), 0);

        let _ = fs::remove_file(cache_path);
    }

    #[test]
    fn test_truncated_header_fails_cleanly() {
        let temp_dir = std::env::temp_dir();
        let cache_path = temp_dir.join("kuroso_test_short.bin");
        {
            let mut f = File::create(&cache_path).unwrap();
            f.write_all(b"SHORT").unwrap();
        }

        let result = LibraryDatabase::load_from_file(&cache_path);
        assert!(matches!(result, Err(DatabaseLoadError::FileTooShort)));

        let _ = fs::remove_file(cache_path);
    }

    #[test]
    fn test_corrupted_payload_quarantines_file() {
        let temp_dir = std::env::temp_dir();
        let cache_path = temp_dir.join("kuroso_test_corrupt.bin");
        {
            let mut f = File::create(&cache_path).unwrap();
            f.write_all(DB_MAGIC).unwrap();
            f.write_all(&DB_VERSION.to_le_bytes()).unwrap();
            f.write_all(b"this is corrupt binary payload garbage").unwrap();
        }

        let (recovered_db, is_hit) = LibraryDatabase::load_or_recover(&cache_path);
        assert!(!is_hit);
        assert_eq!(recovered_db.track_count(), 0);
        assert!(!cache_path.exists());
    }

    #[test]
    fn test_scoped_pruning_protects_unmounted_removable_drives() {
        let db = LibraryDatabase::new();

        let internal_path = PathBuf::from("/music/internal/track1.opus");
        let usb_path = PathBuf::from("/mnt/usb_drive/track2.opus");

        db.insert_track(
            internal_path.clone(),
            100, 1000, "Internal Track", "Artist", None, None,
            180_000, None, None, None, AudioFormat::Opus,
            None, None, None, None, None, None, None, None,
        );

        db.insert_track(
            usb_path.clone(),
            100, 1000, "USB Track", "Artist", None, None,
            180_000, None, None, None, AudioFormat::Opus,
            None, None, None, None, None, None, None, None,
        );

        assert_eq!(db.track_count(), 2);

        // Simulate rescan where /mnt/usb_drive is unmounted (does not exist on disk)
        // Only a local temp folder exists as an active root
        let temp_internal_root = std::env::temp_dir();
        let unmounted_usb_root = PathBuf::from("/mnt/definitely_not_mounted_usb_drive_kuroso");

        let live_paths = HashSet::new();
        // live_paths has NO files at all (e.g. internal deleted, usb unmounted)

        let pruned = db.prune_missing_files_scoped(
            &[&temp_internal_root, &unmounted_usb_root],
            &live_paths,
        );

        // USB track is untouched because unmounted_usb_root does not exist on disk
        assert_eq!(pruned, 0);
        assert_eq!(db.track_count(), 2);
        assert!(db.get_track_by_path(&usb_path).is_some());
    }

    #[test]
    fn test_database_clear_resets_state() {
        let db = LibraryDatabase::new();
        db.insert_track(
            PathBuf::from("/music/s.opus"),
            100,
            100,
            "Song",
            "Artist",
            None,
            None,
            1000,
            None,
            None,
            None,
            AudioFormat::Opus,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        assert_eq!(db.track_count(), 1);

        db.clear();
        assert_eq!(db.track_count(), 0);
        assert_eq!(db.album_count(), 0);
        assert_eq!(db.artist_count(), 0);
    }
}