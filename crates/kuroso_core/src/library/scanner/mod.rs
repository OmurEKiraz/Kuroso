use crate::library::database::LibraryDatabase;
use crate::library::types::AudioFormat;
use jwalk::WalkDir;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::probe::Probe;
use lofty::tag::Accessor;
use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const SUPPORTED_EXTENSIONS: &[&str] = &["opus", "ogg", "flac", "mp3", "m4a", "aac", "wav"];

pub struct ExtractedMetadata {
    pub path: PathBuf,
    pub mtime: u64,
    pub file_size: u64,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_ms: u32,
    pub track_number: Option<u16>,
    pub disc_number: Option<u8>,
    pub year: Option<u16>,
    pub format: AudioFormat,
    pub sample_rate: Option<u32>,
    pub bitrate: Option<u32>,
}

pub fn detect_audio_format(path: &Path) -> AudioFormat {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => match ext.to_ascii_lowercase().as_str() {
            "opus" => AudioFormat::Opus,
            "ogg" => AudioFormat::Vorbis,
            "flac" => AudioFormat::Flac,
            "mp3" => AudioFormat::Mp3,
            "m4a" | "aac" => AudioFormat::Aac,
            "wav" => AudioFormat::Wav,
            _ => AudioFormat::Unknown,
        },
        None => AudioFormat::Unknown,
    }
}

pub fn read_metadata(path: &Path) -> Option<ExtractedMetadata> {
    let metadata = fs::metadata(path).ok()?;
    let file_size = metadata.len();
    let mtime = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs();

    let tagged_file = Probe::open(path).ok()?.read().ok()?;
    let properties = tagged_file.properties();
    let tag = tagged_file.primary_tag().or_else(|| tagged_file.first_tag());

    let title = tag
        .and_then(|t| t.title())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown Title")
                .to_string()
        });

    let artist = tag
        .and_then(|t| t.artist())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Unknown Artist".to_string());

    let album = tag
        .and_then(|t| t.album())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let duration_ms = properties.duration().as_millis() as u32;
    let track_number = tag.and_then(|t| t.track()).map(|n| n as u16);
    let disc_number = tag.and_then(|t| t.disk()).map(|n| n as u8);
    let year = tag.and_then(|t| t.year()).map(|y| y as u16);
    let format = detect_audio_format(path);

    Some(ExtractedMetadata {
        path: path.to_path_buf(),
        mtime,
        file_size,
        title,
        artist,
        album,
        duration_ms,
        track_number,
        disc_number,
        year,
        format,
        sample_rate: properties.sample_rate(),
        bitrate: properties.audio_bitrate(),
    })
}

pub fn scan_directory<P: AsRef<Path>>(root: P, db: &LibraryDatabase) -> usize {
    let audio_files: Vec<PathBuf> = WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| {
            if !entry.file_type().is_file() {
                return false;
            }
            entry
                .path()
                .extension()
                .and_then(|s| s.to_str())
                .map(|ext| SUPPORTED_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
                .unwrap_or(false)
        })
        .map(|entry| entry.path())
        .collect();

    let extracted: Vec<ExtractedMetadata> = audio_files
        .par_iter()
        .filter_map(|path| read_metadata(path))
        .collect();

    let total = extracted.len();

    for meta in extracted {
        db.insert_track(
            meta.path,
            meta.mtime,
            meta.file_size,
            &meta.title,
            &meta.artist,
            meta.album.as_deref(),
            meta.duration_ms,
            meta.track_number,
            meta.disc_number,
            meta.year,
            meta.format,
            meta.sample_rate,
            meta.bitrate,
        );
    }

    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_audio_format() {
        assert_eq!(detect_audio_format(Path::new("song.opus")), AudioFormat::Opus);
        assert_eq!(detect_audio_format(Path::new("track.flac")), AudioFormat::Flac);
        assert_eq!(detect_audio_format(Path::new("audio.mp3")), AudioFormat::Mp3);
        assert_eq!(detect_audio_format(Path::new("file.ogg")), AudioFormat::Vorbis);
        assert_eq!(detect_audio_format(Path::new("m4a_track.m4a")), AudioFormat::Aac);
        assert_eq!(detect_audio_format(Path::new("sound.wav")), AudioFormat::Wav);
        assert_eq!(detect_audio_format(Path::new("file.unknown")), AudioFormat::Unknown);
    }
}