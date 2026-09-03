use crate::ai_service::game_system::auto_save::{
    fingerprint_requires_save, successful_fingerprint,
};

#[test]
fn memory_revision_change_requires_a_subsequent_save() {
    let saved = successful_fingerprint(42, vec![(7, 1)]);
    let unchanged = successful_fingerprint(42, vec![(7, 1)]);
    let after_memory_commit = successful_fingerprint(42, vec![(7, 2)]);

    assert!(!fingerprint_requires_save(
        Some(11),
        Some(&saved),
        11,
        &unchanged,
    ));
    assert!(fingerprint_requires_save(
        Some(11),
        Some(&saved),
        11,
        &after_memory_commit,
    ));
}

#[test]
fn failed_persistence_does_not_advance_success_fingerprint_and_retries() {
    let saved = successful_fingerprint(42, vec![(7, 1)]);
    let after_memory_commit = successful_fingerprint(42, vec![(7, 2)]);
    let mut success_marker = Some(saved.clone());

    // A failed persistence call leaves the marker untouched.
    assert!(fingerprint_requires_save(
        Some(11),
        success_marker.as_ref(),
        11,
        &after_memory_commit,
    ));
    assert_eq!(success_marker.as_ref(), Some(&saved));

    // The next successful persistence advances it, making later checks skip.
    success_marker = Some(after_memory_commit.clone());
    assert!(!fingerprint_requires_save(
        Some(11),
        success_marker.as_ref(),
        11,
        &after_memory_commit,
    ));
}
