use crate::ai_service::game_system::auto_save::{
    AutoSaveManager, fingerprint_requires_save, successful_fingerprint,
};
use crate::ai_service::game_system::game_status::GameStatus;
use crate::ai_service::game_system::persistent_memory_system::MemorySectionLimits;
use crate::ai_service::game_system::role_manager::GameRoleManager;
use crate::ai_service::llm::LlmSlot;
use crate::ai_service::service::AIService;
use crate::ai_service::types::{GameLine, GameMemoryBank, GameRole, LineAttributeExt, LineBase};
use crate::config::tts::TtsConfig;
use crate::db::entities::line::LineAttribute;
use crate::db::managers::memory_repo::MemoryRepo;
use crate::memory_test_api::scripted_provider::ScriptedProvider;
use crate::memory_test_api::temp_db::TemporaryDatabase;
use std::sync::Arc;
use tokio::sync::Mutex;

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

    assert!(fingerprint_requires_save(
        Some(11),
        success_marker.as_ref(),
        11,
        &after_memory_commit,
    ));
    assert_eq!(success_marker.as_ref(), Some(&saved));

    success_marker = Some(after_memory_commit.clone());
    assert!(!fingerprint_requires_save(
        Some(11),
        success_marker.as_ref(),
        11,
        &after_memory_commit,
    ));
}

#[tokio::test]
async fn real_autosave_persists_memory_that_finishes_after_line_save() {
    let db = TemporaryDatabase::open().await.unwrap();
    let (_seed_save, role_id) = db.seed_save_role(707, "late-memory").await.unwrap();
    let provider = ScriptedProvider {
        delay_ms: 100,
        ..Default::default()
    };
    let llm: LlmSlot = provider.clone().slot();
    let service = AIService::new(
        db.connection.clone(),
        db.directory.path().to_path_buf(),
        llm,
        TtsConfig::default(),
        None,
        true,
        1,
        0,
        MemorySectionLimits::default(),
    )
    .await;
    let mut manager = GameRoleManager::new(
        db.directory.path().to_path_buf(),
        provider.clone().slot(),
        TtsConfig::default(),
        None,
        true,
        1,
        0,
        MemorySectionLimits::default(),
    );
    manager.loaded_roles.insert(
        role_id,
        GameRole {
            role_id: Some(role_id),
            display_name: Some("late-memory".into()),
            ..Default::default()
        },
    );
    let status = GameStatus::new(manager);
    let shared = Arc::new(Mutex::new(service));
    shared.lock().await.game_status = Arc::new(Mutex::new(status));
    {
        let service = shared.lock().await;
        let mut status = service.game_status.lock().await;
        status.main_role_id = Some(role_id);
        status.present_role_ids.insert(role_id);
        status.line_list = (0..2)
            .map(|idx| {
                GameLine::from_base(
                    LineBase {
                        content: format!("late save line {idx}"),
                        attribute: LineAttributeExt(LineAttribute::User),
                        sender_role_id: Some(role_id),
                        ..Default::default()
                    },
                    vec![role_id],
                )
            })
            .collect();
        status.refresh_memories(&db.connection).await.unwrap();
    }
    while provider.calls() < 4 {
        tokio::task::yield_now().await;
    }

    let mut autosave = AutoSaveManager::for_test(db.connection.clone(), shared.clone());
    // The line snapshot is saved first while memory compression is in flight.
    autosave.perform_test_save().await.unwrap();
    let first_revision = autosave.test_saved_revision().unwrap();
    provider.wait_idle().await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;

    // Lines did not change, but the real runtime memory revision did.
    autosave.perform_test_save().await.unwrap();
    let second_revision = autosave.test_saved_revision().unwrap();
    assert_ne!(first_revision, second_revision);
    assert!(second_revision.iter().any(|(_, revision)| *revision > 0));

    let save_id = {
        let service = shared.lock().await;
        let status = service.game_status.lock().await;
        status.active_save_id.unwrap()
    };
    let row = MemoryRepo::get_latest_memory(&db.connection, save_id, role_id)
        .await
        .unwrap()
        .unwrap();
    let persisted: GameMemoryBank = serde_json::from_str(&row.info).unwrap();
    assert_eq!(persisted.meta.last_processed_global_idx, 2);
    assert_eq!(persisted.data.short_term, "[scripted:short_term]");
}
