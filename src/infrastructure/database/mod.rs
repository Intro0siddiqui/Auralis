//! Database Infrastructure Module
//!
//! SQLite database implementation using rusqlite.

mod connection;
pub mod repositories;

pub use connection::{Database, DatabaseError};
pub use repositories::*;
