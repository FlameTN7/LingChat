//! Names for deterministic regression scenarios.

pub const SCENARIOS: &[&str] = &[
    "basic-compression",
    "append-during-update",
    "one-section-fails",
    "empty-section-fails",
    "persistence-roundtrip",
    "memory-finishes-after-line-save",
];

pub fn known(name: &str) -> bool {
    SCENARIOS.contains(&name)
}
