use super::*;
use crate::domain::models::*;
use crate::domain::repositories::*;
use crate::infrastructure::database::Database;
use std::sync::Arc;
use uuid::Uuid;

#[test]
fn test_escape_like_pattern_treats_wildcards_literally() {
    assert_eq!(escape_like_pattern("plain"), "plain");
    assert_eq!(escape_like_pattern("50%"), "50\\%");
    assert_eq!(escape_like_pattern("a_b"), "a\\_b");
    assert_eq!(escape_like_pattern("back\\slash"), "back\\\\slash");
    assert_eq!(escape_like_pattern("%_%"), "\\%\\_\\%");
}

#[tokio::test]
async fn test_search_wildcards_match_literally() {
    // Behaviour change, asserted as literal interpretation: a user typing
    // `%` means the character, not "match anything".
    let db_path =
        std::env::temp_dir().join(format!("test_auralis_repo_like_{}.db", Uuid::new_v4()));
    let db = Database::new(&db_path).unwrap();
    db.run_migrations().unwrap();
    let repo = SqliteTrackRepository::new(Arc::new(db));

    let literal = Track::new(
        "50% off".to_string(),
        "/path/to/percent.mp3".to_string(),
        200,
        AudioFormat::Mp3,
    );
    repo.insert(&literal).await.unwrap();
    let other = Track::new(
        "50X off".to_string(),
        "/path/to/other.mp3".to_string(),
        200,
        AudioFormat::Mp3,
    );
    repo.insert(&other).await.unwrap();

    let filter = TrackFilter {
        search: Some("50%".to_string()),
        ..Default::default()
    };
    let found = repo.find_all(filter).await.unwrap();
    assert_eq!(found.len(), 1, "only the literal `%` title must match");
    assert_eq!(found[0].title, "50% off");

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_track_repository_crud() {
    let db_path = std::env::temp_dir().join(format!("test_auralis_repo_{}.db", Uuid::new_v4()));
    let db = Database::new(&db_path).unwrap();
    db.run_migrations().unwrap();
    let db_arc = Arc::new(db);
    let repo = SqliteTrackRepository::new(db_arc);

    let track = Track::new(
        "Test Favorite Track".to_string(),
        "/path/to/test.mp3".to_string(),
        200,
        AudioFormat::Mp3,
    );
    repo.insert(&track).await.unwrap();

    let track_id_str = track.id.to_string();
    repo.set_favorite(&track_id_str, true).await.unwrap();

    let found = repo.find_by_id(track.id).await.unwrap().unwrap();
    assert_eq!(found.title, "Test Favorite Track");

    repo.set_favorite(&track_id_str, false).await.unwrap();

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_track_repository_find_by_paths() {
    let db_path = std::env::temp_dir().join(format!(
        "test_auralis_repo_find_by_paths_{}.db",
        Uuid::new_v4()
    ));
    let db = Database::new(&db_path).unwrap();
    db.run_migrations().unwrap();
    let db_arc = Arc::new(db);
    let repo = SqliteTrackRepository::new(db_arc);

    let empty = repo.find_by_paths(&[]).await.unwrap();
    assert!(empty.is_empty());

    let track1 = Track::new(
        "Track 1".to_string(),
        "/path/to/test_1.mp3".to_string(),
        120,
        AudioFormat::Mp3,
    );
    let track2 = Track::new(
        "Track 2".to_string(),
        "/path/to/test_2.mp3".to_string(),
        180,
        AudioFormat::Mp3,
    );
    repo.insert(&track1).await.unwrap();
    repo.insert(&track2).await.unwrap();

    let paths = vec![
        "/path/to/test_2.mp3",
        "/path/to/nonexistent.mp3",
        "/path/to/test_1.mp3",
    ];
    let found = repo.find_by_paths(&paths).await.unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].file_path, "/path/to/test_2.mp3");
    assert_eq!(found[1].file_path, "/path/to/test_1.mp3");

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_track_repository_limit_offset() {
    let db_path = std::env::temp_dir().join(format!(
        "test_auralis_repo_pagination_{}.db",
        Uuid::new_v4()
    ));
    let db = Database::new(&db_path).unwrap();
    db.run_migrations().unwrap();
    let db_arc = Arc::new(db);
    let repo = SqliteTrackRepository::new(db_arc);

    for i in 1..=5 {
        let track = Track::new(
            format!("Track {:02}", i),
            format!("/path/to/test_{}.mp3", i),
            100 + i * 10,
            AudioFormat::Mp3,
        );
        repo.insert(&track).await.unwrap();
    }

    let filter = TrackFilter {
        sort_by: Some(TrackSortField::Title),
        sort_desc: false,
        limit: Some(2),
        offset: Some(1),
        ..Default::default()
    };

    let tracks = repo.find_all(filter).await.unwrap();
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0].title, "Track 02");
    assert_eq!(tracks[1].title, "Track 03");

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_track_repository_delete_many() {
    let db_path = std::env::temp_dir().join(format!(
        "test_auralis_repo_delete_many_{}.db",
        Uuid::new_v4()
    ));
    let db = Database::new(&db_path).unwrap();
    db.run_migrations().unwrap();
    let db_arc = Arc::new(db);
    let repo = SqliteTrackRepository::new(db_arc);

    // Test empty deletion
    repo.delete_many(vec![]).await.unwrap();

    // Insert test tracks
    let mut track_ids = Vec::new();
    for i in 1..=5 {
        let track = Track::new(
            format!("Track {:02}", i),
            format!("/path/to/test_{}.mp3", i),
            100,
            AudioFormat::Mp3,
        );
        repo.insert(&track).await.unwrap();
        track_ids.push(track.id);
    }

    assert_eq!(repo.count().await.unwrap(), 5);

    // Delete subset of tracks (tracks 0, 1)
    repo.delete_many(vec![track_ids[0], track_ids[1]])
        .await
        .unwrap();

    assert_eq!(repo.count().await.unwrap(), 3);
    assert!(repo.find_by_id(track_ids[0]).await.unwrap().is_none());
    assert!(repo.find_by_id(track_ids[1]).await.unwrap().is_none());
    assert!(repo.find_by_id(track_ids[2]).await.unwrap().is_some());

    // Delete remaining tracks
    repo.delete_many(vec![track_ids[2], track_ids[3], track_ids[4]])
        .await
        .unwrap();
    assert_eq!(repo.count().await.unwrap(), 0);

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_get_albums_and_artists_summary() {
    let db_path = std::env::temp_dir().join(format!(
        "test_auralis_repo_aggregations_{}.db",
        Uuid::new_v4()
    ));
    let db = Database::new(&db_path).unwrap();
    db.run_migrations().unwrap();
    let db_arc = Arc::new(db);
    let repo = SqliteTrackRepository::new(db_arc);

    let mut t1 = Track::new(
        "Song 1".to_string(),
        "/path/1.mp3".to_string(),
        180,
        AudioFormat::Mp3,
    );
    t1.album = Some("Album Alpha".to_string());
    t1.artist = Some("Artist Beta".to_string());
    t1.album_art_path = Some("/art/alpha.jpg".to_string());

    let mut t2 = Track::new(
        "Song 2".to_string(),
        "/path/2.mp3".to_string(),
        200,
        AudioFormat::Mp3,
    );
    t2.album = Some("Album Alpha".to_string());
    t2.artist = Some("Artist Beta".to_string());
    t2.album_art_path = Some("/art/alpha.jpg".to_string());

    let mut t3 = Track::new(
        "Song 3".to_string(),
        "/path/3.mp3".to_string(),
        240,
        AudioFormat::Mp3,
    );
    t3.album = Some("Album Gamma".to_string());
    t3.artist = Some("Artist Delta".to_string());

    repo.insert(&t1).await.unwrap();
    repo.insert(&t2).await.unwrap();
    repo.insert(&t3).await.unwrap();

    let albums = repo.get_albums_summary().await.unwrap();
    assert_eq!(albums.len(), 2);
    assert_eq!(albums[0].album, "Album Alpha");
    assert_eq!(albums[0].track_count, 2);
    assert_eq!(albums[0].artist.as_deref(), Some("Artist Beta"));
    assert_eq!(albums[1].album, "Album Gamma");
    assert_eq!(albums[1].track_count, 1);

    let artists = repo.get_artists_summary().await.unwrap();
    assert_eq!(artists.len(), 2);
    assert_eq!(artists[0].artist, "Artist Beta");
    assert_eq!(artists[0].track_count, 2);
    assert_eq!(artists[1].artist, "Artist Delta");
    assert_eq!(artists[1].track_count, 1);

    let _ = std::fs::remove_file(&db_path);
}
