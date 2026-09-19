use crate::library::database::LibraryDatabase;
use crate::library::types::*;

#[derive(Debug, Clone)]
pub struct AlbumWithTracks {
    pub album: Album,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone)]
pub struct ArtistWithAlbums {
    pub artist: Artist,
    pub albums: Vec<AlbumWithTracks>,
    pub standalone_tracks: Vec<Track>,
}

pub struct LibraryQueries<'a> {
    db: &'a LibraryDatabase,
}

impl<'a> LibraryQueries<'a> {
    pub fn new(db: &'a LibraryDatabase) -> Self {
        Self { db }
    }

    pub fn get_album_with_tracks(&self, album_id: AlbumId) -> Option<AlbumWithTracks> {
        let album = self.db.get_album(album_id)?;
        let mut tracks = Vec::with_capacity(album.tracks.len());

        for track_id in &album.tracks {
            if let Some(track) = self.db.get_track(*track_id) {
                tracks.push(track);
            }
        }

        tracks.sort_by(|a, b| {
            a.disc_number
                .unwrap_or(1)
                .cmp(&b.disc_number.unwrap_or(1))
                .then_with(|| a.track_number.unwrap_or(0).cmp(&b.track_number.unwrap_or(0)))
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

        albums.sort_by(|a, b| b.album.year.unwrap_or(0).cmp(&a.album.year.unwrap_or(0)));

        let mut standalone_tracks = Vec::with_capacity(artist.standalone_tracks.len());
        for track_id in &artist.standalone_tracks {
            if let Some(track) = self.db.get_track(*track_id) {
                standalone_tracks.push(track);
            }
        }

        Some(ArtistWithAlbums {
            artist,
            albums,
            standalone_tracks,
        })
    }

    pub fn search_tracks(&self, query: &str) -> Vec<Track> {
        let query_lower = query.to_lowercase();
        let mut matches = Vec::new();

        for track_id in 1..=self.db.track_count() as u32 {
            if let Some(track) = self.db.get_track(TrackId(track_id)) {
                if track.title.to_lowercase().contains(&query_lower) {
                    matches.push(track);
                }
            }
        }

        matches
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_queries_album_ordering() {
        let db = LibraryDatabase::new();

        db.insert_track(
            PathBuf::from("/music/t2.opus"),
            100,
            1000,
            "Track Two",
            "Artist",
            Some("Album"),
            200000,
            Some(2),
            Some(1),
            Some(2024),
            AudioFormat::Opus,
            Some(48000),
            Some(128000),
        );

        db.insert_track(
            PathBuf::from("/music/t1.opus"),
            100,
            1000,
            "Track One",
            "Artist",
            Some("Album"),
            180000,
            Some(1),
            Some(1),
            Some(2024),
            AudioFormat::Opus,
            Some(48000),
            Some(128000),
        );

        let queries = LibraryQueries::new(&db);
        let album_data = queries.get_album_with_tracks(AlbumId(1)).unwrap();

        assert_eq!(album_data.tracks.len(), 2);
        assert_eq!(album_data.tracks[0].track_number, Some(1));
        assert_eq!(album_data.tracks[1].track_number, Some(2));
    }

    #[test]
    fn test_queries_search() {
        let db = LibraryDatabase::new();

        db.insert_track(
            PathBuf::from("/music/a.opus"),
            100,
            1000,
            "Solar Flare",
            "Artist",
            None,
            150000,
            None,
            None,
            None,
            AudioFormat::Opus,
            None,
            None,
        );

        let queries = LibraryQueries::new(&db);
        let results = queries.search_tracks("solar");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title.as_str(), "Solar Flare");
    }
}