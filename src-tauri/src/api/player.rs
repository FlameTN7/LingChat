use tauri::{AppHandle, Manager};

use crate::AppState;
use crate::db::managers::player_profile_repo::PlayerProfileRepo;
use crate::utils::prompt::{sys_prompt_builder_by_settings, PromptOptions};

/// 读取全局玩家档案。
#[tauri::command]
pub async fn get_player_profile(app: AppHandle) -> Result<serde_json::Value, String> {
    let state = app.state::<AppState>();
    let profile = PlayerProfileRepo::get_profile(&state.db)
        .await
        .map_err(|e| format!("读取玩家档案失败: {e}"))?;
    Ok(serde_json::json!({
        "user_name": profile.user_name,
        "user_subtitle": profile.user_subtitle.unwrap_or_default(),
        "user_prompt": profile.user_prompt.unwrap_or_default(),
    }))
}

/// 保存全局玩家档案。
///
/// 解耦玩家与 AI：保存到 `player_profile` 表的同时，**同步运行时**
/// （`GameStatus.player` 与 AI 系统提示词），这样玩家改名/改设定后
/// LLM 立即感知，不会继续用旧的默认"玩家"。
#[tauri::command]
pub async fn set_player_profile(
    app: AppHandle,
    user_name: String,
    user_subtitle: Option<String>,
    user_prompt: Option<String>,
) -> Result<serde_json::Value, String> {
    let state = app.state::<AppState>();

    // 1. 持久化到数据库
    PlayerProfileRepo::save_profile(&state.db, user_name.clone(), user_subtitle.clone(), user_prompt.clone())
        .await
        .map_err(|e| format!("保存玩家档案失败: {e}"))?;

    // 2. 同步运行时 + 重建系统提示词（让 LLM 立即感知新名字/玩家设定）
    let app_config = crate::config::AppConfig::load(&app).unwrap_or_default();
    let prompt_options = PromptOptions {
        output_sec_lang: app_config.llm_output_sec_lang,
        no_emotion_limit: app_config.no_emotion_limit_prompt,
    };

    {
        let svc = state.ai_service.lock().await;

        // 2a. 更新 GameStatus.player，并替换历史 System 人设行里的旧玩家名
        {
            let mut gs = svc.game_status.lock().await;
            let old_name = gs.player.user_name.clone();
            gs.player.user_name = user_name.clone();
            gs.player.user_subtitle = user_subtitle.clone().unwrap_or_default();
            gs.player.user_prompt = user_prompt.clone().unwrap_or_default();

            // 已注入的 System 人设行里 framing 嵌入了玩家名，改名后把这些行的旧名替换成新名，
            // 避免 LLM 的 system 消息仍带着旧玩家名（用户反馈的"LLM 还是接到玩家"根因之一）。
            if !old_name.is_empty() {
                for line in &mut gs.line_list {
                    if matches!(
                        line.attribute(),
                        crate::db::entities::line::LineAttribute::System
                    ) {
                        line.base.content = line.base.content.replace(&old_name, &user_name);
                    }
                }
            }
        }

        // 2b. 用新玩家名 + 玩家设定重建 AI 系统提示词（需要 &mut 修改 svc 字段）
        let mut svc = svc;
        svc.user_name = user_name.clone();
        svc.user_subtitle = user_subtitle.clone();
        svc.player_prompt = user_prompt.clone().unwrap_or_default();

        if let Some(settings) = svc.settings.clone() {
            let player_prompt = user_prompt.clone().unwrap_or_default();
            svc.ai_prompt = sys_prompt_builder_by_settings(
                &settings,
                Some(&user_name),
                prompt_options,
                &player_prompt,
            );
        }
    }

    Ok(serde_json::json!({"success": true}))
}
