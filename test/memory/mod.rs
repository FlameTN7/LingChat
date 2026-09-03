//! Feature-gated local validation entry point.
//!
//! This deliberately remains a small facade: production implementation details stay
//! private to the library crate and the binary only invokes `run`.

use std::io::{self, BufRead, Write};

pub mod api;
pub mod harness;
pub mod scenarios;
pub mod scripted_provider;
pub mod temp_db;

#[cfg(test)]
#[path = "tests/api.rs"]
mod api_tests;
#[cfg(test)]
#[path = "tests/compression.rs"]
mod compression_tests;
#[cfg(test)]
#[path = "tests/persistence.rs"]
mod persistence_tests;

/// Run the deterministic offline validation service.
///
/// The line-oriented protocol keeps this helper usable from Node/Pi without adding
/// another HTTP dependency to the production application. A future HTTP adapter can
/// call the same `validate` function without changing the memory runtime.
pub fn run() {
    println!("{{\"event\":\"ready\",\"host\":\"127.0.0.1\",\"port\":0,\"api_version\":1}}");
    let _ = io::stdout().flush();
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let command = line.trim();
        if command == "health" {
            println!("{{\"ok\":true,\"mode\":\"scripted\",\"api_version\":1}}");
        } else if command == "shutdown" {
            println!("{{\"ok\":true}}");
            break;
        } else if !command.is_empty() {
            println!("{{\"ok\":false,\"error\":\"unknown_command\"}}");
        }
        let _ = io::stdout().flush();
    }
}
