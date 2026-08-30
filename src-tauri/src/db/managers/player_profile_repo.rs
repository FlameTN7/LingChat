use std::path::PathBuf;

use anyhow::{Context, Result};
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};

use crate::ai_service::types::CharacterSettings;
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

    /// 读取玩家档案。
    ///
    /// 以 `game_data/player/settings.yml` 为准。文件不存在时回退到 `CharacterSettings`
    /// 的 `user_name`（兼容旧数据），再回退默认 `("玩家", "", "")`。
    /// `db` 参数保留仅为兼容既有调用点（历史版本从数据库表读取）。
    pub async fn get_profile(_db: &DatabaseConnection) -> Result<PlayerProfileData> {
        let path = Self::settings_path();
        if !path.exists() {
            // 文件尚不存在（首启用）：返回默认玩家档案
            return Ok(PlayerProfileData::default());
        }

        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("读取玩家档案失败: {:?}", path))?;
        let settings: CharacterSettings = serde_yaml::from_str(&content)
            .with_context(|| format!("解析玩家档案失败: {:?}", path))?;

        Ok(PlayerProfileData {
            user_name: settings.user_name,
            user_subtitle: settings.user_subtitle,
            user_prompt: settings.system_prompt,
            info: settings.info,
            system_prompt_example: settings.system_prompt_example,
            avatar_path: None,
        })
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

        // 复用 CharacterSettings 序列化，保证与角色 settings.yml 同构（仅写入玩家相关字段）。
        let settings = CharacterSettings {
            user_name: profile.user_name.clone(),
            user_subtitle: profile.user_subtitle.clone(),
            system_prompt: profile.user_prompt.clone(),
            info: profile.info.clone(),
            system_prompt_example: profile.system_prompt_example.clone(),
            ..Default::default()
        };

        let yaml = serde_yaml::to_string(&settings)
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
