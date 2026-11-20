//! 资源同步核心逻辑 — 从 resource_dir 的 .official/ 复制文件到 data/。
//!
//! 仅涉及本地文件操作，不走网络。

use std::collections::HashMap;
use std::path::Path;

use tracing::info;

use crate::manifest::DataManifest;

use super::ResourceSyncResult;

/// 首次全量播种：将 .official/game_data/* 复制到 data/game_data/，
/// 以及 .official/data_manifest.json 复制到 data/data_manifest.json。
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn seed_full_from_official(data_dir: &Path, official_dir: &Path) -> anyhow::Result<()> {
    let game_data_src = official_dir.join("game_data");
    let game_data_dst = data_dir.join("game_data");
    let manifest_src = official_dir.join("data_manifest.json");
    let manifest_dst = data_dir.join("data_manifest.json");

    // 1. 复制 game_data/
    if game_data_src.exists() {
        copy_dir_recursive(&game_data_src, &game_data_dst)?;
        info!(
            "Seeded game_data from {} to {}",
            game_data_src.display(),
            game_data_dst.display()
        );
    }

    // 2. 复制 manifest
    if manifest_src.exists() {
        copy_user_file(&manifest_src, &manifest_dst)?;
        info!("Seeded manifest to {}", manifest_dst.display());
    }

    info!("Full seed from .official complete");
    Ok(())
}

/// 从官方资源目录复制用户选中的文件到 data/game_data/，
/// 更新本地 manifest，并在便携模式时删除本地 data/.official/。
pub fn apply_selected_files(
    data_dir: &Path,
    selected_files: &[String],
) -> anyhow::Result<ResourceSyncResult> {
    let (official_dir, is_writable_local) = crate::data_dir::find_official_dir(data_dir)
        .ok_or_else(|| anyhow::anyhow!("未找到官方资源目录 .official"))?;
    apply_selected_files_from(data_dir, &official_dir, is_writable_local, selected_files)
}

fn apply_selected_files_from(
    data_dir: &Path,
    official_dir: &Path,
    is_writable_local: bool,
    selected_files: &[String],
) -> anyhow::Result<ResourceSyncResult> {
    let official_manifest_path = official_dir.join("data_manifest.json");

    let official_manifest = DataManifest::load(&official_manifest_path)
        .map_err(|e| anyhow::anyhow!("读取官方清单失败: {e}"))?;

    let local_manifest_path = data_dir.join("data_manifest.json");
    let mut local_manifest = if local_manifest_path.exists() {
        DataManifest::load(&local_manifest_path).unwrap_or(DataManifest {
            data_version: 0,
            files: HashMap::new(),
        })
    } else {
        DataManifest {
            data_version: 0,
            files: HashMap::new(),
        }
    };

    // 写入前统一校验：只接受官方清单中的普通相对路径，非法选择直接报错并保留 .official/
    if let Some(bad) = selected_files
        .iter()
        .find(|p| !official_manifest.files.contains_key(p.as_str()) || !is_plain_relative(p))
    {
        anyhow::bail!("非法的同步文件路径: {bad}");
    }

    let mut synced = 0usize;

    for path in selected_files {
        // manifest 中的 path 已包含 "game_data/" 前缀，如 "game_data/backgrounds/白天.webp"
        // .official/ 的结构与之对应: .official/game_data/backgrounds/...
        let src = official_dir.join(path);
        let dst = data_dir.join(path);

        if !src.exists() {
            tracing::warn!("官方目录中缺少文件: {path}，跳过");
            continue;
        }

        // 原子写入：先写 .tmp，再 rename
        let tmp = data_dir.join(format!(".{}.tmp", path.replace(['/', '\\'], "_")));
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }

        copy_user_file(&src, &tmp).map_err(|e| anyhow::anyhow!("复制 {path} 失败: {e}"))?;
        std::fs::rename(&tmp, &dst)
            .map_err(|e| anyhow::anyhow!("原子写入 {} 失败: {}", path, e))?;

        // 更新本地 manifest 条目
        if let Some(entry) = official_manifest.files.get(path) {
            local_manifest.files.insert(path.clone(), entry.clone());
        }

        synced += 1;
    }

    // 更新清单版本号
    local_manifest.data_version = official_manifest.data_version;
    local_manifest.save(&local_manifest_path)?;

    // 仅清理位于可写 data_dir 内的 .official/；系统安装只读资源保留并通过 data_version 判重
    if is_writable_local {
        std::fs::remove_dir_all(official_dir)?;
    }
    info!(
        "Synced {} files, official_cleaned={}, data_version={}",
        synced, is_writable_local, local_manifest.data_version
    );

    Ok(ResourceSyncResult {
        success: true,
        files_synced: synced,
        message: format!("成功同步 {} 个文件", synced),
    })
}

fn copy_user_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::copy(src, dst)?;
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(dst)?.permissions();
        let mode = permissions.mode();
        if mode & 0o200 == 0 {
            permissions.set_mode(mode | 0o200);
            std::fs::set_permissions(dst, permissions)?;
        }
    }
    Ok(())
}

// ─── 辅助函数 ────────────────────────────────────────────────

/// 递归复制目录内容。
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<()> {
    if !dst.exists() {
        std::fs::create_dir_all(dst)?;
    }
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            copy_user_file(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

/// 仅由普通路径段组成（无 `..`、根目录、盘符前缀）。
fn is_plain_relative(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::manifest::FileEntry;

    #[test]
    fn readonly_official_sync_preserves_source_and_updates_version() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("xdg_data");
        let official_dir = tmp.path().join("usr_lib_official");
        std::fs::create_dir_all(data_dir.join("game_data")).unwrap();
        std::fs::create_dir_all(official_dir.join("game_data/backgrounds")).unwrap();

        let src_file = official_dir.join("game_data/backgrounds/day.webp");
        std::fs::write(&src_file, b"imgdata").unwrap();

        let mut files = HashMap::new();
        files.insert(
            "game_data/backgrounds/day.webp".to_string(),
            FileEntry {
                sha256: "abc".to_string(),
                size: 7,
                modified_at: 0,
            },
        );
        let official_manifest = DataManifest {
            data_version: 5,
            files,
        };
        official_manifest
            .save(&official_dir.join("data_manifest.json"))
            .unwrap();

        let res = apply_selected_files_from(
            &data_dir,
            &official_dir,
            false,
            &["game_data/backgrounds/day.webp".to_string()],
        )
        .unwrap();
        assert!(res.success);
        assert_eq!(res.files_synced, 1);
        assert!(official_dir.exists());
        assert_eq!(
            std::fs::read(data_dir.join("game_data/backgrounds/day.webp")).unwrap(),
            b"imgdata"
        );
        let local = DataManifest::load(&data_dir.join("data_manifest.json")).unwrap();
        assert_eq!(local.data_version, 5);
    }
}
