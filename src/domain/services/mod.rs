//! Domain Services Module
//!
//! Contains the business logic services that orchestrate domain operations.
//! Services are designed to be framework-agnostic and testable.

pub mod sync_service;

pub use sync_service::{RamTrackBuffer, SyncService};
