//! Domain Layer Module
//!
//! This module contains the core business logic of the Auralis application,
//! organized into models, services, and repository interfaces. The domain
//! layer is designed to be framework-agnostic and can be used independently
//! of Tauri or any other presentation layer.

pub mod models;
pub mod repositories;
pub mod services;
