use crate::library::types::*;
use parking_lot::RwLock;
use smol_str::SmolStr;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Default)]
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
            tracks: Vec::new(),
        };

        state.album_lookup_to_id.insert(key, id);
        state.albums.insert(id, album);

        if let Some(artist) = state.artists.get_mut(&artist_id) {
            artist.albums.push(id);
        }

        id
    }

    pub fn insert_track(
        &self,
        path: PathBuf,
        mtime: u64,
        file_size: u64,
        title: &str,
        artist_name: &str,
        album_name: Option<&str>,
        duration_ms: u32,
        track_number: Option<u16>,
        disc_number: Option<u8>,
        year: Option<u16>,
        format: AudioFormat,
        sample_rate: Option<u32>,
        bitrate: Option<u32>,
    ) -> TrackId {
        let mut state = self.state.write();

        if let Some(&existing_id) = state.path_to_track.get(&path) {
            return existing_id;
        }

        let artist_id = self.resolve_or_create_artist(&mut state, artist_name);
        let album_id = album_name.map(|name| self.resolve_or_create_album(&mut state, artist_id, name, year));
        let track_id = self.next_track_id();

        let track = Track {
            id: track_id,
            path: path.clone(),
            mtime,
            file_size,
            title: SmolStr::new(title),
            artist_id,
            album_id,
            duration_ms,
            track_number,
            disc_number,
            year,
            format,
            sample_rate,
            bitrate,
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

    pub fn update_track_metadata(
        &self,
        path: &Path,
        mtime: u64,
        file_size: u64,
        title: &str,
        artist_name: &str,
        album_name: Option<&str>,
        duration_ms: u32,
        track_number: Option<u16>,
        disc_number: Option<u8>,
        year: Option<u16>,
        sample_rate: Option<u32>,
        bitrate: Option<u32>,
    ) -> Option<TrackId> {
        let mut state = self.state.write();
        let track_id = *state.path_to_track.get(path)?;

        let old_track = state.tracks.get(&track_id)?.clone();
        let new_artist_id = self.resolve_or_create_artist(&mut state, artist_name);
        let new_album_id = album_name.map(|name| self.resolve_or_create_album(&mut state, new_artist_id, name, year));

        if old_track.album_id != new_album_id {
            if let Some(old_aid) = old_track.album_id {
                let mut album_empty = false;
                if let Some(album) = state.albums.get_mut(&old_aid) {
                    album.tracks.retain(|&id| id != track_id);
                    album_empty = album.tracks.is_empty();
                }
                if album_empty {
                    if let Some(removed) = state.albums.remove(&old_aid) {
                        state.album_lookup_to_id.remove(&(removed.artist_id, removed.title));
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
            track.album_id = new_album_id;
            track.duration_ms = duration_ms;
            track.track_number = track_number;
            track.disc_number = disc_number;
            track.year = year;
            track.sample_rate = sample_rate;
            track.bitrate = bitrate;
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_relational_linkage() {
        let db = LibraryDatabase::new();

        let track_id = db.insert_track(
            PathBuf::from("/music/track1.opus"),
            1000,
            2048,
            "Track One",
            "Artist A",
            Some("Album A"),
            210000,
            Some(1),
            Some(1),
            Some(2023),
            AudioFormat::Opus,
            Some(48000),
            Some(160000),
        );

        assert_eq!(db.track_count(), 1);
        assert_eq!(db.album_count(), 1);
        assert_eq!(db.artist_count(), 1);

        let track = db.get_track(track_id).expect("Track must exist");
        assert_eq!(track.title.as_str(), "Track One");

        let album = db.get_album(track.album_id.unwrap()).expect("Album must exist");
        assert_eq!(album.title.as_str(), "Album A");
        assert_eq!(album.tracks, vec![track_id]);

        let artist = db.get_artist(track.artist_id).expect("Artist must exist");
        assert_eq!(artist.name.as_str(), "Artist A");
        assert_eq!(artist.albums, vec![album.id]);
    }

    #[test]
    fn test_update_track_metadata_and_relink() {
        let db = LibraryDatabase::new();
        let path = PathBuf::from("/music/change_me.opus");

        let id = db.insert_track(
            path.clone(),
            100,
            1000,
            "Initial Title",
            "Artist Old",
            Some("Album Old"),
            180000,
            Some(1),
            Some(1),
            Some(2020),
            AudioFormat::Opus,
            Some(48000),
            Some(128000),
        );

        assert_eq!(db.artist_count(), 1);
        assert_eq!(db.album_count(), 1);

        let updated_id = db.update_track_metadata(
            &path,
            200,
            1200,
            "New Title",
            "Artist New",
            Some("Album New"),
            185000,
            Some(2),
            Some(1),
            Some(2021),
            Some(48000),
            Some(160000),
        );

        assert_eq!(updated_id, Some(id));
        assert_eq!(db.artist_count(), 1);
        assert_eq!(db.album_count(), 1);

        let track = db.get_track(id).unwrap();
        assert_eq!(track.title.as_str(), "New Title");
        assert_eq!(track.track_number, Some(2));

        let new_album = db.get_album(track.album_id.unwrap()).unwrap();
        assert_eq!(new_album.title.as_str(), "Album New");
        assert_eq!(new_album.tracks, vec![id]);

        let new_artist = db.get_artist(track.artist_id).unwrap();
        assert_eq!(new_artist.name.as_str(), "Artist New");
    }

    #[test]
    fn test_deletion_and_automatic_orphan_pruning() {
        let db = LibraryDatabase::new();
        let path = PathBuf::from("/music/single.mp3");

        let track_id = db.insert_track(
            path.clone(),
            1000,
            4096,
            "Alone",
            "Solo Artist",
            None,
            180000,
            None,
            None,
            Some(2022),
            AudioFormat::Mp3,
            Some(44100),
            Some(320000),
        );

        assert_eq!(db.track_count(), 1);
        assert_eq!(db.artist_count(), 1);
        assert_eq!(db.album_count(), 0);

        let removed = db.remove_track_by_path(&path);
        assert_eq!(removed, Some(track_id));

        assert_eq!(db.track_count(), 0);
        assert_eq!(db.artist_count(), 0);
        assert_eq!(db.album_count(), 0);
    }
}