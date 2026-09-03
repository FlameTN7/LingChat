//! Feature-gated local validation entry point.
//!
//! This deliberately remains a small facade: production implementation details stay
//! private to the library crate and the binary only invokes the HTTP facade.

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
