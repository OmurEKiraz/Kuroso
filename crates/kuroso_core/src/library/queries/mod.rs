use crate::library::database::LibraryDatabase;
use crate::library::types::*;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtistSortBy {
    Name,
    AlbumCount,
    TrackCount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlbumSortBy {
    Year,
    Title,
    TrackCount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackSortBy {
    AlbumOrder, // Disc -> Track -> Title
    Title,
    Duration,
    Year,
}

/// UI projection of a track with resolved relational metadata.
/// Eliminates runtime HashMap lookups during UI table rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackView {
    pub id: TrackId,
    pub path: std::path::PathBuf,
    pub title: SmolStr,
    pub artist_id: ArtistId,
    pub artist_name: SmolStr,
    pub album_id: Option<AlbumId>,
    pub album_title: Option<SmolStr>,
    pub duration_ms: u32,
    pub track_number: Option<u16>,
    pub disc_number: Option<u8>,
    pub year: Option<u16>,
    pub format: AudioFormat,
    pub bitrate: Option<u32>,
    pub sample_rate: Option<u32>,
}

impl TrackView {
    pub fn formatted_duration(&self) -> String {
        let total_seconds = self.duration_ms / 1000;
        let minutes = total_seconds / 60;
        let seconds = total_seconds % 60;
        format!("{minutes}:{seconds:02}")
    }

    pub fn formatted_track_number(&self) -> String {
        match (self.disc_number, self.track_number) {
            (Some(disc), Some(track)) if disc > 1 => format!("{disc}.{track:02}"),
            (_, Some(track)) => format!("{track:02}"),
            _ => "--".to_string(),
        }
    }
}

/// Normalized scrobble payload ready for Last.fm / ListenBrainz APIs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrobblePayload {
    pub track_title: String,
    pub artist_name: String,
    pub album_title: Option<String>,
    pub duration_seconds: u32,
    pub track_number: Option<u16>,
    pub year: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct AlbumWithTracks {
    pub album: Album,
    pub tracks: Vec<TrackView>,
}

#[derive(Debug, Clone)]
pub struct ArtistWithAlbums {
    pub artist: Artist,
    pub albums: Vec<AlbumWithTracks>,
    pub standalone_tracks: Vec<TrackView>,
}

#[derive(Debug, Clone, Default)]
pub struct SearchResult {
    pub tracks: Vec<TrackView>,
    pub albums: Vec<Album>,
    pub artists: Vec<Artist>,
}

pub struct LibraryQueries<'a> {
    db: &'a LibraryDatabase,
}

impl<'a> LibraryQueries<'a> {
    pub fn new(db: &'a LibraryDatabase) -> Self {
        Self { db }
    }

    /// Resolves a raw Track into a UI-ready TrackView with resolved names.
    pub fn track_to_view(&self, track: &Track) -> Option<TrackView> {
        let artist = self.db.get_artist(track.artist_id)?;
        let album = track.album_id.and_then(|id| self.db.get_album(id));

        Some(TrackView {
            id: track.id,
            path: track.path.clone(),
            title: track.title.clone(),
            artist_id: track.artist_id,
            artist_name: artist.name,
            album_id: track.album_id,
            album_title: album.map(|a| a.title),
            duration_ms: track.duration_ms,
            track_number: track.track_number,
            disc_number: track.disc_number,
            year: track.year,
            format: track.format,
            bitrate: track.bitrate,
            sample_rate: track.sample_rate,
        })
    }

    /// Prepares scrobble metadata. Returns None if duration is under 30s (standard scrobble threshold).
    pub fn get_scrobble_payload(&self, track_id: TrackId) -> Option<ScrobblePayload> {
        let track = self.db.get_track(track_id)?;
        let duration_seconds = track.duration_ms / 1000;

        // Scrobbler standard specification: tracks under 30 seconds should not be scrobbled
        if duration_seconds < 30 {
            return None;
        }

        let artist = self.db.get_artist(track.artist_id)?;
        let album = track.album_id.and_then(|id| self.db.get_album(id));

        Some(ScrobblePayload {
            track_title: track.title.to_string(),
            artist_name: artist.name.to_string(),
            album_title: album.map(|a| a.title.to_string()),
            duration_seconds,
            track_number: track.track_number,
            year: track.year,
        })
    }

    pub fn get_album_with_tracks(&self, album_id: AlbumId) -> Option<AlbumWithTracks> {
        let album = self.db.get_album(album_id)?;
        let mut tracks = Vec::with_capacity(album.tracks.len());

        for track_id in &album.tracks {
            if let Some(track) = self.db.get_track(*track_id) {
                if let Some(view) = self.track_to_view(&track) {
                    tracks.push(view);
                }
            }
        }

        // Natural track ordering: Disc -> Track number -> Title
        tracks.sort_by(|a, b| {
            a.disc_number
                .unwrap_or(1)
                .cmp(&b.disc_number.unwrap_or(1))
                .then_with(|| a.track_number.unwrap_or(0).cmp(&b.track_number.unwrap_or(0)))
                .then_with(|| a.title.cmp(&b.title))
        });

        Some(AlbumWithTracks { album, tracks })
    }

    pub fn get_artist_with_albums(&self, artist_id: ArtistId) -> Option<ArtistWithAlbums> {
        let artist = self.db.get_artist(artist_id)?;
        let mut albums = Vec::with_capacity(artist.albums.len());

        for album_id in &artist.albums {
            if let Some(album_data) = self.get_album_with_tracks(*album_id) {
                albums.push(album_data);
            }
        }

        // Newest albums first by default
        albums.sort_by(|a, b| b.album.year.unwrap_or(0).cmp(&a.album.year.unwrap_or(0)));

        let mut standalone_tracks = Vec::with_capacity(artist.standalone_tracks.len());
        for track_id in &artist.standalone_tracks {
            if let Some(track) = self.db.get_track(*track_id) {
                if let Some(view) = self.track_to_view(&track) {
                    standalone_tracks.push(view);
                }
            }
        }

        Some(ArtistWithAlbums {
            artist,
            albums,
            standalone_tracks,
        })
    }

    /// Retrieve all artists sorted for UI list/sidebar displays
    pub fn get_all_artists(&self, sort_by: ArtistSortBy, direction: SortDirection) -> Vec<Artist> {
        let mut artists = Vec::with_capacity(self.db.artist_count());

        for id_num in 1..=self.db.artist_count() as u32 {
            if let Some(artist) = self.db.get_artist(ArtistId(id_num)) {
                artists.push(artist);
            }
        }

        artists.sort_by(|a, b| {
            let ordering = match sort_by {
                ArtistSortBy::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                ArtistSortBy::AlbumCount => a.albums.len().cmp(&b.albums.len()),
                ArtistSortBy::TrackCount => {
                    let count_a = a.standalone_tracks.len();
                    let count_b = b.standalone_tracks.len();
                    count_a.cmp(&count_b)
                }
            };
            match direction {
                SortDirection::Ascending => ordering,
                SortDirection::Descending => ordering.reverse(),
            }
        });

        artists
    }

    /// Retrieve all albums with sorting
    pub fn get_all_albums(&self, sort_by: AlbumSortBy, direction: SortDirection) -> Vec<Album> {
        let mut albums = Vec::with_capacity(self.db.album_count());

        for id_num in 1..=self.db.album_count() as u32 {
            if let Some(album) = self.db.get_album(AlbumId(id_num)) {
                albums.push(album);
            }
        }

        albums.sort_by(|a, b| {
            let ordering = match sort_by {
                AlbumSortBy::Year => a.year.unwrap_or(0).cmp(&b.year.unwrap_or(0)),
                AlbumSortBy::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
                AlbumSortBy::TrackCount => a.tracks.len().cmp(&b.tracks.len()),
            };
            match direction {
                SortDirection::Ascending => ordering,
                SortDirection::Descending => ordering.reverse(),
            }
        });

        albums
    }

    /// Multi-field search supporting exact prefixes like "id:5", "track:3", or free-text tokens
    pub fn search(&self, query: &str) -> Vec<TrackView> {
        let clean = query.trim();
        if clean.is_empty() {
            return Vec::new();
        }

        // Edge case: Direct ID query syntax e.g. "id:42" or "track:42"
        if let Some(id_str) = clean
            .strip_prefix("id:")
            .or_else(|| clean.strip_prefix("track:"))
        {
            if let Ok(id_num) = id_str.trim().parse::<u32>() {
                if let Some(track) = self.db.get_track(TrackId(id_num)) {
                    if let Some(view) = self.track_to_view(&track) {
                        return vec![view];
                    }
                }
                return Vec::new();
            }
        }

        let tokens: Vec<String> = clean
            .to_lowercase()
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();

        let mut matches = Vec::new();

        self.db.for_each_track(|track, album, artist| {
            let title_lower = track.title.to_lowercase();
            let artist_lower = artist.name.to_lowercase();
            let album_lower = album.map(|a| a.title.to_lowercase()).unwrap_or_default();
            let track_num_str = track.track_number.map(|n| n.to_string()).unwrap_or_default();

            let all_match = tokens.iter().all(|token| {
                title_lower.contains(token)
                    || artist_lower.contains(token)
                    || album_lower.contains(token)
                    || (!track_num_str.is_empty() && track_num_str == *token)
            });

            if all_match {
                if let Some(view) = self.track_to_view(track) {
                    matches.push(view);
                }
            }
        });

        matches
    }

    /// Full multi-entity search across tracks, albums, and artists
    pub fn search_all(&self, query: &str) -> SearchResult {
        let clean = query.trim().to_lowercase();
        if clean.is_empty() {
            return SearchResult::default();
        }

        let tracks = self.search(&clean);
        let mut artists = Vec::new();
        let mut albums = Vec::new();

        for a in self.get_all_artists(ArtistSortBy::Name, SortDirection::Ascending) {
            if a.name.to_lowercase().contains(&clean) {
                artists.push(a);
            }
        }

        for alb in self.get_all_albums(AlbumSortBy::Title, SortDirection::Ascending) {
            if alb.title.to_lowercase().contains(&clean) {
                albums.push(alb);
            }
        }

        SearchResult {
            tracks,
            albums,
            artists,
        }
    }

    /// Generic pagination helper for any UI list (page is 0-indexed)
    pub fn paginate<T: Clone>(items: &[T], page: usize, page_size: usize) -> Vec<T> {
        if page_size == 0 {
            return Vec::new();
        }
        let start = page * page_size;
        if start >= items.len() {
            return Vec::new();
        }
        let end = (start + page_size).min(items.len());
        items[start..end].to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn build_test_db() -> LibraryDatabase {
        let db = LibraryDatabase::new();

        db.insert_track(
            PathBuf::from("/music/song1.opus"),
            100,
            1000,
            "Thunderstruck",
            "AC/DC",
            Some("The Razors Edge"),
            292000,
            Some(1),
            Some(1),
            Some(1990),
            AudioFormat::Opus,
            Some(48000),
            Some(160000),
        );

        db.insert_track(
            PathBuf::from("/music/song2.opus"),
            101,
            1001,
            "Fire Your Guns",
            "AC/DC",
            Some("The Razors Edge"),
            173000,
            Some(2),
            Some(1),
            Some(1990),
            AudioFormat::Opus,
            Some(48000),
            Some(160000),
        );

        db.insert_track(
            PathBuf::from("/music/short_interlude.opus"),
            102,
            500,
            "Intro",
            "AC/DC",
            None,
            15000, // 15 seconds - below scrobble limit
            None,
            None,
            None,
            AudioFormat::Opus,
            Some(48000),
            Some(128000),
        );

        db
    }

    #[test]
    fn test_multi_field_search() {
        let db = build_test_db();
        let queries = LibraryQueries::new(&db);

        let res1 = queries.search("dc thunder");
        assert_eq!(res1.len(), 1);
        assert_eq!(res1[0].title.as_str(), "Thunderstruck");

        let res2 = queries.search("razors");
        assert_eq!(res2.len(), 2);

        let res3 = queries.search("");
        assert_eq!(res3.len(), 0);

        let res4 = queries.search("   ");
        assert_eq!(res4.len(), 0);
    }

    #[test]
    fn test_id_prefix_search() {
        let db = build_test_db();
        let queries = LibraryQueries::new(&db);

        let res = queries.search("id:1");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].id, TrackId(1));

        let res_track = queries.search("track:2");
        assert_eq!(res_track.len(), 1);
        assert_eq!(res_track[0].id, TrackId(2));

        let res_invalid = queries.search("id:9999");
        assert_eq!(res_invalid.len(), 0);
    }

    #[test]
    fn test_album_tracks_natural_sorting() {
        let db = build_test_db();
        let queries = LibraryQueries::new(&db);

        let album_data = queries.get_album_with_tracks(AlbumId(1)).unwrap();
        assert_eq!(album_data.tracks.len(), 2);
        assert_eq!(album_data.tracks[0].track_number, Some(1));
        assert_eq!(album_data.tracks[1].track_number, Some(2));
    }

    #[test]
    fn test_scrobble_payload_threshold() {
        let db = build_test_db();
        let queries = LibraryQueries::new(&db);

        // 292s track passes scrobble requirement
        let payload = queries.get_scrobble_payload(TrackId(1)).unwrap();
        assert_eq!(payload.track_title, "Thunderstruck");
        assert_eq!(payload.artist_name, "AC/DC");
        assert_eq!(payload.duration_seconds, 292);

        // 15s track must return None (rejected for scrobbling)
        let short_payload = queries.get_scrobble_payload(TrackId(3));
        assert!(short_payload.is_none());
    }

    #[test]
    fn test_artist_and_album_sorting() {
        let db = build_test_db();
        let queries = LibraryQueries::new(&db);

        let artists = queries.get_all_artists(ArtistSortBy::Name, SortDirection::Ascending);
        assert_eq!(artists.len(), 1);
        assert_eq!(artists[0].name.as_str(), "AC/DC");

        let albums = queries.get_all_albums(AlbumSortBy::Year, SortDirection::Descending);
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].year, Some(1990));
    }

    #[test]
    fn test_pagination() {
        let items = vec![1, 2, 3, 4, 5, 6, 7];
        assert_eq!(LibraryQueries::paginate(&items, 0, 3), vec![1, 2, 3]);
        assert_eq!(LibraryQueries::paginate(&items, 1, 3), vec![4, 5, 6]);
        assert_eq!(LibraryQueries::paginate(&items, 2, 3), vec![7]);
        assert_eq!(LibraryQueries::paginate(&items, 3, 3), Vec::<i32>::new());
    }

    #[test]
    fn test_formatted_display_helpers() {
        let db = build_test_db();
        let queries = LibraryQueries::new(&db);

        let track = queries.search("id:1").remove(0);
        assert_eq!(track.formatted_duration(), "4:52");
        assert_eq!(track.formatted_track_number(), "01");

        let track_no_meta = queries.search("id:3").remove(0);
        assert_eq!(track_no_meta.formatted_track_number(), "--");
    }
}