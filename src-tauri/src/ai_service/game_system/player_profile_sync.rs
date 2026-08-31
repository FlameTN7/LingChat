use std::path::Path;

use anyhow::Result;
use sea_orm::DatabaseConnection;
use tauri::Manager;

use crate::ai_service::game_system::game_status::GameStatus;
use crate::ai_service::game_system::script_engine::events::ScriptContext;
use crate::ai_service::game_system::script_engine::responses::{
    PlayerIdentityPayload, event_names::SCRIPT_PLAYER_IDENTITY,
};
use crate::ai_service::message_system::events::emit;
use crate::ai_service::types::{CharacterSettings, IdentityScope, Player};
use crate::config::AppConfig;
use crate::db::entities::line::LineAttribute;
use crate::db::managers::player_profile_repo::PlayerProfileRepo;
use crate::db::managers::role_repo::RoleRepo;
use crate::utils::prompt::{sys_prompt_builder_by_settings, PromptOptions};

/// 用当前玩家档案整体重建 line_list 中的 System 人设行。
///
/// 改名/改设定后，历史 System 行里嵌着旧玩家名和旧玩家设定块，单纯字符串
/// 替换会误伤短名（如「你」「A」），因此这里按角色重新调用提示词构造器，
/// 只替换 `base.content`，保留 `base.id` 等其它字段（持久化存档仍按原 id 更新）。
///
/// 返回实际重建的 System 行数量。
pub async fn rebuild_system_lines(
    db: &DatabaseConnection,
    data_dir: &Path,
    gs: &mut GameStatus,
    prompt_options: PromptOptions,
) -> Result<usize> {
    // 先收集 System 行引用的唯一 sender_role_id，避免在后续可变遍历中反复查角色。
    let mut sender_role_ids: Vec<i32> = gs
        .line_list
        .iter()
        .filter(|line| line.attribute() == &LineAttribute::System)
        .filter_map(|line| line.base.sender_role_id)
        .collect();
    sender_role_ids.sort_unstable();
    sender_role_ids.dedup();

    // 每个 sender 角色最多查一次 settings：优先取内存中已加载的角色，再落库读盘。
    let mut settings_by_role: Vec<(i32, Option<CharacterSettings>)> = Vec::new();
    for rid in sender_role_ids {
        let loaded = gs
            .role_manager
            .get_loaded(rid)
            .map(|role| role.settings.clone());
        let settings = match loaded {
            Some(settings) => Some(settings),
            None => match RoleRepo::get_role_settings_by_id(db, data_dir, rid).await {
                Ok(settings) => settings,
                Err(e) => {
                    tracing::warn!("读取角色设置失败，跳过重建其 System 行: role_id={}, {e}", rid);
                    None
                }
            },
        };
        settings_by_role.push((rid, settings));
    }

    let mut rebuilt = 0usize;
    for line in &mut gs.line_list {
        if line.attribute() != &LineAttribute::System {
            continue;
        }
        let Some(rid) = line.base.sender_role_id else {
            continue;
        };
        let settings = settings_by_role
            .iter()
            .find_map(|(id, settings)| {
                if *id == rid {
                    settings.as_ref()
                } else {
                    None
                }
            });
        let Some(settings) = settings else {
            tracing::warn!("System 行缺少角色设置，保留原样: role_id={}", rid);
            continue;
        };

        // 整体重建内容：新玩家名 + 新玩家设定块 + 该角色的 AI 人设与格式提示。
        line.base.content = sys_prompt_builder_by_settings(
            settings,
            Some(&gs.player.user_name),
            prompt_options,
            &gs.player.user_prompt,
        );
        rebuilt += 1;
    }

    Ok(rebuilt)
}

// ============================================================
// set_player_identity：统一身份切换 / 还原逻辑
// ============================================================

/// 从应用配置读取提示词选项；读取失败时回退默认值并告警。
fn prompt_options_from_app(app: &tauri::AppHandle) -> PromptOptions {
    match AppConfig::load(app) {
        Ok(config) => PromptOptions {
            output_sec_lang: config.llm_output_sec_lang,
            no_emotion_limit: config.no_emotion_limit_prompt,
        },
        Err(e) => {
            tracing::warn!("读取应用配置失败，玩家身份重建回退默认提示词选项: {e}");
            PromptOptions::default()
        }
    }
}

/// 在**已持有 GameStatus 锁**的前提下重建 System 行并刷新角色记忆。
/// 本函数自身不再加锁，避免 tokio::Mutex 不可重入导致的死锁。
async fn rebuild_and_refresh(
    db: &DatabaseConnection,
    data_dir: &Path,
    gs: &mut GameStatus,
    prompt_options: PromptOptions,
) -> Result<()> {
    let rebuilt = rebuild_system_lines(db, data_dir, gs, prompt_options).await?;
    tracing::info!("玩家身份变化后重建了 {rebuilt} 条 System 人设行");
    gs.role_manager.invalidate_memory_history();
    gs.refresh_memories(db).await
}

/// 把身份字段应用到 Player。语义与 set_player_identity 事件一致：
/// - `user_name`：trim 后非空才覆盖；
/// - `user_subtitle` / `user_prompt`：键存在就覆盖，允许用空串清空。
fn apply_player_fields(
    player: &mut Player,
    user_name: Option<&str>,
    user_subtitle: Option<String>,
    user_prompt: Option<String>,
) {
    if let Some(name) = user_name {
        let name = name.trim();
        if !name.is_empty() {
            player.user_name = name.to_string();
        }
    }
    if let Some(subtitle) = user_subtitle {
        player.user_subtitle = subtitle;
    }
    if let Some(prompt) = user_prompt {
        player.user_prompt = prompt;
    }
}

/// 广播 `script:player-identity` 事件（在锁外调用）。
fn emit_identity_event(ctx: &ScriptContext<'_>, player: &Player, scope: IdentityScope) {
    let payload = PlayerIdentityPayload {
        user_name: player.user_name.clone(),
        user_subtitle: player.user_subtitle.clone(),
        user_prompt: player.user_prompt.clone(),
        scope: scope.as_str().to_string(),
    };
    if let Err(e) = emit(ctx.app, SCRIPT_PLAYER_IDENTITY, &payload) {
        tracing::warn!("广播玩家身份切换事件失败: {e}");
    }
}

/// 应用 `set_player_identity` 事件：切换/持久化玩家身份并让 LLM 上下文立即感知。
///
/// - chapter/script：先把当前 `gs.player` 连同生效 scope 压入快照栈，再应用字段；
///   之后 chapter_end / on_script_end 按 scope 弹出并还原。
/// - permanent：清空快照栈、写入全局玩家档案、同步 AppState.ai_service 的
///   快照字段与 AI 系统提示词；试玩预览中 permanent 降级为 script，
///   不写盘也不更新 AIService（试玩结束由整场还原兜底）。
pub async fn apply_player_identity(
    ctx: &mut ScriptContext<'_>,
    user_name: Option<String>,
    user_subtitle: Option<String>,
    user_prompt: Option<String>,
    scope: IdentityScope,
) -> Result<()> {
    let prompt_options = prompt_options_from_app(ctx.app);

    // 试玩是临时场次：permanent 也必须可还原，降级为 script 语义。
    let effective_scope = if ctx.is_preview && scope == IdentityScope::Permanent {
        tracing::warn!("试玩中 set_player_identity 的 permanent 已降级为 script，试玩结束会整体还原");
        IdentityScope::Script
    } else {
        scope
    };

    if effective_scope != IdentityScope::Permanent {
        let mut gs = ctx.game_status.lock().await;
        gs.player_identity_override
            .push((gs.player.clone(), effective_scope));
        apply_player_fields(
            &mut gs.player,
            user_name.as_deref(),
            user_subtitle,
            user_prompt,
        );
        rebuild_and_refresh(ctx.db, ctx.data_dir, &mut gs, prompt_options).await?;
        let player = gs.player.clone();
        drop(gs);
        emit_identity_event(ctx, &player, effective_scope);
        return Ok(());
    }

    // permanent（非试玩）：读取现有档案并只覆盖本次提供的字段，
    // 保留简介/说话示例/头像等其它档案内容。
    let mut profile = PlayerProfileRepo::get_profile(ctx.db).await?;
    let mut applied = Player {
        user_name: profile.user_name.clone(),
        user_subtitle: profile.user_subtitle.clone().unwrap_or_default(),
        user_prompt: profile.user_prompt.clone().unwrap_or_default(),
    };
    apply_player_fields(
        &mut applied,
        user_name.as_deref(),
        user_subtitle.clone(),
        user_prompt.clone(),
    );
    profile.user_name = applied.user_name;
    profile.user_subtitle = Some(applied.user_subtitle);
    profile.user_prompt = Some(applied.user_prompt);

    // 先写盘，再更新运行时；写盘失败直接返回错误，不留下“内存已生效但没持久化”的
    // 半永久状态。
    PlayerProfileRepo::save_profile(ctx.db, &profile).await?;
    let player_prompt_fragment = profile.to_prompt_fragment();
    let data_dir = ctx.data_dir.to_path_buf();

    {
        // 只加一次 AI 服务锁；GameStatus 是 AIService 的字段，不在这里再次
        // 单独 lock ctx.game_status（tokio::Mutex 不可重入，嵌套会死锁）。
        let mut svc = ctx.app.state::<crate::AppState>().ai_service.lock().await;
        {
            let mut gs = svc.game_status.lock().await;
            gs.player.user_name = profile.user_name.clone();
            gs.player.user_subtitle = profile.user_subtitle.clone().unwrap_or_default();
            // GameStatus.player.user_prompt 与 AIService.player_prompt 同形态：
            // 存格式化后的设定块（含简介/示例），不是档案里的原始 user_prompt 字段。
            gs.player.user_prompt = player_prompt_fragment.clone();
            gs.player_identity_override.clear();
            rebuild_and_refresh(ctx.db, &data_dir, &mut gs, prompt_options).await?;
        }

        svc.user_name = profile.user_name.clone();
        svc.user_subtitle = profile.user_subtitle.clone();
        svc.player_prompt = player_prompt_fragment.clone();
        if let Some(settings) = svc.settings.clone() {
            svc.ai_prompt = sys_prompt_builder_by_settings(
                &settings,
                Some(&profile.user_name),
                prompt_options,
                &player_prompt_fragment,
            );
        }
    }

    // 锁外发事件，避免其它窗口的事件处理立刻回头请求后端锁。
    let player = Player {
        user_name: profile.user_name.clone(),
        user_subtitle: profile.user_subtitle.clone().unwrap_or_default(),
        user_prompt: profile.user_prompt.clone().unwrap_or_default(),
    };
    emit_identity_event(ctx, &player, IdentityScope::Permanent);

    Ok(())
}

/// 章节结束：只弹出栈顶**连续**的 chapter 快照，script/permanent 不动。
///
/// 若确实有弹出，把玩家还原为最后弹出的那份原身份，并重建 System 行/刷新记忆。
pub async fn restore_player_identity_for_chapter(ctx: &mut ScriptContext<'_>) -> Result<()> {
    let prompt_options = prompt_options_from_app(ctx.app);

    let mut popped: Vec<(Player, IdentityScope)> = Vec::new();
    let player = {
        let mut gs = ctx.game_status.lock().await;
        while matches!(
            gs.player_identity_override.last(),
            Some((_, IdentityScope::Chapter))
        ) {
            if let Some(snapshot) = gs.player_identity_override.pop() {
                popped.push(snapshot);
            }
        }
        if popped.is_empty() {
            return Ok(());
        }

        // 最后弹出的快照 = 最早压入的 chapter 身份，还原到它就对了。
        let (original, _) = popped.pop().expect("popped 非空");
        gs.player = original;
        rebuild_and_refresh(ctx.db, ctx.data_dir, &mut gs, prompt_options).await?;
        gs.player.clone()
    };

    emit_identity_event(ctx, &player, IdentityScope::Chapter);
    Ok(())
}

/// 剧本结束：弹出所有 chapter 与 script 快照，还原最后弹出的原身份。
pub async fn restore_player_identity_for_script_end(ctx: &mut ScriptContext<'_>) -> Result<()> {
    let prompt_options = prompt_options_from_app(ctx.app);

    let mut popped: Vec<(Player, IdentityScope)> = Vec::new();
    let player = {
        let mut gs = ctx.game_status.lock().await;
        while matches!(
            gs.player_identity_override.last(),
            Some((_, IdentityScope::Chapter | IdentityScope::Script))
        ) {
            if let Some(snapshot) = gs.player_identity_override.pop() {
                popped.push(snapshot);
            }
        }
        if popped.is_empty() {
            return Ok(());
        }

        let (original, _) = popped.pop().expect("popped 非空");
        gs.player = original;
        rebuild_and_refresh(ctx.db, ctx.data_dir, &mut gs, prompt_options).await?;
        gs.player.clone()
    };

    emit_identity_event(ctx, &player, IdentityScope::Script);
    Ok(())
}
