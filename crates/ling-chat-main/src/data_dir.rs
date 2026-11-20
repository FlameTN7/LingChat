use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
#[cfg(target_os = "linux")]
static RESOURCE_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// 初始化 data 目录缓存（必须在 App 启动时调用一次）。
pub fn init_data_dir(app: &tauri::AppHandle) {
    #[cfg(target_os = "linux")]
    {
        use tauri::Manager;
        let _ = RESOURCE_DIR.set(app.path().resource_dir().ok());
    }
    let dir = resolve_data_dir(app);
    DATA_DIR.set(dir).expect("data_dir already initialized");
}

/// 获取已缓存的 data 目录（必须先调用 `init_data_dir`）。
pub fn get_data_dir() -> &'static PathBuf {
    DATA_DIR
        .get()
        .expect("data_dir not initialized — call init_data_dir first")
}

#[cfg(target_os = "linux")]
pub fn allow_media_directory(app: &tauri::AppHandle, relative: &Path) -> tauri::Result<()> {
    use tauri::Manager;

    let root = get_data_dir();
    // Tauri 会把已存在的符号链接目标也登记为允许目录。
    if relative
        .ancestors()
        .take_while(|path| !path.as_os_str().is_empty())
        .any(|path| root.join(path).is_symlink())
    {
        return Ok(());
    }
    app.asset_protocol_scope()
        .allow_directory(root.join(relative), true)
}

/// 解析 data 目录路径。
///
/// 优先级：
/// 1. Android：应用专属外部存储 (/storage/emulated/0/Android/data/<package>/files)
/// 2. iOS：沙盒 Documents 目录（配合 Info.ios.plist 的文件共享键，「文件」App 可见）
/// 3. 桌面端开发模式（debug）：项目根目录下的 `data/`
/// 4. 桌面端发布模式（release portable）：exe 所在目录下的 `data/`
fn resolve_data_dir(app: &tauri::AppHandle) -> PathBuf {
    resolve_data_dir_impl(app)
}

#[cfg(target_os = "android")]
fn resolve_data_dir_impl(app: &tauri::AppHandle) -> PathBuf {
    use tauri_plugin_android_fs::{AndroidFsExt, AppDir};
    app.android_fs()
        .app_storage()
        .resolve_path(None, AppDir::Data)
        .expect("failed to resolve Android external files dir")
}

#[cfg(target_os = "ios")]
fn resolve_data_dir_impl(app: &tauri::AppHandle) -> PathBuf {
    use tauri::Manager;
    app.path()
        .document_dir()
        .expect("failed to resolve document_dir on iOS")
}

#[cfg(not(any(target_os = "android", target_os = "ios", target_os = "linux")))]
fn resolve_data_dir_impl(_app: &tauri::AppHandle) -> PathBuf {
    if cfg!(debug_assertions) {
        // 桌面端开发模式：项目根目录的 data/
        // 本 crate 位于 <repo>/crates/ling-chat-main，故需上跳两级回仓库根。
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("data")
    } else {
        // 桌面端便携发布：data 目录放在 exe 旁边
        std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("data")
    }
}

#[cfg(target_os = "linux")]
fn resolve_data_dir_impl(app: &tauri::AppHandle) -> PathBuf {
    use tauri::Manager;
    let env_dir = std::env::var_os("LINGCHAT_DATA_DIR");
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let xdg_app_data = app.path().app_data_dir().ok();
    resolve_linux_data_dir_with(
        env_dir.as_deref(),
        cfg!(debug_assertions),
        exe_dir.as_deref(),
        xdg_app_data.as_deref(),
    )
}

#[cfg(target_os = "linux")]
fn is_dir_writable(dir: &Path) -> bool {
    dir.is_dir() && tempfile::NamedTempFile::new_in(dir).is_ok()
}

/// Linux 双模式 data 目录解析：
/// 1. `LINGCHAT_DATA_DIR` 环境变量（非空时最高优先级）
/// 2. debug 模式：仓库根目录 `data/`
/// 3. 便携模式：`exe_dir/.portable` 存在，或 `exe_dir/data` 存在且可写 -> `exe_dir/data`
/// 4. 系统安装模式（AUR / .deb）：`$XDG_DATA_HOME/com.noiq.ling-chat/data`
#[cfg(target_os = "linux")]
fn resolve_linux_data_dir_with(
    env_dir: Option<&std::ffi::OsStr>,
    is_debug: bool,
    exe_dir: Option<&Path>,
    xdg_app_data_dir: Option<&Path>,
) -> PathBuf {
    if let Some(dir) = env_dir.filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    if is_debug {
        return PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("data");
    }
    if let Some(exe_dir) = exe_dir {
        let portable_marker = exe_dir.join(".portable");
        let portable_data = exe_dir.join("data");
        if portable_marker.exists() || is_dir_writable(&portable_data) {
            return portable_data;
        }
    }
    xdg_app_data_dir
        .expect("failed to resolve app_data_dir on Linux")
        .join("data")
}

#[cfg(target_os = "linux")]
fn linux_resource_roots(data_dir: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(Some(res)) = RESOURCE_DIR.get() {
        roots.push(res.clone());
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            if exe_dir.join("data") != data_dir {
                roots.push(exe_dir.to_path_buf());
            }
        }
    }
    roots.push(PathBuf::from("/usr/lib/LingChat"));
    roots.push(PathBuf::from("/usr/lib/lingchat"));
    roots
}

/// 查找官方资源包（`.official/`）位置，返回 `(路径, 是否位于可写本地数据目录内)`。
///
/// 便携模式下位于 `data_dir/.official`（可写，同步后清理）；
/// Linux 系统安装模式下回退到只读 `resource_dir/data/.official`（不可删除）。
pub fn find_official_dir(data_dir: &Path) -> Option<(PathBuf, bool)> {
    #[cfg(not(target_os = "linux"))]
    {
        let local = data_dir.join(".official");
        local.exists().then_some((local, true))
    }
    #[cfg(target_os = "linux")]
    {
        let roots = linux_resource_roots(data_dir);
        find_official_dir_in(data_dir, &roots)
    }
}

#[cfg(target_os = "linux")]
fn find_official_dir_in(data_dir: &Path, resource_roots: &[PathBuf]) -> Option<(PathBuf, bool)> {
    let local = data_dir.join(".official");
    if local.exists() {
        return Some((local, true));
    }
    for root in resource_roots {
        let candidate = root.join("data").join(".official");
        if candidate.exists() {
            return Some((candidate, false));
        }
    }
    None
}

/// 优先使用用户的完整模型，Linux 系统安装可从只读资源目录加载。
pub fn resolve_model_dir(name: &str, required_files: &[&str]) -> PathBuf {
    let data_dir = get_data_dir();
    #[cfg(target_os = "linux")]
    {
        find_model_dir(
            data_dir,
            &linux_resource_roots(data_dir),
            name,
            required_files,
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = required_files;
        data_dir.join("third_party").join(name)
    }
}

#[cfg(target_os = "linux")]
fn find_model_dir(
    data_dir: &Path,
    resource_roots: &[PathBuf],
    name: &str,
    required_files: &[&str],
) -> PathBuf {
    let primary = data_dir.join("third_party").join(name);
    let complete = |dir: &Path| required_files.iter().all(|file| dir.join(file).is_file());
    if complete(&primary) {
        return primary;
    }
    resource_roots
        .iter()
        .map(|root| root.join("data/third_party").join(name))
        .find(|dir| complete(dir))
        .unwrap_or(primary)
}

/// 首次启动时将内嵌资源播种到 data 目录。
///
/// - 移动端（android/ios）：从 APK 中解压 data.7z
/// - 桌面端：从安装包的 `data/.official/` 复制 default 资源
pub fn seed_data_dir(app: &tauri::AppHandle) -> anyhow::Result<()> {
    let data_dir = get_data_dir().clone();
    let seeded = data_dir.join(".seeded");

    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let manifest = data_dir.join("data_manifest.json");
        if seeded.exists() && manifest.exists() {
            return Ok(());
        }
        let _ = std::fs::remove_file(&seeded);
        seed_via_fs_plugin(app, &data_dir)?;
        std::fs::write(&seeded, b"")
            .map_err(|e| anyhow::anyhow!("failed to write .seeded marker: {}", e))?;
        tracing::info!("Data directory seeding complete (mobile)");
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        seed_desktop(app, &data_dir, &seeded)?;
    }

    Ok(())
}

/// 桌面端播种逻辑。
///
/// - `.official/` 不存在 → 无需操作
/// - `.seeded` 不存在 → 首次启动，全量 seed ← 删除 .official
/// - `.seeded` 存在 + `.official` 存在 → 更新待同步（留给前端 check_resource_sync）
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn seed_desktop(
    _app: &tauri::AppHandle,
    data_dir: &std::path::Path,
    seeded: &std::path::Path,
) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    std::fs::create_dir_all(data_dir)?;

    let Some((official, is_writable_local)) = find_official_dir(data_dir) else {
        return Ok(());
    };

    if !seeded.exists() {
        // 首次启动：全量播种
        tracing::info!("First launch — seeding from {}", official.display());
        crate::resource_sync::sync::seed_full_from_official(data_dir, &official)?;
        std::fs::write(seeded, b"")?;
        if is_writable_local {
            std::fs::remove_dir_all(&official)?;
            tracing::info!("Seed complete, .official removed");
        } else {
            tracing::info!("Seed complete from read-only resource_dir");
        }
    }
    // seeded 存在 + official 存在 → 更新待处理，不自动操作

    Ok(())
}

/// 通过 tauri-plugin-fs 读取打包的 data.7z 并解压到 data_dir。
#[cfg(any(target_os = "android", target_os = "ios"))]
fn seed_via_fs_plugin(app: &tauri::AppHandle, data_dir: &std::path::Path) -> anyhow::Result<()> {
    use anyhow::Context;
    use tauri::Manager;
    use tauri_plugin_fs::FsExt;

    let resource_dir = app
        .path()
        .resource_dir()
        .context("failed to resolve resource_dir on mobile")?;

    let base = resource_dir.to_string_lossy();
    let base = base.trim_end_matches('/');

    let archive_asset = format!("{}/data/data.7z", base);

    let archive_bytes = app
        .fs()
        .read(std::path::Path::new(&archive_asset))
        .with_context(|| format!("failed to read data.7z from {}", archive_asset))?;

    let cursor = std::io::Cursor::new(archive_bytes);
    tracing::info!("Extracting data.7z ({} bytes)", cursor.get_ref().len());

    sevenz_rust2::decompress(cursor, data_dir).context("failed to extract data.7z")?;

    tracing::info!("Data extraction from data.7z complete");
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn linux_data_dir_env_override_wins() {
        let tmp = tempfile::tempdir().unwrap();
        let exe_dir = tmp.path().join("bin");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::write(exe_dir.join(".portable"), b"").unwrap();
        let xdg = tmp.path().join("xdg");

        let resolved = resolve_linux_data_dir_with(
            Some(OsStr::new("/custom/lingchat-data")),
            false,
            Some(&exe_dir),
            Some(&xdg),
        );
        assert_eq!(resolved, PathBuf::from("/custom/lingchat-data"));
    }

    #[test]
    fn linux_data_dir_portable_marker_or_writable_data() {
        let tmp = tempfile::tempdir().unwrap();
        let exe_dir = tmp.path().join("portable");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::write(exe_dir.join(".portable"), b"").unwrap();
        let xdg = tmp.path().join("xdg");

        let by_marker = resolve_linux_data_dir_with(None, false, Some(&exe_dir), Some(&xdg));
        assert_eq!(by_marker, exe_dir.join("data"));

        let exe_dir2 = tmp.path().join("portable2");
        std::fs::create_dir_all(exe_dir2.join("data")).unwrap();
        let by_writable = resolve_linux_data_dir_with(None, false, Some(&exe_dir2), Some(&xdg));
        assert_eq!(by_writable, exe_dir2.join("data"));
    }

    #[test]
    fn linux_data_dir_falls_back_to_xdg_without_portable_data() {
        let tmp = tempfile::tempdir().unwrap();
        let exe_dir = tmp.path().join("usr_bin");
        std::fs::create_dir_all(&exe_dir).unwrap();
        let xdg = tmp.path().join("xdg/com.noiq.ling-chat");

        let resolved = resolve_linux_data_dir_with(None, false, Some(&exe_dir), Some(&xdg));
        assert_eq!(resolved, xdg.join("data"));
    }

    #[test]
    fn linux_official_and_third_party_fallback_to_resource_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("xdg_data");
        let res_root = tmp.path().join("usr_lib_lingchat");
        std::fs::create_dir_all(res_root.join("data/.official")).unwrap();
        std::fs::create_dir_all(res_root.join("data/third_party/asr_vad")).unwrap();
        let model = res_root.join("data/third_party/asr_vad/silero-vad.onnx");
        std::fs::write(&model, b"onnx").unwrap();

        let (official, writable) = find_official_dir_in(&data_dir, &[res_root.clone()]).unwrap();
        assert_eq!(official, res_root.join("data/.official"));
        assert!(!writable);

        let found_dir = find_model_dir(&data_dir, &[res_root], "asr_vad", &["silero-vad.onnx"]);
        assert_eq!(found_dir.join("silero-vad.onnx"), model);
    }
}
