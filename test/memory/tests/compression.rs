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
            4,
            1,
            Duration::from_secs(5),
            false,
        )
        .await
        .unwrap();
        assert!(!result.committed);
        assert_eq!(result.processed_idx, 0);
        assert_eq!(result.calls, 4);
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
            4,
            1,
            Duration::from_secs(5),
            true,
        )
        .await
        .unwrap();
        assert!(result.committed);
        assert_eq!(result.processed_idx, 4);
        assert_eq!(result.tail_lines, 1);
    }
}
