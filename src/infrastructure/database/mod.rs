//! Database Infrastructure Module
//!
//! SQLite database implementation using rusqlite.

mod connection;
pub mod repositories;

pub use connection::Database;
pub use repositories::*;
