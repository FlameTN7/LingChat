use std::borrow::Cow;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// 在 Linux 上将配置或归档内可能携带的 Windows `\` 分隔符规范化为 `/`。
pub fn normalize_separators(raw: &str) -> Cow<'_, str> {
    #[cfg(target_os = "linux")]
    if raw.contains('\\') {
        return Cow::Owned(raw.replace('\\', "/"));
    }
    Cow::Borrowed(raw)
}

/// Linux 优先精确匹配，并兼容 Windows 模型文件中的分隔符与大小写。
pub(crate) fn resolve_relative_path(base: &Path, relative: &str) -> PathBuf {
    let normalized = normalize_separators(relative);
    let direct = base.join(normalized.as_ref());
    #[cfg(target_os = "linux")]
    if !direct.exists() {
        if let Some(matched) = resolve_case_insensitive_in(base, normalized.as_ref()) {
            return matched;
        }
    }
    direct
}

#[cfg(target_os = "linux")]
fn resolve_case_insensitive_in(base: &Path, normalized_rel: &str) -> Option<PathBuf> {
    use std::path::Component;
    let mut cur = base.to_path_buf();
    for comp in Path::new(normalized_rel).components() {
        match comp {
            Component::CurDir => {},
            Component::Normal(seg) => {
                let exact = cur.join(seg);
                if exact.exists() {
                    cur = exact;
                    continue;
                }
                let seg_str = seg.to_str()?;
                let mut found = None;
                for entry in fs::read_dir(&cur).ok()?.flatten() {
                    if entry
                        .file_name()
                        .to_string_lossy()
                        .eq_ignore_ascii_case(seg_str)
                    {
                        found = Some(entry.path());
                        break;
                    }
                }
                cur = found?;
            },
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(cur)
}

/// 校验并规范化用户提供的单层目录名。
///
/// 分类名最终会被拼接到资源根目录下，因此这里只允许一个普通路径组件，
/// 同时拒绝 Windows 文件系统中的保留字符、尾随点/空格和设备名。
pub fn validate_directory_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("分类名不能为空".to_string());
    }

    let mut components = Path::new(name).components();
    let is_single_normal_component = matches!(
        components.next(),
        Some(std::path::Component::Normal(component)) if component == name
    ) && components.next().is_none();
    if !is_single_normal_component
        || name.chars().any(|c| {
            c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
        })
        || name.ends_with('.')
        || name.ends_with(' ')
    {
        return Err("分类名包含非法字符".to_string());
    }

    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    let is_reserved_device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            });
    if is_reserved_device {
        return Err("分类名不能使用系统保留名称".to_string());
    }

    Ok(name.to_string())
}
fn collect_regular_files(base: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(base)
        .map_err(|e| format!("读取分类目录失败: {} - 路径: {}", e, base.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("读取分类目录项失败: {e}"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|e| format!("读取文件类型失败: {} - 路径: {}", e, path.display()))?;
        if file_type.is_symlink() {
            return Err(format!("分类目录包含不支持的符号链接: {}", path.display()));
        }
        if file_type.is_dir() {
            collect_regular_files(&path, out)?;
        } else if file_type.is_file() {
            out.push(path);
        } else {
            return Err(format!("分类目录包含不支持的文件类型: {}", path.display()));
        }
    }
    Ok(())
}

fn remove_empty_directory_tree(dir: &Path) -> Result<(), String> {
    let entries = fs::read_dir(dir)
        .map_err(|e| format!("检查分类目录失败: {} - 路径: {}", e, dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("读取分类目录项失败: {e}"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|e| format!("读取文件类型失败: {} - 路径: {}", e, path.display()))?;
        if !file_type.is_dir() || file_type.is_symlink() {
            return Err(format!("分类目录仍包含未转移的文件: {}", path.display()));
        }
        remove_empty_directory_tree(&path)?;
    }
    fs::remove_dir(dir).map_err(|e| format!("删除空分类目录失败: {} - 路径: {}", e, dir.display()))
}

/// 将目录树中的所有普通文件扁平移动到目标目录。
pub fn move_directory_files_to(source_dir: &Path, target_dir: &Path) -> Result<usize, String> {
    let mut files = Vec::new();
    collect_regular_files(source_dir, &mut files)?;

    let mut destination_names = HashSet::new();
    for source in &files {
        let file_name = source
            .file_name()
            .ok_or_else(|| format!("无效的文件路径: {}", source.display()))?;
        let collision_key = file_name.to_string_lossy().to_lowercase();
        if !destination_names.insert(collision_key) {
            return Err(format!(
                "分类中存在重名文件，无法安全移动: {}",
                file_name.to_string_lossy()
            ));
        }
        let destination = target_dir.join(file_name);
        if destination.exists() {
            return Err(format!("目标目录已存在同名文件: {}", destination.display()));
        }
    }

    for source in &files {
        let destination = target_dir.join(source.file_name().expect("file name checked above"));
        move_file_with_exdev_fallback(source, &destination).map_err(|e| {
            format!(
                "移动文件失败，源分类已保留且不会删除: {} -> {}: {}",
                source.display(),
                destination.display(),
                e
            )
        })?;
    }

    remove_empty_directory_tree(source_dir)?;
    Ok(files.len())
}

/// 移动单个文件；在 Linux 跨挂载点（`EXDEV`）时回退到同分区临时复制再原子替换并删源。
pub fn move_file_with_exdev_fallback(src: &Path, dst: &Path) -> std::io::Result<()> {
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        #[cfg(target_os = "linux")]
        Err(e) if e.raw_os_error() == Some(18) => {
            let parent = dst.parent().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing dst parent")
            })?;
            let tmp_name = format!(".lingchat_move_{}.tmp", uuid::Uuid::new_v4().simple());
            let tmp = parent.join(tmp_name);
            if let Err(err) = fs::copy(src, &tmp).and_then(|_| fs::rename(&tmp, dst)) {
                let _ = fs::remove_file(&tmp);
                return Err(err);
            }
            fs::remove_file(src)
        },
        Err(e) => Err(e),
    }
}

/// 异步移动单个文件；在 Linux 跨挂载点（`EXDEV`）时回退到目标目录同分区复制替换再删源。
pub async fn move_file_with_exdev_fallback_async(src: &Path, dst: &Path) -> std::io::Result<()> {
    match tokio::fs::rename(src, dst).await {
        Ok(()) => Ok(()),
        #[cfg(target_os = "linux")]
        Err(e) if e.raw_os_error() == Some(18) => {
            let parent = dst.parent().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing dst parent")
            })?;
            let tmp = parent.join(format!(
                ".lingchat_move_{}.tmp",
                uuid::Uuid::new_v4().simple()
            ));
            if let Err(err) = async {
                tokio::fs::copy(src, &tmp).await?;
                tokio::fs::rename(&tmp, dst).await
            }
            .await
            {
                let _ = tokio::fs::remove_file(&tmp).await;
                return Err(err);
            }
            tokio::fs::remove_file(src).await
        },
        Err(e) => Err(e),
    }
}

/// 将角色资源路径解析为绝对路径。
pub fn resolve_character_path(data_dir: &Path, resource_path: &str) -> PathBuf {
    #[cfg(target_os = "linux")]
    let normalized = normalize_separators(resource_path);
    #[cfg(not(target_os = "linux"))]
    let normalized = resource_path;
    if let Some(rest) = normalized.strip_prefix("plugin:")
        && let Some((plugin_id, folder)) = rest.split_once('/')
        && !plugin_id.is_empty()
        && !folder.is_empty()
    {
        return data_dir
            .join("plugins")
            .join(plugin_id)
            .join("characters")
            .join(folder);
    }
    let path = PathBuf::from(&*normalized);
    if path.is_absolute() {
        path
    } else {
        data_dir.join("game_data").join("characters").join(path)
    }
}

/// 批量创建目录（幂等）。任一失败立即返回错误。
pub fn ensure_dirs(dirs: &[&Path]) -> Result<(), String> {
    for d in dirs {
        std::fs::create_dir_all(d).map_err(|e| format!("create_dir_all {}: {e}", d.display()))?;
    }
    Ok(())
}

/// 路径穿越防护：验证 canonical 路径是否以预期的基础目录开头。
pub fn validate_path_in_base(resolved: &Path, base: &Path) -> Result<(), String> {
    let canon_resolved = resolved
        .canonicalize()
        .map_err(|e| format!("路径解析失败: {} - 路径: {:?}", e, resolved))?;

    let canon_base = base
        .canonicalize()
        .map_err(|e| format!("基础目录解析失败: {} - 路径: {:?}", e, base))?;

    if !canon_resolved.starts_with(&canon_base) {
        return Err(format!(
            "非法路径：试图访问基础目录之外的文件\n\
             请求路径: {:?}\n\
             规范路径: {:?}\n\
             基础目录: {:?}\n\
             规范基础目录: {:?}",
            resolved, canon_resolved, base, canon_base
        ));
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn linux_normalize_separators_and_character_path() {
        let base = Path::new("/tmp/lingchat-data");
        assert_eq!(normalize_separators("a\\b\\c.png").as_ref(), "a/b/c.png");
        assert_eq!(
            resolve_character_path(base, "alice\\portrait.png"),
            base.join("game_data/characters/alice/portrait.png")
        );
    }

    #[test]
    fn linux_move_file_with_exdev_fallback_moves_within_fs() {
        let root = std::env::temp_dir().join(format!("lc-exdev-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let src = root.join("src.txt");
        let dst = root.join("nested").join("dst.txt");
        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
        std::fs::write(&src, b"hello").unwrap();

        move_file_with_exdev_fallback(&src, &dst).unwrap();
        assert!(!src.exists());
        assert_eq!(std::fs::read(&dst).unwrap(), b"hello");
        let _ = std::fs::remove_dir_all(&root);
    }
}
