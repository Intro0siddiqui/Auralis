//! Filesystem Infrastructure Module
//!
//! Handles file system operations including directory scanning,
//! platform-specific storage ingestion, and audio metadata extraction.

pub mod android;
pub mod desktop;
mod metadata;
pub mod scanner;

pub use android::AndroidScanner;
pub use desktop::DesktopScanner;
pub use metadata::MetadataExtractor;
pub use scanner::{DirectoryScanner, ScanProgress, ScannerError};
