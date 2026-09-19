use kuroso_core::library::database::LibraryDatabase;
use kuroso_core::library::queries::{ArtistSortBy, LibraryQueries, SortDirection};
use kuroso_core::library::types::*;
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

fn create_populated_db() -> (LibraryDatabase, [TrackId; 4]) {
    let db = LibraryDatabase::new();

    let p1 = PathBuf::from("/music/camellia/crystallized/01_crystallized.opus");
    let p2 = PathBuf::from("/music/camellia/crystallized/02_first_town.opus");
    let p3 = PathBuf::from("/music/camellia/singles/spin_eternally.opus");
    let p4 = PathBuf::from("/music/taku_inoue/aliens.flac");

    let t1 = db.insert_track(
        p1,
        1700000000,
        15_000_000,
        "Crystallized",
        "Camellia",
        None,
        Some("Crystallized"),
        284_000,
        Some(1),
        Some(1),
        Some(2015),
        AudioFormat::Opus,
        Some(48_000),
        Some(160_000),
        Some(16),
        Some(2),
        Some(-6.0),
        Some(0.95),
        Some(-5.5),
        Some(0.98),
    );

    let t2 = db.insert_track(
        p2,
        1700000001,
        12_000_000,
        "First Town",
        "Camellia",
        None,
        Some("Crystallized"),
        210_000,
        Some(2),
        Some(1),
        Some(2015),
        AudioFormat::Opus,
        Some(48_000),
        Some(160_000),
        Some(16),
        Some(2),
        Some(-6.2),
        Some(0.94),
        Some(-5.5),
        Some(0.98),
    );

    let t3 = db.insert_track(
        p3,
        1700000002,
        18_000_000,
        "Spin Eternally",
        "Camellia",
        None,
        None,
        290_000,
        None,
        None,
        Some(2021),
        AudioFormat::Opus,
        Some(48_000),
        Some(192_000),
        Some(16),
        Some(2),
        Some(-7.0),
        Some(0.99),
        None,
        None,
    );

    let t4 = db.insert_track(
        p4,
        1700000003,
        35_000_000,
        "Aliens",
        "Taku Inoue",
        None,
        None,
        245_000,
        None,
        None,
        Some(2022),
        AudioFormat::Flac,
        Some(96_000),
        Some(900_000),
        Some(24),
        Some(2),
        Some(-5.0),
        Some(0.92),
        None,
        None,
    );

    (db, [t1, t2, t3, t4])
}

#[test]
fn test_full_library_lifecycle() {
    let (db, [t1, t2, t3, t4]) = create_populated_db();

    assert_eq!(db.track_count(), 4);
    assert_eq!(db.album_count(), 1);
    assert_eq!(db.artist_count(), 2);

    let queries = LibraryQueries::new(&db);

    let camellia_artist_id = db.get_track(t1).unwrap().artist_id;
    let camellia_tree = queries
        .get_artist_with_albums(camellia_artist_id)
        .expect("Artist Camellia must exist");

    assert_eq!(camellia_tree.artist.name.as_str(), "Camellia");
    assert_eq!(camellia_tree.albums.len(), 1);
    assert_eq!(camellia_tree.standalone_tracks.len(), 1);
    assert_eq!(camellia_tree.standalone_tracks[0].id, t3);

    let crystallized_album = &camellia_tree.albums[0];
    assert_eq!(crystallized_album.album.title.as_str(), "Crystallized");
    assert_eq!(crystallized_album.tracks.len(), 2);
    assert_eq!(crystallized_album.tracks[0].id, t1);
    assert_eq!(crystallized_album.tracks[1].id, t2);

    let search_res = queries.search_tracks("spin");
    assert_eq!(search_res.len(), 1);
    assert_eq!(search_res[0].id, t3);

    let p4 = PathBuf::from("/music/taku_inoue/aliens.flac");
    let resolved_track = db.get_track_by_path(&p4).expect("Lookup by path failed");
    assert_eq!(resolved_track.id, t4);

    let removed = db.remove_track_by_path(&p4);
    assert_eq!(removed, Some(t4));
    assert_eq!(db.track_count(), 3);
    assert_eq!(db.artist_count(), 1);
    assert!(db.get_track_by_path(&p4).is_none());

    let p1 = PathBuf::from("/music/camellia/crystallized/01_crystallized.opus");
    let p2 = PathBuf::from("/music/camellia/crystallized/02_first_town.opus");
    let p3 = PathBuf::from("/music/camellia/singles/spin_eternally.opus");

    db.remove_track_by_path(&p1);
    db.remove_track_by_path(&p2);

    assert_eq!(db.album_count(), 0);
    assert_eq!(db.artist_count(), 1);

    db.remove_track_by_path(&p3);

    assert_eq!(db.track_count(), 0);
    assert_eq!(db.album_count(), 0);
    assert_eq!(db.artist_count(), 0);
}

#[test]
fn test_metadata_mutation_and_relational_cascades() {
    let (db, [t1, _, _, _]) = create_populated_db();
    let p1 = PathBuf::from("/music/camellia/crystallized/01_crystallized.opus");

    let updated_id = db.update_track_metadata(
        &p1,
        1700000010,
        15_000_100,
        "Crystallized (VIP)",
        "Camellia feat. Hatsune Miku",
        Some("Camellia"),
        Some("Heart of Android"),
        310_000,
        Some(5),
        Some(1),
        Some(2016),
        Some(48_000),
        Some(320_000),
        Some(16),
        Some(2),
        Some(-6.0),
        Some(0.95),
        Some(-5.0),
        Some(0.98),
    );

    assert_eq!(updated_id, Some(t1));

    let track = db.get_track(t1).expect("Track must exist");
    assert_eq!(track.title.as_str(), "Crystallized (VIP)");
    assert_eq!(track.track_number, Some(5));
    assert_eq!(track.mtime, 1700000010);
    assert_eq!(track.bit_depth, Some(16));
    assert_eq!(track.channels, Some(2));

    let queries = LibraryQueries::new(&db);
    let view = queries.track_to_view(&track).expect("View conversion failed");
    assert_eq!(view.artist_name.as_str(), "Camellia feat. Hatsune Miku");
    assert_eq!(view.album_artist_name.as_deref(), Some("Camellia"));
    assert_eq!(view.album_title.as_deref(), Some("Heart of Android"));
    assert_eq!(view.formatted_duration(), "5:10");
    assert_eq!(view.formatted_track_number(), "05");

    let old_album = queries.get_album_with_tracks(AlbumId(1)).unwrap();
    assert_eq!(old_album.tracks.len(), 1);
    assert_eq!(old_album.tracks[0].title.as_str(), "First Town");
}

#[test]
fn test_dead_file_pruning() {
    let (db, [t1, t2, _, _]) = create_populated_db();
    assert_eq!(db.track_count(), 4);

    let mut live_paths = HashSet::new();
    live_paths.insert(PathBuf::from("/music/camellia/crystallized/01_crystallized.opus"));
    live_paths.insert(PathBuf::from("/music/camellia/crystallized/02_first_town.opus"));

    let pruned_count = db.prune_missing_files(&live_paths);
    assert_eq!(pruned_count, 2);
    assert_eq!(db.track_count(), 2);
    assert!(db.get_track(t1).is_some());
    assert!(db.get_track(t2).is_some());
}

#[test]
fn test_diff_rescan_validator() {
    let (db, _) = create_populated_db();
    let p = PathBuf::from("/music/camellia/crystallized/01_crystallized.opus");

    assert!(!db.should_rescan(&p, 1700000000, 15_000_000));
    assert!(db.should_rescan(&p, 1700000005, 15_000_000));
    assert!(db.should_rescan(&p, 1700000000, 16_000_000));
    assert!(db.should_rescan(&PathBuf::from("/music/new_song.flac"), 100, 100));
}

#[test]
fn test_persistence_atomic_roundtrip_and_corruption_resilience() {
    let (db, _) = create_populated_db();
    let cache_path = std::env::temp_dir().join("kuroso_integration_cache_test.bin");
    let _ = fs::remove_file(&cache_path);

    db.save_to_file(&cache_path).expect("Atomic save failed");
    assert!(cache_path.exists());

    let (loaded_db, is_hit) = LibraryDatabase::load_or_recover(&cache_path);
    assert!(is_hit);
    assert_eq!(loaded_db.track_count(), 4);
    assert_eq!(loaded_db.album_count(), 1);
    assert_eq!(loaded_db.artist_count(), 2);

    {
        let mut f = OpenOptions::new().write(true).open(&cache_path).unwrap();
        f.write_all(b"KUROSO\0\x01\x01\0\0\0CORRUPTED_INLINE_PAYLOAD_TEST").unwrap();
    }

    let (recovered_db, hit) = LibraryDatabase::load_or_recover(&cache_path);
    assert!(!hit);
    assert_eq!(recovered_db.track_count(), 0);
    assert!(!cache_path.exists());

    let _ = fs::remove_file(cache_path);
}

#[test]
fn test_search_and_id_syntax() {
    let (db, [t1, t2, _, _]) = create_populated_db();
    let queries = LibraryQueries::new(&db);

    let res_album = queries.search("camellia crystal");
    assert_eq!(res_album.len(), 2);

    let res_specific = queries.search("camellia town");
    assert_eq!(res_specific.len(), 1);
    assert_eq!(res_specific[0].id, t2);

    let res_case = queries.search("tAkU");
    assert_eq!(res_case.len(), 1);
    assert_eq!(res_case[0].title.as_str(), "Aliens");

    let res_id = queries.search("id:1");
    assert_eq!(res_id.len(), 1);
    assert_eq!(res_id[0].id, t1);

    let res_track = queries.search("track:1");
    assert_eq!(res_track.len(), 1);
    assert_eq!(res_track[0].id, t1);

    let all_res = queries.search_all("camellia");
    assert_eq!(all_res.tracks.len(), 3);
    assert_eq!(all_res.artists.len(), 1);
    assert_eq!(all_res.artists[0].name.as_str(), "Camellia");

    assert!(queries.search("").is_empty());
    assert!(queries.search("     ").is_empty());
}

#[test]
fn test_scrobble_criteria() {
    let db = LibraryDatabase::new();

    let t_valid = db.insert_track(
        PathBuf::from("/music/valid.opus"),
        100,
        1000,
        "Valid Song",
        "Artist",
        None,
        None,
        200_000,
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

    let t_short = db.insert_track(
        PathBuf::from("/music/short.opus"),
        101,
        1000,
        "Short Jingle",
        "Artist",
        None,
        None,
        18_000,
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

    let queries = LibraryQueries::new(&db);
    let payload = queries.get_scrobble_payload(t_valid);
    assert!(payload.is_some());
    let p = payload.unwrap();
    assert_eq!(p.duration_seconds, 200);

    let short_payload = queries.get_scrobble_payload(t_short);
    assert!(short_payload.is_none());
}

#[test]
fn test_engine_multi_root_lifecycle() {
    let temp_base = std::env::temp_dir().join("kuroso_test_engine_multi");
    let dir_a = temp_base.join("music_a");
    let dir_b = temp_base.join("music_b");
    let cache = temp_base.join("engine_cache.bin");

    let _ = fs::create_dir_all(&dir_a);
    let _ = fs::create_dir_all(&dir_b);
    let _ = fs::remove_file(&cache);

    // 1. Open engine with single root
    let engine = kuroso_core::library::LibraryEngine::open(&dir_a, &cache)
        .expect("Engine failed to open");

    assert_eq!(engine.tracked_roots().len(), 1);
    assert_eq!(engine.tracked_roots()[0], dir_a);

    // 2. Add a second root dynamically
    engine.add_root(&dir_b).expect("Failed to add root");
    assert_eq!(engine.tracked_roots().len(), 2);
    assert!(engine.tracked_roots().contains(&dir_b));

    // 3. Remove a root dynamically
    engine.remove_root(&dir_a).expect("Failed to remove root");
    assert_eq!(engine.tracked_roots().len(), 1);
    assert_eq!(engine.tracked_roots()[0], dir_b);

    let _ = fs::remove_dir_all(temp_base);
}

#[test]
fn test_pagination_and_sorting() {
    let (db, _) = create_populated_db();
    let queries = LibraryQueries::new(&db);

    let artists_asc = queries.get_all_artists(ArtistSortBy::Name, SortDirection::Ascending);
    assert_eq!(artists_asc[0].name.as_str(), "Camellia");
    assert_eq!(artists_asc[1].name.as_str(), "Taku Inoue");

    let artists_desc = queries.get_all_artists(ArtistSortBy::Name, SortDirection::Descending);
    assert_eq!(artists_desc[0].name.as_str(), "Taku Inoue");
    assert_eq!(artists_desc[1].name.as_str(), "Camellia");

    let all_tracks = queries.search("camellia");
    assert_eq!(all_tracks.len(), 3);

    let page_1 = LibraryQueries::paginate(&all_tracks, 0, 2);
    assert_eq!(page_1.len(), 2);

    let page_2 = LibraryQueries::paginate(&all_tracks, 1, 2);
    assert_eq!(page_2.len(), 1);

    let page_empty = LibraryQueries::paginate(&all_tracks, 2, 2);
    assert!(page_empty.is_empty());
}