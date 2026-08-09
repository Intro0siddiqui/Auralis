//! Auralis v2 - Main Entry Point
//!
//! This is the entry point for the Auralis music player application.
//! It initializes the Tauri runtime, sets up logging, and configures
//! the application before launching.

use auralis_lib::AuralisApp;
use std::panic;
use tracing::{error, info};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

fn main() {
    // Set up panic hook for better error reporting
    panic::set_hook(Box::new(|panic_info| {
        let payload = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "Unknown panic payload".to_string()
        };

        let location = if let Some(loc) = panic_info.location() {
            format!("{}:{}:{}", loc.file(), loc.line(), loc.column())
        } else {
            "unknown location".to_string()
        };

        error!(target: "panic", payload = %payload, location = %location, "Application panic occurred");
        eprintln!("PANIC at {}: {}", location, payload);
    }));

    // Initialize logging
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,auralis=debug"));

    tracing_subscriber::registry()
        .with(fmt::layer().with_target(true).with_thread_ids(true))
        .with(filter)
        .init();

    info!("Auralis v2 starting...");
    info!("Initializing Tauri application");

    // Build and run the Tauri application
    let result = AuralisApp::build();

    match result {
        Ok(app) => {
            info!("Application built successfully, running...");
            app.run(|_app, event| {
                info!(event = ?event, "Tauri event received");
            });
        }
        Err(e) => {
            error!(error = %e, "Failed to build application");
            eprintln!("Failed to build application: {}", e);
            std::process::exit(1);
        }
    }

    info!("Auralis v2 shutting down gracefully");
}
