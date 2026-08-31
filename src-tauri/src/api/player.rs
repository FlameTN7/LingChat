use base64::Engine;
use tauri::{AppHandle, Emitter, Manager};

use crate::AppState;
use crate::ai_service::game_system::player_profile_sync::rebuild_system_lines;
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
        // 只加一次 AI 服务锁，在其内部依次锁 GameStatus 并完成全部状态修改，
        // 避免持有不同层级的锁跨 await 时出现顺序死锁。
        let mut svc = state.ai_service.lock().await;

        {
            let mut gs = svc.game_status.lock().await;
            gs.player.user_name = profile.user_name.clone();
            gs.player.user_subtitle = profile.user_subtitle.clone().unwrap_or_default();
            gs.player.user_prompt = player_prompt.clone();

            // 改名/改设定热更新：按新玩家档案整体重建所有 System 人设行，
            // 而不是用裸字符串替换（会误伤短名，如「你」「A」）。
            rebuild_system_lines(&state.db, &svc.data_dir, &mut gs, prompt_options)
                .await
                .map_err(|e| format!("重建 System 人设行失败: {e}"))?;
            gs.role_manager.invalidate_memory_history();
            gs.refresh_memories(&state.db)
                .await
                .map_err(|e| format!("刷新角色记忆失败: {e}"))?;
        }

        // 用新玩家名 + 玩家设定重建 AI 服务自身的系统提示词快照。
        svc.user_name = profile.user_name.clone();
        svc.user_subtitle = profile.user_subtitle.clone();
        svc.player_prompt = player_prompt.clone();

        if let Some(settings) = svc.settings.clone() {
            svc.ai_prompt = sys_prompt_builder_by_settings(
                &settings,
                Some(&profile.user_name),
                prompt_options,
                &player_prompt,
            );
        }
    }

    // 锁释放后再广播事件，避免事件回调（其他窗口）等待后端锁造成串行等待。
    // 多窗口同步：主窗口与设置窗口都会收到此事件并刷新本地玩家档案展示。
    let avatar_path = PlayerProfileRepo::avatar_abs_path().map(|p| p.to_string_lossy().into_owned());
    let _ = app.emit(
        "player-profile-updated",
        serde_json::json!({
            "user_name": profile.user_name,
            "user_subtitle": profile.user_subtitle.unwrap_or_default(),
            "user_prompt": profile.user_prompt.unwrap_or_default(),
            "info": profile.info.unwrap_or_default(),
            "system_prompt_example": profile.system_prompt_example.unwrap_or_default(),
            "avatar_path": avatar_path,
        }),
    );

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
