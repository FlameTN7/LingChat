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

    // 0. 统一校验 + 归一化：先 trim，再检查长度与控制字符。
    //    允许 \n 与 \t（多行设定/排版），拒绝其余 C0 控制字符。
    let user_name = user_name.trim().to_string();
    if contains_forbidden_control(&user_name) {
        return Err("玩家昵称不能包含控制字符".to_string());
    }
    let name_chars = user_name.chars().count();
    if name_chars == 0 || name_chars > 32 {
        return Err("玩家昵称不能为空，且长度需为 1~32 个字符".to_string());
    }
    let user_subtitle = validate_optional_text("玩家副标题", user_subtitle, 64)?;
    let user_prompt = validate_optional_text("玩家设定", user_prompt, 4000)?;
    let info = validate_optional_text("玩家简介", info, 4000)?;
    let system_prompt_example = validate_optional_text("玩家说话风格示例", system_prompt_example, 4000)?;

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

    const ALLOWED_AVATAR_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp"];
    let ext = ext
        .map(|s| s.trim_start_matches('.').to_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "png".to_string());
    if !ALLOWED_AVATAR_EXTS.contains(&ext.as_str()) {
        return Err(format!(
            "头像格式不支持: .{}，仅支持 png/jpg/jpeg/webp/gif/bmp",
            ext
        ));
    }

    // 解码后的真实字节数才是占用磁盘/内存的大小，必须在落盘前校验。
    if data.len() > 10 * 1024 * 1024 {
        return Err("头像图片过大，解码后不能超过 10MB".to_string());
    }

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

/// 是否包含不允许的控制字符：仅放行 \n 与 \t，其余 C0 控制字符拒绝。
fn contains_forbidden_control(value: &str) -> bool {
    value
        .chars()
        .any(|c| (c as u32) < 0x20 && c != '\n' && c != '\t')
}

/// 校验可选文本字段：trim 首尾后返回归一化值；检查控制字符与字符数上限。
fn validate_optional_text(
    label: &str,
    value: Option<String>,
    max_chars: usize,
) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim().to_string();
    if contains_forbidden_control(&trimmed) {
        return Err(format!("{}不能包含除换行和制表符以外的控制字符", label));
    }
    if trimmed.chars().count() > max_chars {
        return Err(format!("{}长度不能超过 {} 个字符", label, max_chars));
    }
    Ok(Some(trimmed))
}
