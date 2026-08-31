use std::path::Path;

use anyhow::Result;
use sea_orm::DatabaseConnection;

use crate::ai_service::game_system::game_status::GameStatus;
use crate::ai_service::types::CharacterSettings;
use crate::db::entities::line::LineAttribute;
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
            })
            .flatten();
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
