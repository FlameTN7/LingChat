#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::ai_service::types::GameMemoryBank;
    use crate::memory_test_api::harness::{validate_real, validate_scripted};
    use crate::memory_test_api::scripted_provider::ScriptedProvider;

    #[tokio::test]
    async fn successful_scripted_provider_returns_four_distinct_sections() {
        let sections = validate_scripted(&ScriptedProvider::default())
            .await
            .unwrap();
        assert_eq!(sections.len(), 4);
        assert_ne!(sections[0], sections[1]);
    }

    #[tokio::test]
    async fn an_empty_response_is_a_failure_without_pointer_advance() {
        let provider = ScriptedProvider {
            empty_section: Some("promises".into()),
            ..Default::default()
        };
        let result = validate_real(
            provider,
            GameMemoryBank::default(),
            7,
            None,
            4,
            1,
            0,
            crate::ai_service::game_system::persistent_memory_system::MemorySectionLimits::default(
            ),
            Duration::from_secs(5),
            false,
            false,
            "Test AI",
        )
        .await
        .unwrap();
        assert!(!result.committed);
        assert_eq!(result.processed_idx, 0);
        assert_eq!(result.calls, 4);
    }

    #[tokio::test]
    async fn display_name_reaches_production_compression_prompt() {
        let provider = ScriptedProvider::default();
        let result = validate_real(
            provider.clone(),
            GameMemoryBank::default(),
            7,
            None,
            1,
            1,
            0,
            crate::ai_service::game_system::persistent_memory_system::MemorySectionLimits::default(
            ),
            Duration::from_secs(5),
            false,
            false,
            "雪月花",
        )
        .await
        .unwrap();
        assert!(result.committed);
        assert!(provider.saw_prompt_text("【角色名称】：雪月花"));
    }

    #[tokio::test]
    async fn append_during_update_commits_original_target_and_leaves_tail() {
        let result = validate_real(
            ScriptedProvider {
                delay_ms: 15,
                ..Default::default()
            },
            GameMemoryBank::default(),
            7,
            None,
            4,
            1,
            0,
            crate::ai_service::game_system::persistent_memory_system::MemorySectionLimits::default(
            ),
            Duration::from_secs(5),
            true,
            false,
            "Test AI",
        )
        .await
        .unwrap();
        assert!(result.committed);
        assert_eq!(result.first_processed_idx, 4);
        assert!(result.second_batch_committed);
        assert_eq!(result.tail_lines, 1);
        assert_eq!(result.calls, 8);
    }
}
