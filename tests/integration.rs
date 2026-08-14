//! Integration tests for the Auralis library.
//!
//! These exercise the public surface that the Tauri app and future
//! integrations rely on: the SQLite database module, the shared parse
//! helpers, and core domain model constructors.

use auralis_lib::domain::models::{DeviceType, PairedDevice};
use auralis_lib::infrastructure::database::repositories::{parse_datetime, parse_format};
use auralis_lib::infrastructure::database::Database;
use chrono::Datelike;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// Produce a unique on-disk database path inside the system temp dir so
/// concurrent test runs don't stomp on each other.
fn unique_db_path() -> std::path::PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let name = format!("auralis-test-{}-{}.db", std::process::id(), n);
    std::env::temp_dir().join(name)
}

#[test]
fn database_module_loads_and_migrates() {
    let path = unique_db_path();
    let _ = std::fs::remove_file(&path);

    let db = Database::new(&path).expect("database should open");
    db.run_migrations().expect("migrations should run");

    // A guarded connection must be obtainable after migrating.
    {
        let _conn = db.connection().expect("connection should be available");
    }

    // Clean up the temp file.
    drop(db);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn parse_format_round_trips_known_formats() {
    assert_eq!(
        parse_format("mp3"),
        auralis_lib::domain::models::AudioFormat::Mp3
    );
    assert_eq!(
        parse_format("FLAC"),
        auralis_lib::domain::models::AudioFormat::Flac
    );
    assert_eq!(
        parse_format("wav"),
        auralis_lib::domain::models::AudioFormat::Wav
    );
    // Unknown formats fall back to MP3 rather than erroring.
    assert_eq!(
        parse_format("bogus"),
        auralis_lib::domain::models::AudioFormat::Mp3
    );
}

#[test]
fn parse_datetime_accepts_rfc3339() {
    let dt = parse_datetime("2024-01-02T03:04:05Z");
    // Should parse to a fixed point (year 2024) without falling back to "now".
    assert_eq!(dt.year(), 2024);
    // Garbage input must not panic — it falls back to the current time.
    let _ = parse_datetime("not-a-date");
}

#[test]
fn paired_device_model_constructs() {
    let device = PairedDevice::new("Living Room Speaker".to_string(), DeviceType::Desktop);
    assert_eq!(device.name, "Living Room Speaker");
    assert!(!device.id.is_nil());
}

#[test]
fn pairing_info_generates_unique_pins() {
    let a = auralis_lib::domain::models::PairingInfo::generate();
    let b = auralis_lib::domain::models::PairingInfo::generate();
    assert_ne!(a.pin, b.pin);
    assert!(!a.pin.is_empty());
}

#[test]
fn integration_harness_is_timed() {
    // Sanity check that the test runtime itself is functional.
    let start = Instant::now();
    std::thread::sleep(std::time::Duration::from_millis(1));
    assert!(start.elapsed().as_millis() >= 1);
}

fn create_realistic_mp3(path: &std::path::Path, title: &str, artist: &str, album: &str) {
    let mut data = Vec::new();

    // Build ID3v2.3 tag
    let mut tag_body = Vec::new();
    let mut add_frame = |tag_body: &mut Vec<u8>, frame_id: &[u8; 4], text: &str| {
        let text_bytes = text.as_bytes();
        let frame_size = (text_bytes.len() + 1) as u32;
        tag_body.extend_from_slice(frame_id);
        tag_body.extend_from_slice(&frame_size.to_be_bytes());
        tag_body.extend_from_slice(&[0x00, 0x00]); // flags
        tag_body.push(0x00); // ISO-8859-1 encoding
        tag_body.extend_from_slice(text_bytes);
    };

    add_frame(&mut tag_body, b"TIT2", title);
    add_frame(&mut tag_body, b"TPE1", artist);
    add_frame(&mut tag_body, b"TALB", album);

    // ID3v2 header (10 bytes)
    let tag_size = tag_body.len() as u32;
    let synchsafe_size = [
        ((tag_size >> 21) & 0x7F) as u8,
        ((tag_size >> 14) & 0x7F) as u8,
        ((tag_size >> 7) & 0x7F) as u8,
        (tag_size & 0x7F) as u8,
    ];
    data.extend_from_slice(b"ID3\x03\x00\x00");
    data.extend_from_slice(&synchsafe_size);
    data.extend_from_slice(&tag_body);

    // Append MPEG-1 Layer 3 audio frames (128 kbps, 44.1 kHz, Joint Stereo)
    // Frame length = 144 * 128000 / 44100 = 417 bytes.
    let frame_header = [0xFF, 0xFB, 0x90, 0x64];
    for _ in 0..10 {
        data.extend_from_slice(&frame_header);
        data.resize(data.len() + (417 - 4), 0x00);
    }

    std::fs::write(path, data).expect("write mp3 fixture");
}

fn create_realistic_wav(path: &std::path::Path) {
    let mut data = Vec::new();
    let num_samples = 44100u32; // 1 second
    let num_channels = 2u16;
    let bits_per_sample = 16u16;
    let sample_rate = 44100u32;
    let byte_rate = sample_rate * num_channels as u32 * (bits_per_sample as u32 / 8);
    let block_align = num_channels * (bits_per_sample / 8);
    let data_len = num_samples * block_align as u32;
    let riff_len = 36 + data_len;

    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(&riff_len.to_le_bytes());
    data.extend_from_slice(b"WAVEfmt ");
    data.extend_from_slice(&16u32.to_le_bytes()); // subchunk1 size
    data.extend_from_slice(&1u16.to_le_bytes()); // PCM
    data.extend_from_slice(&num_channels.to_le_bytes());
    data.extend_from_slice(&sample_rate.to_le_bytes());
    data.extend_from_slice(&byte_rate.to_le_bytes());
    data.extend_from_slice(&block_align.to_le_bytes());
    data.extend_from_slice(&bits_per_sample.to_le_bytes());
    data.extend_from_slice(b"data");
    data.extend_from_slice(&data_len.to_le_bytes());
    data.resize(data.len() + data_len as usize, 0);

    std::fs::write(path, data).expect("write wav fixture");
}

fn create_realistic_flac(path: &std::path::Path) {
    let mut data = Vec::new();
    data.extend_from_slice(b"fLaC");
    // STREAMINFO block header: last block flag (0x80) | type 0, length 34 (0x00, 0x00, 0x22)
    data.extend_from_slice(&[0x80, 0x00, 0x00, 0x22]);
    // min block size (2 bytes): 4096 = 0x1000
    data.extend_from_slice(&[0x10, 0x00]);
    // max block size (2 bytes): 4096 = 0x1000
    data.extend_from_slice(&[0x10, 0x00]);
    // min frame size (3 bytes): 0
    data.extend_from_slice(&[0x00, 0x00, 0x00]);
    // max frame size (3 bytes): 0
    data.extend_from_slice(&[0x00, 0x00, 0x00]);
    // sample rate (20 bits), channels (3 bits - 1), bps (5 bits - 1), total samples (36 bits)
    data.extend_from_slice(&[0x0A, 0xC4, 0x42, 0xF0, 0x00, 0x00, 0xAC, 0x44]);
    // 16 bytes md5:
    data.extend_from_slice(&[0u8; 16]);

    std::fs::write(path, data).expect("write flac fixture");
}

#[tokio::test]
async fn auto_scan_discovers_mp3_and_ingests_into_database() {
    use auralis_lib::domain::models::TrackFilter;
    use auralis_lib::domain::repositories::TrackRepository;
    use auralis_lib::infrastructure::database::repositories::SqliteTrackRepository;
    use auralis_lib::infrastructure::filesystem::scanner::DirectoryScanner;
    use std::sync::Arc;

    // 1. Create a unique temp directory structure
    let test_dir = std::env::temp_dir().join(format!("auralis-scan-test-{}", uuid::Uuid::new_v4()));
    let nested_dir = test_dir.join("subfolder");
    std::fs::create_dir_all(&nested_dir).expect("should create test dirs");

    // 2. Create sample audio files (.mp3, .flac, .wav) and non-audio (.txt)
    let mp3_file = test_dir.join("sample_track.mp3");
    let flac_file = nested_dir.join("nested_song.FLAC");
    let wav_file = nested_dir.join("sound.wav");
    let txt_file = test_dir.join("readme.txt");

    create_realistic_mp3(
        &mp3_file,
        "Auralis Test Title",
        "Auralis Artist",
        "Auralis Album",
    );
    create_realistic_flac(&flac_file);
    create_realistic_wav(&wav_file);
    std::fs::write(&txt_file, b"This is not a music file").unwrap();

    // 3. Scan using DirectoryScanner
    let scanner = DirectoryScanner::default_audio();
    let found_files = scanner.scan(&test_dir).await.expect("scan should succeed");

    assert_eq!(
        found_files.len(),
        3,
        "Scanner must discover exactly the 3 audio files"
    );
    assert!(
        found_files.contains(&mp3_file),
        "Must discover root .mp3 file"
    );
    assert!(
        found_files.contains(&flac_file),
        "Must discover nested .FLAC file (case-insensitive)"
    );
    assert!(
        found_files.contains(&wav_file),
        "Must discover nested .wav file"
    );
    assert!(
        !found_files.contains(&txt_file),
        "Must exclude non-audio .txt file"
    );

    // 4. Initialize SQLite Database & Repository
    let db_path = unique_db_path();
    let db = Database::new(&db_path).expect("database should open");
    db.run_migrations().expect("migrations should run");
    let repo: Arc<dyn TrackRepository> = Arc::new(SqliteTrackRepository::new(Arc::new(db)));

    // 5. Ingest tracks via scan_library_paths
    let summary = scanner
        .scan_library_paths(&[test_dir.clone()], repo.clone())
        .await
        .expect("library ingestion must succeed");

    assert_eq!(summary.tracks_added, 3);
    assert_eq!(summary.tracks_updated, 0);
    assert_eq!(summary.tracks_removed, 0);
    assert!(
        summary.errors.is_empty(),
        "Ingestion should have no errors: {:?}",
        summary.errors
    );

    // 6. Verify tracks in SQLite repository
    let tracks = repo
        .find_all(TrackFilter::default())
        .await
        .expect("query tracks");
    assert_eq!(
        tracks.len(),
        3,
        "Database must persist exactly 3 tracks from scan"
    );

    let mp3_path_str = mp3_file.to_string_lossy().to_string();
    let mp3_track = tracks
        .iter()
        .find(|t| t.file_path == mp3_path_str)
        .expect("mp3 track must exist in database");

    assert_eq!(mp3_track.title, "Auralis Test Title");
    assert_eq!(mp3_track.artist.as_deref(), Some("Auralis Artist"));
    assert_eq!(mp3_track.album.as_deref(), Some("Auralis Album"));
    assert_eq!(
        mp3_track.format,
        auralis_lib::domain::models::AudioFormat::Mp3
    );

    // Clean up
    let _ = std::fs::remove_dir_all(&test_dir);
    let _ = std::fs::remove_file(&db_path);
}
