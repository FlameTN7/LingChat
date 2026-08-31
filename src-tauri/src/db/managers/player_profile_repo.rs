use std::path::PathBuf;

use anyhow::{Context, Result};
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};

use crate::ai_service::types::CharacterSettings;
use crate::db::managers::role_repo::RoleRepo;
use crate::init::static_copy::get_data_dir;

/// 玩家档案数据（文件驱动）。字段与前端 `PlayerProfile` 接口对齐。
///
/// 解耦玩家与 AI：玩家身份独立存储在 `game_data/player/settings.yml`，
/// 不再写入各 AI 角色的 settings.yml，也不依赖数据库表。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerProfileData {
    #[serde(default = "default_user_name")]
    pub user_name: String,
    #[serde(default)]
    pub user_subtitle: Option<String>,
    #[serde(default)]
    pub user_prompt: Option<String>,
    /// 简介 / 一句话人设（类似角色卡的 `info`）。
    #[serde(default)]
    pub info: Option<String>,
    /// 说话风格示例（类似角色卡的 `system_prompt_example`）。
    #[serde(default)]
    pub system_prompt_example: Option<String>,
    /// 玩家头像文件名（相对 `game_data/player/` 目录，如 `头像.png`）。
    #[serde(default)]
    pub avatar_path: Option<String>,
}

fn default_user_name() -> String {
    "玩家".to_string()
}

/// 归一化旧角色卡中的文本字段：过滤空串、纯空白和 serde 缺省值。
fn normalize_legacy_text(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "user_name未设定" {
        return None;
    }
    Some(trimmed)
}

impl Default for PlayerProfileData {
    fn default() -> Self {
        Self {
            user_name: default_user_name(),
            user_subtitle: None,
            user_prompt: None,
            info: None,
            system_prompt_example: None,
            avatar_path: None,
        }
    }
}

impl PlayerProfileData {
    /// 把玩家档案的「设定块」合并成一段文本，注入系统提示词。
    ///
    /// 组合顺序：简介（info）→ 人格设定（user_prompt）→ 说话风格示例（system_prompt_example）。
    /// 与角色卡的 `info` / `system_prompt` / `system_prompt_example` 语义一致，
    /// 让 AI 更完整地了解屏幕对面的真实用户。空字段自动跳过。
    pub fn to_prompt_fragment(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(s) = self.info.as_deref().filter(|s| !s.is_empty()) {
            parts.push(format!("【简介】{}", s));
        }
        if let Some(s) = self.user_prompt.as_deref().filter(|s| !s.is_empty()) {
            parts.push(format!("【人格设定】{}", s));
        }
        if let Some(s) = self.system_prompt_example.as_deref().filter(|s| !s.is_empty()) {
            parts.push(format!("【说话风格示例】\n{}", s));
        }
        parts.join("\n")
    }
}

/// 玩家档案仓库：文件驱动（`game_data/player/settings.yml`）。
///
/// 存储方式与 AI 角色卡对齐：一个 `player/` 目录 + 一个 `settings.yml` + 一张头像。
/// 玩家身份（名字/副标题/简介/人格设定/说话示例）全部由此文件承载。
pub struct PlayerProfileRepo;

impl PlayerProfileRepo {
    /// 玩家目录：`game_data/player/`（与 `characters/` 平级，避免被 role_sync 误扫为 AI 角色）。
    fn player_dir() -> PathBuf {
        get_data_dir().join("game_data").join("player")
    }

    /// 玩家 settings.yml 路径。
    fn settings_path() -> PathBuf {
        Self::player_dir().join("settings.yml")
    }

    /// 读取玩家档案（兼容旧数据的便捷包装）。
    ///
    /// 实际逻辑见 [`Self::ensure_profile`]，此处等价于 `ensure_profile(db, None)`。
    pub async fn get_profile(db: &DatabaseConnection) -> Result<PlayerProfileData> {
        Self::ensure_profile(db, None).await
    }

    /// 确保玩家档案存在并返回可用档案。
    ///
    /// - 文件存在时照旧解析 `game_data/player/settings.yml`；
    /// - 文件不存在时尝试从旧 AI 角色卡迁移 `user_name/user_subtitle`：
    ///   优先使用调用方传入的 `fallback`，没有可用的 fallback 时再查数据库里的
    ///   第一个主角色；旧卡上的 `system_prompt` 是 AI 角色人设，绝不能当成
    ///   玩家的 `user_prompt` 迁移，避免 AI 人设"污染"玩家档案。
    /// - 迁移成功后写入 `game_data/player/settings.yml`；写入失败只告警并继续
    ///   返回内存中的迁移结果，不阻断启动，下次启动会重新尝试迁移。
    pub async fn ensure_profile(
        db: &DatabaseConnection,
        fallback: Option<&CharacterSettings>,
    ) -> Result<PlayerProfileData> {
        let path = Self::settings_path();
        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("读取玩家档案失败: {:?}", path))?;
            let settings: CharacterSettings = serde_yaml::from_str(&content)
                .with_context(|| format!("解析玩家档案失败: {:?}", path))?;

            return Ok(PlayerProfileData {
                user_name: settings.user_name,
                user_subtitle: settings.user_subtitle,
                user_prompt: settings.system_prompt,
                info: settings.info,
                system_prompt_example: settings.system_prompt_example,
                avatar_path: None,
            });
        }

        // 文件尚不存在：进入旧玩家数据自动迁移路径。
        let migrated = fallback.and_then(Self::extract_legacy_profile_fields);

        // fallback 不可用时，从数据库主角色里找第一张含有效旧玩家字段的角色卡。
        // 注意：这里只读取旧卡字段用于迁移，不修改/删除旧卡上的任何字段。
        let (user_name, user_subtitle) = if let Some((name, subtitle)) = migrated {
            (name, subtitle)
        } else {
            let mut found: Option<(String, Option<String>)> = None;
            match RoleRepo::get_all_main_roles(db).await {
                Ok(roles) => {
                    for role in roles {
                        match RoleRepo::get_role_settings_by_id(db, get_data_dir(), role.id).await {
                            Ok(Some(settings)) => {
                                if let Some(pair) = Self::extract_legacy_profile_fields(&settings) {
                                    found = Some(pair);
                                    break;
                                }
                            }
                            Ok(None) => {}
                            Err(e) => {
                                tracing::warn!("读取主角色设置以迁移玩家档案失败: role_id={}, {e}", role.id);
                            }
                        }
                    }
                }
                Err(e) => tracing::warn!("查询主角色列表以迁移玩家档案失败: {e}"),
            }
            found.unwrap_or_else(|| (PlayerProfileData::default().user_name, None))
        };

        let profile = PlayerProfileData {
            user_name,
            user_subtitle,
            ..Default::default()
        };

        // 迁移后立即落盘；失败不阻断启动，后续调用仍会再次尝试迁移。
        if let Err(e) = Self::save_profile(db, &profile).await {
            tracing::warn!("旧玩家数据迁移写入失败，本次会话继续使用内存中的迁移结果，下次启动会重试: {e}");
        } else {
            tracing::info!("已从旧 AI 角色卡迁移玩家档案并写入 {}", path.display());
        }

        Ok(profile)
    }

    /// 从旧 AI 角色卡中提取可迁移的玩家字段。
    ///
    /// 过滤空串、纯空白以及 serde 缺省值 `user_name未设定`；`system_prompt`
    /// 不会被带到这里（那是 AI 人设，不是玩家设定）。
    fn extract_legacy_profile_fields(
        settings: &CharacterSettings,
    ) -> Option<(String, Option<String>)> {
        let user_name = normalize_legacy_text(&settings.user_name)?;
        let user_subtitle = settings
            .user_subtitle
            .as_deref()
            .and_then(normalize_legacy_text)
            .map(|s| s.to_string());
        Some((user_name, user_subtitle))
    }

    /// 保存玩家档案到 `game_data/player/settings.yml`。
    ///
    /// 写入的字段复用 `CharacterSettings` 的相关字段（user_name / user_subtitle /
    /// system_prompt / info / system_prompt_example），与角色卡存储格式一致。
    pub async fn save_profile(
        _db: &DatabaseConnection,
        profile: &PlayerProfileData,
    ) -> Result<()> {
        let dir = Self::player_dir();
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("创建玩家目录失败: {:?}", dir))?;

        // 复用 CharacterSettings 的相关字段，但只序列化玩家关心的键，
        // 避免把 ai_name/scale 等 AI 角色默认值混入玩家 settings.yml。
        let mut obj = serde_json::Map::new();
        obj.insert("user_name".to_string(), serde_json::json!(profile.user_name));
        if let Some(s) = profile.user_subtitle.as_deref().filter(|s| !s.is_empty()) {
            obj.insert("user_subtitle".to_string(), serde_json::json!(s));
        }
        if let Some(s) = profile.user_prompt.as_deref().filter(|s| !s.is_empty()) {
            obj.insert("system_prompt".to_string(), serde_json::json!(s));
        }
        if let Some(s) = profile.info.as_deref().filter(|s| !s.is_empty()) {
            obj.insert("info".to_string(), serde_json::json!(s));
        }
        if let Some(s) = profile.system_prompt_example.as_deref().filter(|s| !s.is_empty()) {
            obj.insert("system_prompt_example".to_string(), serde_json::json!(s));
        }

        let yaml = serde_yaml::to_string(&obj)
            .context("序列化玩家档案失败")?;
        let path = Self::settings_path();
        std::fs::write(&path, yaml)
            .with_context(|| format!("写入玩家档案失败: {:?}", path))?;

        Ok(())
    }

    /// 保存玩家头像文件（写入 `game_data/player/` 目录）。
    /// 文件名固定为 `头像.<ext>`，与角色卡头像命名惯例一致。
    pub fn save_avatar(data: &[u8], ext: &str) -> Result<String> {
        let dir = Self::player_dir();
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("创建玩家目录失败: {:?}", dir))?;

        let ext = if ext.is_empty() { "png" } else { ext };
        let filename = format!("头像.{}", ext);
        let path = dir.join(&filename);
        std::fs::write(&path, data)
            .with_context(|| format!("写入玩家头像失败: {:?}", path))?;

        Ok(filename)
    }

    /// 玩家头像绝对路径（存在则返回，否则返回 None）。
    pub fn avatar_abs_path() -> Option<PathBuf> {
        let dir = Self::player_dir();
        for ext in &["png", "jpg", "jpeg", "webp", "gif", "bmp"] {
            let p = dir.join(format!("头像.{}", ext));
            if p.exists() {
                return Some(p);
            }
        }
        // 兜底：目录内以"头像"开头的任意图片
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("头像")
                    && !entry.file_type().map(|t| t.is_dir()).unwrap_or(true)
                {
                    return Some(entry.path());
                }
            }
        }
        None
    }
}
