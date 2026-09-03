//! Harness for the real PersistentMemorySystem, used by both HTTP scenarios and tests.

use anyhow::{Result, anyhow};
use std::time::{Duration, Instant};
use tokio::time::sleep;

use super::scripted_provider::ScriptedProvider;
use crate::ai_service::game_system::persistent_memory_system::{
    MemorySectionLimits, PersistentMemorySystem,
};
use crate::ai_service::types::{GameLine, GameMemoryBank, LineAttributeExt, LineBase};
use crate::db::entities::line::LineAttribute;

pub struct ValidationResult {
    pub triggered: bool,
    pub committed: bool,
    pub calls: usize,
    pub bank: GameMemoryBank,
    pub processed_idx: i64,
    pub tail_lines: usize,
    pub updating: bool,
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
    line_count: usize,
    update_interval: usize,
    timeout: Duration,
    append_during_update: bool,
) -> Result<ValidationResult> {
    let llm = provider.clone().slot();
    let memory = PersistentMemorySystem::new(
        role_id,
        &initial_bank,
        llm,
        true,
        update_interval,
        0,
        MemorySectionLimits::default(),
        "Test AI",
    );
    let mut history = lines(role_id, line_count);
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
        // This append must remain after the target captured by the running job.
        history.extend(lines(role_id, 1));
    }
    let deadline = Instant::now() + timeout;
    while memory.is_updating() && Instant::now() < deadline {
        sleep(Duration::from_millis(2)).await;
    }
    let snapshot = memory.snapshot().await;
    let processed_idx = snapshot.bank.meta.last_processed_global_idx;
    let committed = processed_idx >= target as i64;
    if snapshot.updating {
        return Err(anyhow!(
            "validation timed out while compression was running"
        ));
    }
    let tail_lines = history.len().saturating_sub(processed_idx.max(0) as usize);
    Ok(ValidationResult {
        triggered,
        committed,
        calls: provider.calls(),
        bank: snapshot.bank,
        processed_idx,
        tail_lines,
        updating: snapshot.updating,
    })
}

/// Compatibility helper retained for small callers; unlike the old implementation it
/// exercises four real LlmClient calls and the production memory state machine.
pub async fn validate_scripted(provider: &ScriptedProvider) -> Result<[String; 4], String> {
    let result = validate_real(
        provider.clone(),
        GameMemoryBank::default(),
        7,
        4,
        1,
        Duration::from_secs(5),
        false,
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
