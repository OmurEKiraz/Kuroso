use crate::library::database::LibraryDatabase;
use crate::library::types::AudioFormat;
use jwalk::WalkDir;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey, Tag};
use rayon::prelude::*;
use std::collections::HashSet;
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

pub fn parse_number_field(val: &str) -> Option<u16> {
    let clean = val.trim();
    if clean.is_empty() {
        return None;
    }

    let num_str = clean.split(['/', '\\']).next()?.trim();
    num_str.parse::<u16>().ok()
}

pub fn parse_track_from_filename(filename: &str) -> Option<u16> {
    let stem = filename.trim();
    let digits: String = stem.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() {
        digits.parse::<u16>().ok()
    } else {
        None
    }
}

pub fn extract_track_number(tag: &Tag) -> Option<u16> {
    if let Some(t) = tag.track() {
        return Some(t as u16);
    }

    if let Some(item) = tag.get(&ItemKey::TrackNumber) {
        if let Some(text) = item.value().text() {
            if let Some(parsed) = parse_number_field(text) {
                return Some(parsed);
            }
        }
    }

    for item in tag.items() {
        let is_track_key = match item.key() {
            ItemKey::TrackNumber => true,
            ItemKey::Unknown(k) => {
                let lower = k.to_ascii_lowercase();
                lower == "tracknumber" || lower == "track" || lower == "track_number"
            }
            _ => false,
        };

        if is_track_key {
            if let Some(text) = item.value().text() {
                if let Some(parsed) = parse_number_field(text) {
                    return Some(parsed);
                }
            }
        }
    }

    None
}

pub fn extract_disc_number(tag: &Tag) -> Option<u8> {
    if let Some(d) = tag.disk() {
        return Some(d as u8);
    }

    if let Some(item) = tag.get(&ItemKey::DiscNumber) {
        if let Some(text) = item.value().text() {
            if let Some(parsed) = parse_number_field(text) {
                return Some(parsed as u8);
            }
        }
    }

    for item in tag.items() {
        let is_disc_key = match item.key() {
            ItemKey::DiscNumber => true,
            ItemKey::Unknown(k) => {
                let lower = k.to_ascii_lowercase();
                lower == "discnumber" || lower == "disc" || lower == "disk" || lower == "disctotal"
            }
            _ => false,
        };

        if is_disc_key {
            if let Some(text) = item.value().text() {
                if let Some(parsed) = parse_number_field(text) {
                    return Some(parsed as u8);
                }
            }
        }
    }

    None
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

    let track_number = tag
        .and_then(extract_track_number)
        .or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .and_then(parse_track_from_filename)
        });

    let disc_number = tag.and_then(extract_disc_number);
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

pub fn inspect_file_tags(path: &Path) {
    println!("Inspecting file: {:?}", path);
    if let Ok(tagged_file) = Probe::open(path).and_then(|p| p.read()) {
        if let Some(tag) = tagged_file.primary_tag().or_else(|| tagged_file.first_tag()) {
            println!("Tag Type: {:?}", tag.tag_type());
            println!("Items count: {}", tag.item_count());
            for item in tag.items() {
                println!("  Key: {:?} | Value: {:?}", item.key(), item.value());
            }
        } else {
            println!("No tags found in file!");
        }
    } else {
        println!("Failed to probe file on disk");
    }
}

pub struct ScanReport {
    pub scanned_files: usize,
    pub newly_added: usize,
    pub updated: usize,
    pub pruned: usize,
}

pub fn scan_directory<P: AsRef<Path>>(root: P, db: &LibraryDatabase) -> ScanReport {
    let mut live_paths = HashSet::new();

    let audio_files: Vec<(PathBuf, u64, u64)> = WalkDir::new(root)
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
        .filter_map(|entry| {
            let path = entry.path();
            let meta = fs::metadata(&path).ok()?;
            let file_size = meta.len();
            let mtime = meta
                .modified()
                .ok()?
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_secs();
            Some((path, mtime, file_size))
        })
        .collect();

    for (p, _, _) in &audio_files {
        live_paths.insert(p.clone());
    }

    let to_process: Vec<PathBuf> = audio_files
        .into_iter()
        .filter(|(path, mtime, size)| db.should_rescan(path, *mtime, *size))
        .map(|(path, _, _)| path)
        .collect();

    let extracted: Vec<ExtractedMetadata> = to_process
        .par_iter()
        .filter_map(|path| read_metadata(path))
        .collect();

    let mut newly_added = 0;
    let mut updated = 0;

    for meta in extracted {
        if db.get_track_by_path(&meta.path).is_some() {
            db.update_track_metadata(
                &meta.path,
                meta.mtime,
                meta.file_size,
                &meta.title,
                &meta.artist,
                meta.album.as_deref(),
                meta.duration_ms,
                meta.track_number,
                meta.disc_number,
                meta.year,
                meta.sample_rate,
                meta.bitrate,
            );
            updated += 1;
        } else {
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
            newly_added += 1;
        }
    }

    let pruned = db.prune_missing_files(&live_paths);

    ScanReport {
        scanned_files: live_paths.len(),
        newly_added,
        updated,
        pruned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lofty::tag::{ItemValue, Tag, TagItem, TagType};

    #[test]
    fn test_detect_audio_format() {
        assert_eq!(detect_audio_format(Path::new("song.opus")), AudioFormat::Opus);
        assert_eq!(detect_audio_format(Path::new("track.flac")), AudioFormat::Flac);
        assert_eq!(detect_audio_format(Path::new("audio.mp3")), AudioFormat::Mp3);
        assert_eq!(detect_audio_format(Path::new("test.m4a")), AudioFormat::Aac);
        assert_eq!(detect_audio_format(Path::new("test.wav")), AudioFormat::Wav);
        assert_eq!(detect_audio_format(Path::new("unknown.xyz")), AudioFormat::Unknown);
    }

    #[test]
    fn test_parse_number_field() {
        assert_eq!(parse_number_field("01"), Some(1));
        assert_eq!(parse_number_field(" 07 "), Some(7));
        assert_eq!(parse_number_field("12/20"), Some(12));
        assert_eq!(parse_number_field("3\\10"), Some(3));
        assert_eq!(parse_number_field(""), None);
        assert_eq!(parse_number_field("invalid"), None);
    }

    #[test]
    fn test_parse_track_from_filename() {
        assert_eq!(parse_track_from_filename("01 - Thunderstruck"), Some(1));
        assert_eq!(parse_track_from_filename("12_Hells_Bells"), Some(12));
        assert_eq!(parse_track_from_filename("03. TNT"), Some(3));
        assert_eq!(parse_track_from_filename("Meltdown"), None);
        assert_eq!(parse_track_from_filename(""), None);
    }

    #[test]
    fn test_extract_track_number_from_vorbis_custom_tag() {
        let mut tag = Tag::new(TagType::VorbisComments);
        tag.insert(TagItem::new(
            ItemKey::TrackNumber,
            ItemValue::Text("04/12".to_string()),
        ));

        let extracted = extract_track_number(&tag);
        assert_eq!(extracted, Some(4));
    }

    #[test]
    fn test_extract_disc_number_from_vorbis_custom_tag() {
        let mut tag = Tag::new(TagType::VorbisComments);
        tag.insert(TagItem::new(
            ItemKey::DiscNumber,
            ItemValue::Text("2/2".to_string()),
        ));

        let extracted = extract_disc_number(&tag);
        assert_eq!(extracted, Some(2));
    }
}