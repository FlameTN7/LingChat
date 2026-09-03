#[cfg(test)]
mod tests {
    use crate::ai_service::types::GameMemoryBank;
    use crate::db::managers::memory_repo::MemoryRepo;
    use crate::memory_test_api::temp_db::TemporaryDatabase;

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
