//! Filesystem Infrastructure Module
//!
//! Handles file system operations including directory scanning,
//! platform-specific storage ingestion, and audio metadata extraction.

pub mod android;
pub mod desktop;
pub mod metadata;
pub mod scanner;

pub use android::AndroidScanner;
pub use desktop::DesktopScanner;
pub use metadata::{
    verify_audio_health, verify_audio_health_async, write_metadata, MetadataExtractor,
};
pub use scanner::{DirectoryScanner, ScanProgress, ScannerError};
