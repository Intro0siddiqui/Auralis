//! Filesystem Infrastructure Module
//!
//! Handles file system operations including directory scanning
//! and audio metadata extraction.

mod metadata;
mod scanner;

pub use metadata::MetadataExtractor;
pub use scanner::DirectoryScanner;
