use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use chrono::Local;
use sea_orm::DatabaseConnection;
use serde::Serialize;
use tauri::{AppHandle, Emitter, WebviewWindow};
use tokio::sync::Mutex;

use crate::ai_service::service::SharedAIService;
use crate::ai_service::types::{GameLine, GameMemoryBank};
use crate::db::managers::memory_repo::MemoryRepo;
use crate::db::managers::save_repo::SaveRepo;

const AUTO_SAVE_PREFIX: &str = "自动存档";
const AUTO_SAVE_INTERVAL_SECS: u64 = 300; // 5 minutes
const EXIT_SAVE_TIMEOUT_SECS: u64 = 5;

/// Runtime state used to decide whether the target auto-save slot is current.
/// Role revisions are sorted by `role_id` by `GameRoleManager` before hashing.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AutoSaveFingerprint {
    line_hash: u64,
    memory_revisions: Vec<(i32, u64)>,
}

fn fingerprint_requires_save(
    last_save_id: Option<i32>,
    last: Option<&AutoSaveFingerprint>,
    save_id: i32,
    current: &AutoSaveFingerprint,
) -> bool {
    last_save_id != Some(save_id) || last != Some(current)
}

fn successful_fingerprint(line_hash: u64, revisions: Vec<(i32, u64)>) -> AutoSaveFingerprint {
    AutoSaveFingerprint {
        line_hash,
        memory_revisions: revisions,
    }
}

/// Payload emitted to frontend after each successful auto-save.
#[derive(Debug, Clone, Serialize)]
struct AutoSaveEventPayload {
    save_id: i32,
    title: String,
    timestamp: String,
}

pub struct AutoSaveManager {
    app: Option<AppHandle>,
    db: DatabaseConnection,
    ai_service: SharedAIService,
    /// Snapshot fingerprint at the moment of the last successful auto-save.
    last_saved_fingerprint: Option<AutoSaveFingerprint>,
    /// Save slot to which `last_saved_fingerprint` was written. Fingerprints are
    /// slot-local: the same runtime state in another save must not be skipped.
    last_saved_save_id: Option<i32>,
    /// Resolved auto-save slot ID (lazily found or created on first save).
    auto_save_id: Option<i32>,
}

#[cfg(all(test, feature = "memory-test-api"))]
#[path = "../../../../test/memory/tests/auto_save.rs"]
mod tests;

impl AutoSaveManager {
    pub fn new(app: AppHandle, db: DatabaseConnection, ai_service: SharedAIService) -> Self {
        Self {
            app: Some(app),
            db,
            ai_service,
            last_saved_fingerprint: None,
            last_saved_save_id: None,
            auto_save_id: None,
        }
    }

    // ========== Periodic Loop ==========

    /// Run the periodic auto-save loop (every 5 minutes).  Never returns.
    pub async fn run_periodic(manager: Arc<Mutex<Self>>) {
        let mut interval = tokio::time::interval(Duration::from_secs(AUTO_SAVE_INTERVAL_SECS));
        loop {
            interval.tick().await;
            let mut mgr = manager.lock().await;
            if let Err(e) = mgr.perform_save().await {
                tracing::warn!("[AutoSave] 自动存档失败: {}", e);
            }
        }
    }

    // ========== Close Handler ==========

    /// Register a close-requested handler on the main window that performs a
    /// final auto-save before allowing the window to actually close.
    pub fn setup_close_handler(app: AppHandle, window: WebviewWindow, manager: Arc<Mutex<Self>>) {
        window.clone().on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Prevent the window from closing immediately
                api.prevent_close();

                let mgr = manager.clone();
                let ah = app.clone();

                tauri::async_runtime::spawn(async move {
                    tracing::info!("[AutoSave] 正在执行退出前自动存档...");

                    let save_result =
                        tokio::time::timeout(Duration::from_secs(EXIT_SAVE_TIMEOUT_SECS), async {
                            let mut mgr = mgr.lock().await;
                            mgr.perform_exit_save().await
                        })
                        .await;

                    match save_result {
                        Ok(Ok(())) => tracing::info!("[AutoSave] 退出前存档完成"),
                        Ok(Err(ref e)) => tracing::error!("[AutoSave] 退出前存档失败: {}", e),
                        Err(_) => tracing::warn!(
                            "[AutoSave] 退出前存档超时（{} 秒），放弃等待",
                            EXIT_SAVE_TIMEOUT_SECS
                        ),
                    }

                    // Drop the manager lock before exiting
                    drop(save_result);

                    // 通知前端存档已完成，由前端决定是否退出
                    let _ = ah.emit("app:close-ready", ());
                });
            }
        });
    }

    // ========== Core Save Logic ==========

    /// Perform a save if line_list is non-empty and has changed since last save.
    async fn perform_save(&mut self) -> Result<(), String> {
        // Capture the fingerprint and the exact lines that will be sent to
        // sync_lines as one immutable session snapshot. Re-reading line_list
        // after this point could pair marker A with persisted lines B when an
        // append/rollback races the save.
        let (current_fingerprint, lines, memory_snapshots) = match self.compute_snapshot().await {
            Some(snapshot) => snapshot,
            None => return Ok(()),
        };

        // 2. Find or create the auto-save slot before deciding whether to skip.
        // This keeps the success marker explicitly scoped to that slot.
        let save_id = self.find_or_create_slot().await?;

        // 3. A completed memory compression changes this fingerprint even when
        // line_list is unchanged, so it cannot be skipped.
        if !fingerprint_requires_save(
            self.last_saved_save_id,
            self.last_saved_fingerprint.as_ref(),
            save_id,
            &current_fingerprint,
        ) {
            return Ok(());
        }

        // 4. Perform the actual save
        let service = self.ai_service.lock().await;

        // 4a. Sync the immutable snapshot captured above (smart diff)
        SaveRepo::sync_lines(&self.db, save_id, &lines)
            .await
            .map_err(|e| format!("同步台词失败: {}", e))?;

        // 4b. Set active save
        service.game_status.lock().await.active_save_id = Some(save_id);

        // 4c. Write GameStatus snapshot
        let snapshot = service.game_status.lock().await.to_snapshot();
        let snapshot_json =
            serde_json::to_string(&snapshot).map_err(|e| format!("序列化状态失败: {}", e))?;
        SaveRepo::update_save_status(&self.db, save_id, &snapshot_json)
            .await
            .map_err(|e| format!("保存状态失败: {}", e))?;

        // 4d. Persist the immutable bank snapshots without retaining the
        // GameStatus lock across database awaits. A concurrent commit therefore
        // cannot make the success marker claim a newer bank was persisted.
        let mut saved_memory_revisions = Vec::with_capacity(memory_snapshots.len());
        for (role_id, bank, revision) in &memory_snapshots {
            let info =
                serde_json::to_string(bank).map_err(|e| format!("序列化记忆库失败: {}", e))?;
            MemoryRepo::upsert_memory(&self.db, save_id, *role_id, &info, None)
                .await
                .map_err(|e| format!("保存记忆库失败: {}", e))?;
            saved_memory_revisions.push((*role_id, *revision));
        }

        // 4e. Persist script state (if running)
        if let Some(ref script_status) = service.game_status.lock().await.script_status {
            let vars_json = serde_json::to_string(&script_status.vars)
                .map_err(|e| format!("序列化剧本状态失败: {}", e))?;
            SaveRepo::upsert_running_script(
                &self.db,
                save_id,
                &script_status.folder_key,
                &vars_json,
                &script_status.current_chapter_key,
                script_status.current_event_process,
            )
            .await
            .map_err(|e| format!("保存剧本状态失败: {}", e))?;
        }

        drop(service);

        // 5. Update tracking state only after every required write above succeeds.
        // Replace memory revisions with the revisions captured by the successful
        // DB write; this is the exact snapshot represented by this save slot.
        let saved_fingerprint =
            successful_fingerprint(current_fingerprint.line_hash, saved_memory_revisions);
        self.last_saved_fingerprint = Some(saved_fingerprint);
        self.last_saved_save_id = Some(save_id);

        // 6. Emit event to frontend
        let now = Local::now();
        let title = format!("{} {}", AUTO_SAVE_PREFIX, now.format("%Y-%m-%d %H:%M:%S"));
        let timestamp = now.format("%H:%M:%S").to_string();

        if let Some(app) = &self.app {
            let _ = app.emit(
                "save:auto-saved",
                AutoSaveEventPayload {
                    save_id,
                    title,
                    timestamp,
                },
            );
        }

        tracing::info!("[AutoSave] 自动存档完成 save_id={}", save_id);
        Ok(())
    }

    #[cfg(feature = "memory-test-api")]
    pub fn for_test(db: DatabaseConnection, ai_service: SharedAIService) -> Self {
        Self {
            app: None,
            db,
            ai_service,
            last_saved_fingerprint: None,
            last_saved_save_id: None,
            auto_save_id: None,
        }
    }

    #[cfg(feature = "memory-test-api")]
    pub async fn perform_test_save(&mut self) -> Result<(), String> {
        self.perform_save().await
    }

    #[cfg(feature = "memory-test-api")]
    pub fn test_saved_revision(&self) -> Option<Vec<(i32, u64)>> {
        self.last_saved_fingerprint
            .as_ref()
            .map(|fingerprint| fingerprint.memory_revisions.clone())
    }

    /// Exit save: force a save regardless of change detection.
    async fn perform_exit_save(&mut self) -> Result<(), String> {
        // Reset hash to force save even if nothing changed
        self.invalidate_saved_fingerprint();
        self.perform_save().await
    }

    // ========== Helpers ==========

    /// Clear the target-slot success marker, for example after loading a save.
    pub fn invalidate_saved_fingerprint(&mut self) {
        self.last_saved_fingerprint = None;
        self.last_saved_save_id = None;
    }

    /// Capture a stable fingerprint and the exact immutable line snapshot it
    /// describes. The hash must be computed from the same lines passed to the
    /// repository, not from a second later lock acquisition.
    async fn compute_snapshot(
        &self,
    ) -> Option<(
        AutoSaveFingerprint,
        Vec<GameLine>,
        Vec<(i32, GameMemoryBank, u64)>,
    )> {
        let service = self.ai_service.lock().await;
        let status = service.game_status.lock().await;

        // 初始化时 line_list 自带一条 system 台词（角色人设），
        // 只有大于 1 条时才说明有实际对话发生，才需要自动存档。
        if status.line_list.len() <= 1 {
            return None;
        }

        let lines = status.line_list.clone();
        let mut hasher = DefaultHasher::new();
        for line in &lines {
            line.base.content.hash(&mut hasher);
            line.base.sender_role_id.hash(&mut hasher);
            line.base.attribute.as_str().hash(&mut hasher);
        }

        let memory_snapshots = status.role_manager.memory_bank_snapshots().await;
        Some((
            AutoSaveFingerprint {
                line_hash: hasher.finish(),
                memory_revisions: memory_snapshots
                    .iter()
                    .map(|(role_id, _, revision)| (*role_id, *revision))
                    .collect(),
            },
            lines,
            memory_snapshots,
        ))
    }

    /// Find the existing auto-save slot by title prefix, or create a new one.
    /// Updates the title with the current timestamp.
    async fn find_or_create_slot(&mut self) -> Result<i32, String> {
        // Try to find an existing auto-save by prefix
        // Read current main_role_id once (used in both branches)
        let main_id = {
            let service = self.ai_service.lock().await;
            let gs = service.game_status.lock().await;
            gs.main_role_id
        };

        if let Ok(Some(existing)) =
            SaveRepo::find_save_by_title_prefix(&self.db, AUTO_SAVE_PREFIX).await
        {
            let save_id = existing.id;
            let new_title = format!(
                "{} {}",
                AUTO_SAVE_PREFIX,
                Local::now().format("%Y-%m-%d %H:%M:%S")
            );
            SaveRepo::update_save_title(&self.db, save_id, &new_title)
                .await
                .map_err(|e| format!("更新自动存档标题失败: {}", e))?;
            // 每次存档都同步 main_role_id，防止切角色后指向旧角色
            SaveRepo::update_save_main_role(&self.db, save_id, main_id)
                .await
                .map_err(|e| format!("设置主角失败: {}", e))?;
            self.auto_save_id = Some(save_id);
            return Ok(save_id);
        }

        // Create a new auto-save slot
        let title = format!(
            "{} {}",
            AUTO_SAVE_PREFIX,
            Local::now().format("%Y-%m-%d %H:%M:%S")
        );
        let model = SaveRepo::create_save(&self.db, &title)
            .await
            .map_err(|e| format!("创建自动存档失败: {}", e))?;
        let save_id = model.id;

        SaveRepo::update_save_main_role(&self.db, save_id, main_id)
            .await
            .map_err(|e| format!("设置主角失败: {}", e))?;

        self.auto_save_id = Some(save_id);
        Ok(save_id)
    }
}
