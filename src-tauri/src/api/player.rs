use base64::Engine;
use tauri::{AppHandle, Manager};

use crate::AppState;
use crate::db::managers::player_profile_repo::{PlayerProfileData, PlayerProfileRepo};
use crate::utils::prompt::{sys_prompt_builder_by_settings, PromptOptions};

/// 读取全局玩家档案（文件驱动）。
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
        "info": profile.info.unwrap_or_default(),
        "system_prompt_example": profile.system_prompt_example.unwrap_or_default(),
        "avatar_path": PlayerProfileRepo::avatar_abs_path()
            .map(|p| p.to_string_lossy().into_owned()),
    }))
}

/// 保存全局玩家档案。
///
/// 解耦玩家与 AI：文件驱动写入 `game_data/player/settings.yml`，同时**同步运行时**
/// （`GameStatus.player` 与 AI 系统提示词），这样玩家改名/改设定后 LLM 立即感知，
/// 不会继续用旧的默认"玩家"。
#[tauri::command]
pub async fn set_player_profile(
    app: AppHandle,
    user_name: String,
    user_subtitle: Option<String>,
    user_prompt: Option<String>,
    info: Option<String>,
    system_prompt_example: Option<String>,
) -> Result<serde_json::Value, String> {
    let state = app.state::<AppState>();

    // 1. 持久化到文件（game_data/player/settings.yml）
    let profile = PlayerProfileData {
        user_name: user_name.clone(),
        user_subtitle,
        user_prompt,
        info,
        system_prompt_example,
        avatar_path: None,
    };
    PlayerProfileRepo::save_profile(&state.db, &profile)
        .await
        .map_err(|e| format!("保存玩家档案失败: {e}"))?;

    // 2. 同步运行时 + 重建系统提示词（让 LLM 立即感知新名字/玩家设定）
    let app_config = crate::config::AppConfig::load(&app).unwrap_or_default();
    let prompt_options = PromptOptions {
        output_sec_lang: app_config.llm_output_sec_lang,
        no_emotion_limit: app_config.no_emotion_limit_prompt,
    };

    let player_prompt = profile.to_prompt_fragment();

    {
        let svc = state.ai_service.lock().await;

        // 2a. 更新 GameStatus.player，并替换历史 System 人设行里的旧玩家名
        {
            let mut gs = svc.game_status.lock().await;
            let old_name = gs.player.user_name.clone();
            gs.player.user_name = user_name.clone();
            gs.player.user_subtitle = profile.user_subtitle.clone().unwrap_or_default();
            gs.player.user_prompt = player_prompt.clone();

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
        svc.user_subtitle = profile.user_subtitle.clone();
        svc.player_prompt = player_prompt.clone();

        if let Some(settings) = svc.settings.clone() {
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

/// 保存玩家头像（base64 图片数据写入 `game_data/player/头像.<ext>`）。
///
/// `ext` 为图片扩展名（不含点，如 "png"）。返回玩家头像绝对路径。
#[tauri::command]
pub async fn save_player_avatar(
    image_base64: String,
    ext: Option<String>,
) -> Result<serde_json::Value, String> {
    // base64 数据可能带 data URL 前缀（如 `data:image/png;base64,`），剥离之。
    let b64_payload = match image_base64.split_once(',') {
        Some((prefix, payload)) if prefix.starts_with("data:") => payload,
        _ => image_base64.as_str(),
    };
    let data = base64::engine::general_purpose::STANDARD
        .decode(b64_payload)
        .map_err(|e| format!("解码图片数据失败: {e}"))?;

    let ext = ext
        .map(|s| s.trim_start_matches('.').to_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "png".to_string());

    let filename = PlayerProfileRepo::save_avatar(&data, &ext)
        .map_err(|e| format!("保存玩家头像失败: {e}"))?;
    let abs = PlayerProfileRepo::avatar_abs_path()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    Ok(serde_json::json!({
        "success": true,
        "filename": filename,
        "avatar_path": abs,
    }))
}
