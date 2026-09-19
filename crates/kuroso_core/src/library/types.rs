use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TrackId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AlbumId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ArtistId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AudioFormat {
    Opus,
    Vorbis,
    Flac,
    Mp3,
    Aac,
    Wav,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub path: PathBuf,
    pub mtime: u64,
    pub file_size: u64,
    pub title: SmolStr,
    pub artist_id: ArtistId,
    pub album_id: Option<AlbumId>,
    pub duration_ms: u32,
    pub track_number: Option<u16>,
    pub disc_number: Option<u8>,
    pub year: Option<u16>,
    pub format: AudioFormat,
    pub sample_rate: Option<u32>,
    pub bitrate: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Album {
    pub id: AlbumId,
    pub title: SmolStr,
    pub artist_id: ArtistId,
    pub year: Option<u16>,
    pub tracks: Vec<TrackId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artist {
    pub id: ArtistId,
    pub name: SmolStr,
    pub albums: Vec<AlbumId>,
    pub standalone_tracks: Vec<TrackId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_id_hash_and_equality() {
        let mut set = HashSet::new();
        set.insert(TrackId(1));
        set.insert(TrackId(2));
        set.insert(TrackId(1));

        assert_eq!(set.len(), 2);
        assert!(set.contains(&TrackId(1)));
        assert!(set.contains(&TrackId(2)));
    }

    #[test]
    fn test_track_serialization_roundtrip() {
        let track = Track {
            id: TrackId(42),
            path: PathBuf::from("/music/sample.opus"),
            mtime: 1710000000,
            file_size: 1048576,
            title: SmolStr::new("Kinetic Drift"),
            artist_id: ArtistId(1),
            album_id: Some(AlbumId(10)),
            duration_ms: 184500,
            track_number: Some(1),
            disc_number: Some(1),
            year: Some(2024),
            format: AudioFormat::Opus,
            sample_rate: Some(48000),
            bitrate: Some(128000),
        };

        let json = serde_json::to_string(&track).expect("Serialization failed");
        let deserialized: Track = serde_json::from_str(&json).expect("Deserialization failed");

        assert_eq!(track, deserialized);
    }

    #[test]
    fn test_relational_integrity_representations() {
        let artist_id = ArtistId(1);
        let album_id = AlbumId(1);
        let track_id = TrackId(100);

        let artist = Artist {
            id: artist_id,
            name: SmolStr::new("Kuroso Sound"),
            albums: vec![album_id],
            standalone_tracks: vec![],
        };

        let album = Album {
            id: album_id,
            title: SmolStr::new("Zero Overhead"),
            artist_id,
            year: Some(2025),
            tracks: vec![track_id],
        };

        assert_eq!(artist.albums[0], album.id);
        assert_eq!(album.tracks[0], track_id);
    }
}