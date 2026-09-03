//! Harness for the real PersistentMemorySystem, used by both HTTP scenarios and tests.

use anyhow::{Result, anyhow};
use std::time::{Duration, Instant};
use tokio::time::sleep;

use super::scripted_provider::ScriptedProvider;
use crate::ai_service::game_system::game_status::GameStatus;
use crate::ai_service::game_system::persistent_memory_system::{
    MemorySectionLimits, PersistentMemorySystem,
};
use crate::ai_service::game_system::role_manager::GameRoleManager;
use crate::ai_service::llm::LlmSlot;
use crate::ai_service::types::{GameLine, GameMemoryBank, LineAttributeExt, LineBase};
use crate::config::tts::TtsConfig;
use crate::db::entities::line::LineAttribute;

pub struct ValidationResult {
    pub triggered: bool,
    pub committed: bool,
    pub calls: usize,
    pub bank: GameMemoryBank,
    pub processed_idx: i64,
    pub tail_lines: usize,
    pub updating: bool,
    pub system_memory: String,
    pub short_term_memory: String,
    pub target_idx: usize,
    pub first_processed_idx: i64,
    pub second_batch_committed: bool,
}

fn lines(role_id: i32, count: usize) -> Vec<GameLine> {
    (0..count)
        .map(|idx| {
            GameLine::from_base(
                LineBase {
                    content: format!("deterministic test line {idx}"),
                    attribute: LineAttributeExt(LineAttribute::User),
                    sender_role_id: Some(0),
                    ..Default::default()
                },
                vec![role_id],
            )
        })
        .collect()
}

/// Run one complete compression through the production PersistentMemorySystem.
/// The provider is delayed per request, allowing callers to append while it runs.
pub async fn validate_real(
    provider: ScriptedProvider,
    initial_bank: GameMemoryBank,
    role_id: i32,
    input_lines: Option<Vec<GameLine>>,
    line_count: usize,
    update_interval: usize,
    recent_window: usize,
    section_limits: MemorySectionLimits,
    timeout: Duration,
    append_during_update: bool,
    rollback_during_update: bool,
    display_name: &str,
) -> Result<ValidationResult> {
    let llm = provider.clone().slot();
    let memory = PersistentMemorySystem::new(
        role_id,
        &initial_bank,
        llm,
        true,
        update_interval,
        recent_window,
        section_limits,
        display_name,
    );
    let mut history = input_lines.unwrap_or_else(|| lines(role_id, line_count));
    let target = history.len();
    memory.check_and_trigger_auto_update(&history);
    let triggered = {
        let deadline = Instant::now() + timeout;
        loop {
            if memory.is_updating() || provider.calls() > 0 {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            sleep(Duration::from_millis(2)).await;
        }
    };
    if append_during_update && triggered {
        // Exercise the production append entry point. No roles are present in
        // this minimal status, so refresh_memories is a no-op after the append;
        // the memory system above still owns the real compression task.
        let db = super::temp_db::TemporaryDatabase::open().await?;
        let _ = db.seed_save_role(role_id, display_name).await?;
        let slot: LlmSlot = provider.clone().slot();
        let mut manager = GameRoleManager::new(
            db.directory.path().to_path_buf(),
            slot,
            TtsConfig::default(),
            None,
            false,
            1,
            0,
            MemorySectionLimits::default(),
        );
        manager.loaded_roles.insert(
            role_id,
            crate::ai_service::types::GameRole {
                role_id: Some(role_id),
                display_name: Some(display_name.to_string()),
                ..Default::default()
            },
        );
        let mut status = GameStatus::new(manager);
        status.present_role_ids.insert(role_id);
        status.line_list = history;
        status
            .add_line(
                &db.connection,
                LineBase {
                    content: "deterministic appended line".into(),
                    attribute: LineAttributeExt(LineAttribute::User),
                    sender_role_id: Some(0),
                    ..Default::default()
                },
            )
            .await?;
        history = status.line_list;
    }
    if rollback_during_update && triggered {
        // Rewrite invalidates the in-flight result; use the same production
        // PersistentMemorySystem guard rather than a fake result override.
        memory.rewrite_from(0).await;
    }
    let deadline = Instant::now() + timeout;
    while memory.is_updating() && Instant::now() < deadline {
        sleep(Duration::from_millis(2)).await;
    }
    let first_snapshot = memory.snapshot().await;
    let first_processed_idx = first_snapshot.bank.meta.last_processed_global_idx;
    let first_tail_lines = history
        .len()
        .saturating_sub(first_processed_idx.max(0) as usize);
    let mut snapshot = first_snapshot;
    let mut second_batch_committed = false;
    if append_during_update && first_processed_idx >= target as i64 {
        // The newly appended tail must be eligible for a subsequent threshold
        // batch, not merely present in a local Vec used for assertions.
        memory.check_and_trigger_auto_update(&history);
        let deadline = Instant::now() + timeout;
        while memory.is_updating() && Instant::now() < deadline {
            sleep(Duration::from_millis(2)).await;
        }
        snapshot = memory.snapshot().await;
        second_batch_committed =
            snapshot.bank.meta.last_processed_global_idx >= history.len() as i64;
    }
    let processed_idx = snapshot.bank.meta.last_processed_global_idx;
    let committed = processed_idx >= target as i64;
    if snapshot.updating {
        return Err(anyhow!(
            "validation timed out while compression was running"
        ));
    }
    let tail_lines = first_tail_lines;
    Ok(ValidationResult {
        triggered,
        committed,
        calls: provider.calls(),
        bank: snapshot.bank,
        processed_idx,
        tail_lines,
        updating: snapshot.updating,
        system_memory: memory.get_system_memory_text().await,
        short_term_memory: memory.get_short_term_user_text().await,
        target_idx: target,
        first_processed_idx,
        second_batch_committed,
    })
}

/// Compatibility helper retained for small callers; unlike the old implementation it
/// exercises four real LlmClient calls and the production memory state machine.
pub async fn validate_scripted(provider: &ScriptedProvider) -> Result<[String; 4], String> {
    let result = validate_real(
        provider.clone(),
        GameMemoryBank::default(),
        7,
        None,
        4,
        1,
        0,
        MemorySectionLimits::default(),
        Duration::from_secs(5),
        false,
        false,
        "Test AI",
    )
    .await
    .map_err(|e| e.to_string())?;
    if !result.committed {
        return Err("compression was not committed".into());
    }
    Ok([
        result.bank.data.short_term,
        result.bank.data.long_term,
        result.bank.data.user_info,
        result.bank.data.promises,
    ])
}
