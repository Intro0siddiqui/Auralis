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
