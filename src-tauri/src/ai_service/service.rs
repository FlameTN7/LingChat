use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use sea_orm::DatabaseConnection;
use tokio::sync::Mutex;

use crate::ai_service::config::AIServiceConfig;
use crate::ai_service::game_system::game_status::GameStatus;
use crate::ai_service::game_system::role_manager::GameRoleManager;
use crate::ai_service::game_system::script_engine::ScriptManager;
use crate::ai_service::llm::LlmSlot;
use crate::ai_service::memory::{MemoryConfig, MemorySectionLimits};
use crate::ai_service::tts::local::LocalTtsRuntime;
use crate::ai_service::types::{
    CharacterSettings, GameLine, GameMemoryBank, LineAttributeExt, LineBase, ScriptStatus,
};
use crate::config::tts::TtsConfig;
use crate::db::entities::line::LineAttribute;
use crate::db::managers::memory_repo::MemoryRepo;
use crate::db::managers::save_repo::SaveRepo;
use crate::utils::prompt::{PromptOptions, sys_prompt_builder};

/// AI 服务：承载 `GameStatus` 与会话级配置。
///
/// 本轮仅实现 Python 版 `AIService` 中与状态管理相关的部分：
/// import_settings / init_game_status / get_lines / load_lines /
/// reset_lines / clear_lines / set_active_save_id。
/// 消息生成（MessageGenerator）、主动对话（ProactiveSystem）、剧本引擎（ScriptManager）
/// 等子系统按计划稍后补。
/// Immutable session state captured before any save I/O starts.
///
/// The snapshot deliberately does not include a save id: persistence belongs to
/// the caller's target slot, while this data can safely be written to manual or
/// auto-save slots without sharing dirty state between them.
#[derive(Clone)]
pub struct SessionSnapshot {
    pub lines: Vec<GameLine>,
    pub main_role_id: Option<i32>,
    pub status: crate::ai_service::game_system::game_status::GameStatusSnapshot,
    pub memory_banks: Vec<(i32, GameMemoryBank, u64)>,
    pub running_script: Option<ScriptStatus>,
}

pub struct AIService {
    pub db: DatabaseConnection,
    pub data_dir: PathBuf,
    pub game_status: Arc<Mutex<GameStatus>>,
    pub config: AIServiceConfig,

    pub init_character_id: Option<i32>, // 注释：这个是用于标记游戏状态初始化角色的
    pub prompt_options: Option<PromptOptions>, // 记录角色提示词构成方式选项

    /// Script/story mode engine: discovers and runs scripts.
    pub script_manager: ScriptManager,
}

impl AIService {
    pub async fn new(
        db: DatabaseConnection,
        data_dir: PathBuf,
        llm: LlmSlot,
        tts_config: TtsConfig,
        local_tts: Option<LocalTtsRuntime>,
        use_persistent_memory: bool,
        memory_update_interval: u32,
        memory_recent_window: u32,
        memory_limits: MemorySectionLimits,
    ) -> Self {
        // Initialize the event handler registry before any script is run
        crate::ai_service::game_system::script_engine::init_event_registry();

        let role_manager = GameRoleManager::new(
            data_dir.clone(),
            db.clone(),
            llm,
            tts_config,
            local_tts,
            MemoryConfig {
                enabled: use_persistent_memory,
                update_interval: memory_update_interval as usize,
                recent_window: memory_recent_window as usize,
                limits: memory_limits,
            },
        );
        let game_status = Arc::new(Mutex::new(GameStatus::new(role_manager)));
        let script_manager = ScriptManager::new(&data_dir);
        Self {
            db,
            data_dir,
            game_status,
            config: AIServiceConfig::default(),
            init_character_id: None,
            prompt_options: None,
            script_manager,
        }
    }

    pub async fn set_clothes_overrides(&mut self, overrides: HashMap<i32, String>) {
        let mut gs = self.game_status.lock().await;
        gs.role_manager.set_clothes_overrides(overrides);
    }

    pub async fn init_game_intro_character(
        &mut self,
        character_id: Option<i32>,
        prompt_options: PromptOptions,
    ) -> Result<()> {
        self.init_character_id = character_id;
        self.prompt_options = Some(prompt_options);

        let default_prompt =
            "你的信息被设置错误了，请你在接下来的对话中提示用户检查配置信息".to_string();

        let cid = match character_id {
            Some(v) => v,
            None => {
                tracing::error!("初始化游戏主角失败，未指定角色ID。");
                return Ok(());
            },
        };

        tracing::info!("正在初始化的角色id是: {:?}", cid);

        let mut gs = self.game_status.lock().await;

        let settings = gs
            .role_manager
            .get_role(&self.db, cid)
            .await?
            .settings
            .clone();

        let ai_prompt = sys_prompt_builder(
            &settings.user_name.clone(),
            &settings.ai_name.clone(),
            &settings.system_prompt.clone().unwrap_or(default_prompt),
            settings.system_prompt_example.clone().as_deref(),
            settings.system_prompt_example_old.clone().as_deref(),
            prompt_options,
        );
        gs.player.user_name = settings.user_name.clone();
        gs.player.user_subtitle = settings.user_subtitle.clone().unwrap_or_default();

        // 此处是初始角色被注册的地方
        let _ = gs.get_role(&self.db, cid).await?;
        gs.current_role_id = Some(cid);
        gs.onstage_role(cid);
        gs.main_role_id = Some(cid);

        // 若恢复的服装不是默认服装，生成换装旁白
        let clothes = gs
            .role_manager
            .get_loaded(cid)
            .map(|r| r.current_clothes.clone())
            .unwrap_or_default();
        tracing::info!("外部获取的当前服装是: {:?}", clothes);
        if clothes != "default" && !clothes.is_empty() {
            // 不是你个傻逼 AI 角色服装已经换过了你再他妈比较那台词表能变吗我草你的？，已修复
            let _ = gs
                .add_character_clothes_change_line(&self.db, cid, &clothes)
                .await;
        }

        let system_line = LineBase {
            content: ai_prompt.clone(),
            attribute: LineAttributeExt(LineAttribute::System),
            sender_role_id: Some(cid),
            display_name: Some(settings.ai_name.clone()),
            ..Default::default()
        };
        gs.add_line(&self.db, system_line).await?;

        Ok(())
    }

    pub async fn init_game_status(
        &mut self,
        cid: Option<i32>,
        prompt_options: PromptOptions,
    ) -> Result<()> {
        self.clear_game_status().await;
        self.init_game_intro_character(cid, prompt_options).await?;
        Ok(())
    }

    pub async fn reset_game_status(&mut self) -> Result<()> {
        self.clear_game_status().await;
        let prompt_options = match self.prompt_options {
            None => PromptOptions {
                output_sec_lang: true,
                no_emotion_limit: true,
            },
            Some(p) => p,
        };
        self.init_game_intro_character(self.init_character_id, prompt_options)
            .await?;
        Ok(())
    }

    async fn clear_game_status(&mut self) {
        let mut gs = self.game_status.lock().await;
        // Reset drops all role runtimes; join owned jobs first so a detached
        // remote compaction cannot outlive this session reset.
        gs.role_manager.abort_memory_updates().await;
        gs.role_manager.reset_roles();
        gs.line_list.clear();
        gs.onstage_role_ids.clear();
        gs.present_role_ids.clear();
        gs.player_entered = false;
    }

    pub async fn set_active_save_id(&mut self, save_id: Option<i32>) {
        self.game_status.lock().await.active_save_id = save_id;
    }

    /// 载入存档台词并恢复 MemoryBank。
    pub async fn load_lines(
        &mut self,
        lines: Vec<GameLine>,
        main_role_id: i32,
        save_id: Option<i32>,
    ) -> Result<()> {
        {
            let mut gs = self.game_status.lock().await;
            // The bank was restored for this save immediately before loading
            // lines. Invalidate any in-flight job, but retain the restored bank
            // and its processed pointer; this replacement is not a destructive
            // rewrite of an already loaded save.
            gs.role_manager.invalidate_memory_history();
            gs.line_list = lines;
            if let Some(sid) = save_id {
                gs.active_save_id = Some(sid);
            }
            // `restore_memory_banks` installed this save's immutable bank just
            // before lines were replaced. Rebuild contexts without resetting it.
            gs.rebuild_memories_after_restore(&self.db).await?;
            let _ = gs.get_role(&self.db, main_role_id).await?;
            gs.current_role_id = Some(main_role_id);
            gs.main_role_id = Some(main_role_id);
        }
        Ok(())
    }

    /// Whether the shared status currently hosts an editor preview rather than
    /// the canonical player session. This is diagnostic-only; persistence must
    /// use `capture_formal_session_snapshot` so the answer cannot become stale.
    pub async fn is_preview_session(&self) -> bool {
        self.game_status
            .lock()
            .await
            .role_manager
            .is_memory_preview()
    }

    /// Acquire the common preview/formal-operation gate. The returned guard
    /// must stay alive through every DB/file effect. Preview transitions acquire
    /// the same gate before changing mode, so either a formal operation finishes
    /// first or preview starts first and the operation has zero side effects.
    ///
    /// The async gate is independent of `RoleMemoryState`; no synchronous lock
    /// is held across `.await`, and callers acquire it before GameStatus.
    pub async fn acquire_formal_session_gate(&self) -> Result<tokio::sync::OwnedMutexGuard<()>> {
        let gate = self.game_status.lock().await.preview_session_gate();
        let guard = gate.lock_owned().await;
        if self
            .game_status
            .lock()
            .await
            .role_manager
            .is_memory_preview()
        {
            anyhow::bail!("试玩期间不能保存正式会话");
        }
        Ok(guard)
    }

    /// Capture the immutable snapshot while a caller holds the formal-session
    /// gate from `acquire_formal_session_gate`.
    pub(crate) async fn capture_guarded_session_snapshot(&self) -> SessionSnapshot {
        let status = self.game_status.lock().await;
        SessionSnapshot {
            lines: status.line_list.clone(),
            main_role_id: status.main_role_id,
            status: status.to_snapshot(),
            memory_banks: status.role_manager.memory_bank_snapshots(),
            running_script: status.script_status.clone(),
        }
    }

    /// Acquire the common gate and capture an immutable formal-session snapshot.
    pub async fn capture_formal_session_snapshot(
        &self,
    ) -> Result<(tokio::sync::OwnedMutexGuard<()>, SessionSnapshot)> {
        let guard = self.acquire_formal_session_gate().await?;
        let snapshot = self.capture_guarded_session_snapshot().await;
        Ok((guard, snapshot))
    }

    /// Persist a previously captured immutable snapshot to one save slot.
    ///
    /// Callers must retain the guard returned by
    /// `capture_formal_session_snapshot` through this method. This retains the
    /// repository ordering and partial-write behavior of the existing system;
    /// a transaction is intentionally not introduced here.
    pub(crate) async fn write_session_snapshot(
        &mut self,
        save_id: i32,
        snapshot: &SessionSnapshot,
    ) -> Result<Vec<(i32, u64)>> {
        SaveRepo::sync_lines(&self.db, save_id, &snapshot.lines).await?;
        SaveRepo::update_save_main_role(&self.db, save_id, snapshot.main_role_id).await?;
        SaveRepo::update_save_status(&self.db, save_id, &serde_json::to_string(&snapshot.status)?)
            .await?;

        let mut revisions = Vec::with_capacity(snapshot.memory_banks.len());
        for (role_id, bank, revision) in &snapshot.memory_banks {
            MemoryRepo::upsert_for_save_role(&self.db, save_id, *role_id, bank).await?;
            revisions.push((*role_id, *revision));
        }

        if let Some(script) = &snapshot.running_script {
            SaveRepo::upsert_running_script(
                &self.db,
                save_id,
                &script.folder_key,
                &serde_json::to_string(&script.vars)?,
                &script.current_chapter_key,
                script.current_event_process,
            )
            .await?;
        } else {
            // A snapshot is authoritative: retaining an old running script when
            // the runtime has none would resurrect a completed/stopped script
            // on the next load. The repository clears both the link and row.
            SaveRepo::clear_running_script_for_save(&self.db, save_id).await?;
        }
        Ok(revisions)
    }

    /// Capture and write the current session, then mark the target as active
    /// only after the snapshot's required writes succeeded.
    pub(crate) async fn persist_captured_formal_session(
        &mut self,
        save_id: i32,
        snapshot: &SessionSnapshot,
    ) -> Result<Vec<(i32, u64)>> {
        let revisions = self.write_session_snapshot(save_id, snapshot).await?;
        self.game_status.lock().await.active_save_id = Some(save_id);
        Ok(revisions)
    }

    pub async fn save_current_session(&mut self, save_id: i32) -> Result<Vec<(i32, u64)>> {
        let (_formal_gate, snapshot) = self.capture_formal_session_snapshot().await?;
        self.persist_captured_formal_session(save_id, &snapshot)
            .await
    }

    /// 从 DB 恢复所有 MemoryBank 到对应已加载角色，并惰性创建压缩系统。
    pub async fn restore_memory_banks(&mut self, save_id: i32) -> Result<()> {
        self.game_status
            .lock()
            .await
            .role_manager
            .load_memory_banks_from_db(&self.db, save_id, None)
            .await
    }

/// 辅助函数，用于快速获取人物的设定
    pub async fn get_role_settings_by_id(&self, role_id: i32) -> Result<CharacterSettings> {
        Ok(self
            .game_status
            .lock()
            .await
            .role_manager
            .get_role(&self.db, role_id)
            .await?
            .settings
            .clone())
    }
    /// Roll back through the canonical history API and, when a save is active,
    /// write the complete immutable session snapshot. This is shared by the
    /// Tauri command and feature-gated regression tests so lines and
    /// MemoryBank cannot diverge after a crash/reload.
    pub async fn rollback_conversation(&mut self, message_seq: u32) -> Result<Vec<GameLine>> {
        // Admission must precede the first history/memory mutation, even when
        // no save is active. Retain the permit through snapshot persistence so
        // Preview cannot turn a partially applied rollback into a rejected save.
        let _formal_gate = self.acquire_formal_session_gate().await?;
        let active_save_id = {
            let mut gs = self.game_status.lock().await;
            let mut count = 0_u32;
            let idx = gs
                .line_list
                .iter()
                .position(|line| {
                    if line.base.sender_role_id == Some(0)
                        && matches!(line.attribute(), LineAttribute::User)
                    {
                        count += 1;
                        count == message_seq
                    } else {
                        false
                    }
                })
                .ok_or_else(|| anyhow::anyhow!("未找到序号为 {} 的用户消息", message_seq))?;
            gs.truncate_lines(&self.db, idx).await?;
            gs.active_save_id
        };
        if let Some(save_id) = active_save_id {
            // Already admitted: do not reacquire the non-reentrant session gate.
            let snapshot = self.capture_guarded_session_snapshot().await;
            self.persist_captured_formal_session(save_id, &snapshot)
                .await?;
        }
        Ok(self.game_status.lock().await.line_list.clone())
    }
}

/// 在 Tauri managed state 中共享的句柄。
pub type SharedAIService = Arc<Mutex<AIService>>;
