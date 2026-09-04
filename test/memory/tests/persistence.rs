#[cfg(test)]
mod tests {
    use crate::ai_service::game_system::persistent_memory_system::MemorySectionLimits;
    use crate::ai_service::game_system::role_manager::GameRoleManager;
    use crate::ai_service::llm::LlmSlot;
    use crate::ai_service::types::{GameMemoryBank, GameRole};
    use crate::config::tts::TtsConfig;
    use crate::db::entities::memory_bank;
    use crate::db::managers::memory_repo::MemoryRepo;
    use crate::memory_test_api::temp_db::TemporaryDatabase;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    #[tokio::test]
    async fn memory_bank_round_trips_through_migrated_sqlite() {
        let db = TemporaryDatabase::open().await.unwrap();
        let (save_id, role_id) = db.seed_save_role(701, "round-trip").await.unwrap();
        let mut bank = GameMemoryBank::default();
        bank.meta.last_processed_global_idx = 13;
        bank.data.short_term = "短期对话".into();
        bank.data.long_term = "Long term".into();
        bank.data.user_info = "用户偏好".into();
        bank.data.promises = "约定".into();
        let loaded = db.round_trip(save_id, role_id, &bank).await.unwrap();
        assert_eq!(loaded, bank);
    }

    #[test]
    fn multilingual_fixture_matches_game_line_serde_contract() {
        let value: serde_json::Value =
            serde_json::from_str(include_str!("../../fixtures/memory/multilingual.json")).unwrap();
        let line = value["lines"][0].clone();
        let parsed: crate::ai_service::types::GameLine = serde_json::from_value(line).unwrap();
        assert_eq!(parsed.base.content, "风雪说：你好，世界 🌏");
        assert_eq!(parsed.base.attribute.as_str(), "user");
        assert_eq!(value["display_name"], "测试角色");
    }

    #[tokio::test]
    async fn duplicate_rows_select_latest_before_parsing() {
        let db = TemporaryDatabase::open().await.unwrap();
        let (save_id, role_id) = db.seed_save_role(708, "duplicates").await.unwrap();
        let old = memory_bank::ActiveModel {
            info: sea_orm::Set("{not-json".into()),
            save_id: sea_orm::Set(save_id),
            role_id: sea_orm::Set(Some(role_id)),
            ..Default::default()
        };
        use sea_orm::ActiveModelTrait;
        old.insert(&db.connection).await.unwrap();
        let mut bank = GameMemoryBank::default();
        bank.data.long_term = "latest valid".into();
        let encoded = serde_json::to_string(&bank).unwrap();
        memory_bank::ActiveModel {
            info: sea_orm::Set(encoded),
            save_id: sea_orm::Set(save_id),
            role_id: sea_orm::Set(Some(role_id)),
            ..Default::default()
        }
        .insert(&db.connection)
        .await
        .unwrap();
        let row = MemoryRepo::get_latest_memory(&db.connection, save_id, role_id)
            .await
            .unwrap()
            .unwrap();
        let loaded: GameMemoryBank = serde_json::from_str(&row.info).unwrap();
        assert_eq!(loaded.data.long_term, "latest valid");
    }

    #[tokio::test]
    async fn role_manager_ignores_old_bad_duplicate_but_rejects_new_bad_duplicate() {
        let db = TemporaryDatabase::open().await.unwrap();
        let (save_id, role_id) = db.seed_save_role(709, "duplicate-load").await.unwrap();
        use sea_orm::ActiveModelTrait;
        memory_bank::ActiveModel {
            info: sea_orm::Set("{old-bad".into()),
            save_id: sea_orm::Set(save_id),
            role_id: sea_orm::Set(Some(role_id)),
            ..Default::default()
        }
        .insert(&db.connection)
        .await
        .unwrap();
        let valid = serde_json::to_string(&GameMemoryBank::default()).unwrap();
        memory_bank::ActiveModel {
            info: sea_orm::Set(valid),
            save_id: sea_orm::Set(save_id),
            role_id: sea_orm::Set(Some(role_id)),
            ..Default::default()
        }
        .insert(&db.connection)
        .await
        .unwrap();
        let llm: LlmSlot = Arc::new(RwLock::new(None));
        let mut manager = GameRoleManager::new(
            db.directory.path().to_path_buf(),
            llm,
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
                display_name: Some("duplicate-load".into()),
                ..Default::default()
            },
        );
        manager
            .load_memory_banks_from_db(&db.connection, save_id, Some(&[role_id]))
            .await
            .unwrap();

        memory_bank::ActiveModel {
            info: sea_orm::Set("{new-bad".into()),
            save_id: sea_orm::Set(save_id),
            role_id: sea_orm::Set(Some(role_id)),
            ..Default::default()
        }
        .insert(&db.connection)
        .await
        .unwrap();
        let error = manager
            .load_memory_banks_from_db(&db.connection, save_id, Some(&[role_id]))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("memory_bank.id"));
    }

    #[tokio::test]
    async fn malformed_memory_json_is_reported() {
        let db = TemporaryDatabase::open().await.unwrap();
        let (save_id, role_id) = db.seed_save_role(702, "malformed").await.unwrap();
        let error = db.malformed_json_error(save_id, role_id).await.unwrap_err();
        assert!(error.to_string().contains("malformed memory JSON"));
    }

    #[tokio::test]
    async fn rows_are_isolated_by_save_and_role() {
        let db = TemporaryDatabase::open().await.unwrap();
        let (save_a, role_a) = db.seed_save_role(703, "a").await.unwrap();
        let (save_b, role_b) = db.seed_save_role(704, "b").await.unwrap();
        let mut bank = GameMemoryBank::default();
        bank.data.long_term = "A".into();
        let row_a = db.round_trip(save_a, role_a, &bank).await.unwrap();
        bank.data.long_term = "B".into();
        let row_b = db.round_trip(save_b, role_b, &bank).await.unwrap();
        assert_eq!(row_a.data.long_term, "A");
        assert_eq!(row_b.data.long_term, "B");
    }

    #[tokio::test]
    async fn explicit_row_id_cannot_cross_save_or_role_ownership() {
        let db = TemporaryDatabase::open().await.unwrap();
        let (save_a, role_a) = db.seed_save_role(705, "a").await.unwrap();
        let (save_b, role_b) = db.seed_save_role(706, "b").await.unwrap();
        let bank = GameMemoryBank::default();
        db.round_trip(save_a, role_a, &bank).await.unwrap();
        let json = serde_json::to_string(&bank).unwrap();
        let error = MemoryRepo::upsert_memory(
            &db.connection,
            save_b,
            role_b,
            &json,
            Some(row_id(&db, save_a, role_a).await),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("belongs to"));
    }

    async fn row_id(db: &TemporaryDatabase, save_id: i32, role_id: i32) -> i32 {
        MemoryRepo::get_latest_memory(&db.connection, save_id, role_id)
            .await
            .unwrap()
            .unwrap()
            .id
    }
}
