use kuroso_core::library::database::LibraryDatabase;
use kuroso_core::library::queries::LibraryQueries;
use kuroso_core::library::types::*;
use std::path::PathBuf;

#[test]
fn test_full_library_lifecycle() {
    let db = LibraryDatabase::new();

    let p1 = PathBuf::from("/music/camellia/crystallized/01_crystallized.opus");
    let p2 = PathBuf::from("/music/camellia/crystallized/02_first_town.opus");
    let p3 = PathBuf::from("/music/camellia/singles/spin_eternally.opus");
    let p4 = PathBuf::from("/music/taku_inouse/aliens.flac");

    let t1 = db.insert_track(
        p1.clone(),
        1700000000,
        15_000_000,
        "Crystallized",
        "Camellia",
        Some("Crystallized"),
        284_000,
        Some(1),
        Some(1),
        Some(2015),
        AudioFormat::Opus,
        Some(48_000),
        Some(160_000),
    );

    let t2 = db.insert_track(
        p2.clone(),
        1700000001,
        12_000_000,
        "First Town",
        "Camellia",
        Some("Crystallized"),
        210_000,
        Some(2),
        Some(1),
        Some(2015),
        AudioFormat::Opus,
        Some(48_000),
        Some(160_000),
    );

    let t3 = db.insert_track(
        p3.clone(),
        1700000002,
        18_000_000,
        "Spin Eternally",
        "Camellia",
        None,
        290_000,
        None,
        None,
        Some(2021),
        AudioFormat::Opus,
        Some(48_000),
        Some(192_000),
    );

    let t4 = db.insert_track(
        p4.clone(),
        1700000003,
        35_000_000,
        "Aliens",
        "Taku Inoue",
        None,
        245_000,
        None,
        None,
        Some(2022),
        AudioFormat::Flac,
        Some(96_000),
        Some(900_000),
    );

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

    let resolved_track = db.get_track_by_path(&p4).expect("Lookup by path failed");
    assert_eq!(resolved_track.id, t4);

    let removed = db.remove_track_by_path(&p4);
    assert_eq!(removed, Some(t4));
    assert_eq!(db.track_count(), 3);
    assert_eq!(db.artist_count(), 1);
    assert!(db.get_track_by_path(&p4).is_none());

    db.remove_track_by_path(&p1);
    db.remove_track_by_path(&p2);

    assert_eq!(db.album_count(), 0);
    assert_eq!(db.artist_count(), 1);

    db.remove_track_by_path(&p3);

    assert_eq!(db.track_count(), 0);
    assert_eq!(db.album_count(), 0);
    assert_eq!(db.artist_count(), 0);
}