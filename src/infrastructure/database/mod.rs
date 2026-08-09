//! Database Infrastructure Module
//!
//! SQLite database implementation using rusqlite.

mod connection;
mod repositories;

pub use connection::Database;
pub use repositories::*;
