use crate::library::database::LibraryDatabase;
use crate::library::queries::TrackView;
use crate::library::scanner::read_metadata;
use crate::library::types::TrackId;
use crossbeam_channel::{unbounded, Receiver, Sender};
use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode, DebounceEventResult, Debouncer};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, UNIX_EPOCH};

const SUPPORTED_EXTENSIONS: &[&str] = &["opus", "ogg", "flac", "mp3", "m4a", "aac", "wav"];

#[derive(Debug, Clone)]
pub enum LibraryEvent {
    TrackAdded(TrackView),
    TrackUpdated(TrackView),
    TrackRemoved(TrackId),
}

pub struct LibraryWatcher {
    debouncer: Debouncer<notify_debouncer_mini::notify::RecommendedWatcher>,
    worker_handle: Option<JoinHandle<()>>,
    stop_signal: Arc<AtomicBool>,
}

impl LibraryWatcher {
    /// Starts watching multiple root directories simultaneously.
    pub fn start_multiple<P: AsRef<Path>>(
        watch_paths: &[P],
        db: Arc<LibraryDatabase>,
        debounce_duration: Duration,
    ) -> Result<(Self, Receiver<LibraryEvent>), Box<dyn std::error::Error>> {
        let (raw_event_tx, raw_event_rx) = unbounded();
        let (client_event_tx, client_event_rx) = unbounded();
        let stop_signal = Arc::new(AtomicBool::new(false));

        let mut debouncer = new_debouncer(debounce_duration, move |res: DebounceEventResult| {
            if let Ok(events) = res {
                for event in events {
                    let _ = raw_event_tx.send(event.path);
                }
            }
        })?;

        for path in watch_paths {
            let p = path.as_ref();
            if p.exists() && p.is_dir() {
                debouncer.watcher().watch(p, RecursiveMode::Recursive)?;
            }
        }

        let stop_clone = Arc::clone(&stop_signal);
        let worker_handle = thread::spawn(move || {
            Self::worker_loop(raw_event_rx, client_event_tx, db, stop_clone);
        });

        Ok((
            Self {
                debouncer,
                worker_handle: Some(worker_handle),
                stop_signal,
            },
            client_event_rx,
        ))
    }

    /// Single directory helper maintaining backwards compatibility.
    pub fn start<P: AsRef<Path>>(
        watch_path: P,
        db: Arc<LibraryDatabase>,
        debounce_duration: Duration,
    ) -> Result<(Self, Receiver<LibraryEvent>), Box<dyn std::error::Error>> {
        Self::start_multiple(&[watch_path.as_ref()], db, debounce_duration)
    }

    /// Dynamically registers a newly mounted or added directory to live watching.
    pub fn watch_directory<P: AsRef<Path>>(&mut self, path: P) -> Result<(), Box<dyn std::error::Error>> {
        let p = path.as_ref();
        if p.exists() && p.is_dir() {
            self.debouncer.watcher().watch(p, RecursiveMode::Recursive)?;
        }
        Ok(())
    }

    /// Unregisters an unmounted directory from live watching.
    pub fn unwatch_directory<P: AsRef<Path>>(&mut self, path: P) -> Result<(), Box<dyn std::error::Error>> {
        let _ = self.debouncer.watcher().unwatch(path.as_ref());
        Ok(())
    }

    fn worker_loop(
        rx: Receiver<PathBuf>,
        tx: Sender<LibraryEvent>,
        db: Arc<LibraryDatabase>,
        stop_signal: Arc<AtomicBool>,
    ) {
        while !stop_signal.load(Ordering::Relaxed) {
            let path = match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(p) => p,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            };

            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();

            if !SUPPORTED_EXTENSIONS.contains(&ext.as_str()) {
                continue;
            }

            // Case 1: File removed
            if !path.exists() {
                if let Some(removed_id) = db.remove_track_by_path(&path) {
                    let _ = tx.send(LibraryEvent::TrackRemoved(removed_id));
                }
                continue;
            }

            // Case 2: File exists / modified
            let metadata = match fs::metadata(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let mtime = match metadata.modified() {
                Ok(t) => t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
                Err(_) => continue,
            };
            let file_size = metadata.len();

            if !db.should_rescan(&path, mtime, file_size) {
                continue;
            }

            let extracted = match read_metadata(&path) {
                Some(meta) => meta,
                None => continue,
            };

            let is_update = db.get_track_by_path(&path).is_some();

            if is_update {
                if let Some(track_id) = db.update_track_metadata(
                    &extracted.path,
                    extracted.mtime,
                    extracted.file_size,
                    &extracted.title,
                    &extracted.artist,
                    extracted.album_artist.as_deref(),
                    extracted.album.as_deref(),
                    extracted.duration_ms,
                    extracted.track_number,
                    extracted.disc_number,
                    extracted.year,
                    extracted.sample_rate,
                    extracted.bitrate,
                    extracted.bit_depth,
                    extracted.channels,
                    extracted.track_gain_db,
                    extracted.track_peak,
                    extracted.album_gain_db,
                    extracted.album_peak,
                ) {
                    if let Some(track) = db.get_track(track_id) {
                        if let Some(view) = Self::resolve_view(&db, &track) {
                            let _ = tx.send(LibraryEvent::TrackUpdated(view));
                        }
                    }
                }
            } else {
                let track_id = db.insert_track(
                    extracted.path,
                    extracted.mtime,
                    extracted.file_size,
                    &extracted.title,
                    &extracted.artist,
                    extracted.album_artist.as_deref(),
                    extracted.album.as_deref(),
                    extracted.duration_ms,
                    extracted.track_number,
                    extracted.disc_number,
                    extracted.year,
                    extracted.format,
                    extracted.sample_rate,
                    extracted.bitrate,
                    extracted.bit_depth,
                    extracted.channels,
                    extracted.track_gain_db,
                    extracted.track_peak,
                    extracted.album_gain_db,
                    extracted.album_peak,
                );

                if let Some(track) = db.get_track(track_id) {
                    if let Some(view) = Self::resolve_view(&db, &track) {
                        let _ = tx.send(LibraryEvent::TrackAdded(view));
                    }
                }
            }
        }
    }

    fn resolve_view(db: &LibraryDatabase, track: &crate::library::types::Track) -> Option<TrackView> {
        let artist = db.get_artist(track.artist_id)?;
        let album = track.album_id.and_then(|id| db.get_album(id));
        let album_artist = track.album_artist_id.and_then(|id| db.get_artist(id));

        Some(TrackView {
            id: track.id,
            path: track.path.clone(),
            title: track.title.clone(),
            artist_id: track.artist_id,
            artist_name: artist.name,
            album_artist_id: track.album_artist_id,
            album_artist_name: album_artist.map(|a| a.name),
            album_id: track.album_id,
            album_title: album.map(|a| a.title),
            duration_ms: track.duration_ms,
            track_number: track.track_number,
            disc_number: track.disc_number,
            year: track.year,
            format: track.format,
            bitrate: track.bitrate,
            sample_rate: track.sample_rate,
            bit_depth: track.bit_depth,
            channels: track.channels,
            track_gain_db: track.track_gain_db,
            track_peak: track.track_peak,
            album_gain_db: track.album_gain_db,
            album_peak: track.album_peak,
        })
    }
}

impl Drop for LibraryWatcher {
    fn drop(&mut self) {
        self.stop_signal.store(true, Ordering::Relaxed);
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_watcher_ignores_unsupported_extensions() {
        let temp_dir = std::env::temp_dir().join("kuroso_test_watcher_filter");
        let _ = fs::create_dir_all(&temp_dir);

        let db = Arc::new(LibraryDatabase::new());
        let (_watcher, rx) = LibraryWatcher::start(
            &temp_dir,
            Arc::clone(&db),
            Duration::from_millis(50),
        )
        .expect("Watcher must start");

        let txt_path = temp_dir.join("notes.txt");
        let mut f = File::create(&txt_path).unwrap();
        writeln!(f, "This should be ignored").unwrap();

        let received = rx.recv_timeout(Duration::from_millis(200));
        assert!(received.is_err(), "Expected timeout for non-audio file");

        let _ = fs::remove_file(txt_path);
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_watcher_multiple_directories() {
        let base = std::env::temp_dir().join("kuroso_test_multi_watch");
        let dir_a = base.join("dir_a");
        let dir_b = base.join("dir_b");
        let _ = fs::create_dir_all(&dir_a);
        let _ = fs::create_dir_all(&dir_b);

        let db = Arc::new(LibraryDatabase::new());
        let (watcher, _rx) = LibraryWatcher::start_multiple(
            &[&dir_a, &dir_b],
            Arc::clone(&db),
            Duration::from_millis(50),
        )
        .expect("Multi watcher must start");

        // Dynamically add a third folder
        let dir_c = base.join("dir_c");
        let _ = fs::create_dir_all(&dir_c);
        let mut w = watcher;
        assert!(w.watch_directory(&dir_c).is_ok());

        let _ = fs::remove_dir_all(base);
    }
}