use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
static RESOURCE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 应用 identifier（与 tauri.conf.json 保持一致），用于 XDG 路径。
pub const APP_IDENTIFIER: &str = "com.noiq.ling-chat";

/// 初始化 data / resource 目录缓存（必须在 App 启动时调用一次）。
pub fn init_data_dir(app: &tauri::AppHandle) {
    let dir = resolve_data_dir(app);
    // 系统安装态 XDG 路径可能尚不存在，提前建好供日志/数据库使用
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!("创建 data 目录失败 ({}): {e}", dir.display());
    }
    DATA_DIR.set(dir).expect("data_dir already initialized");
    let res = resolve_resource_dir(app);
    RESOURCE_DIR
        .set(res)
        .expect("resource_dir already initialized");
}

/// 获取已缓存的 data 目录（必须先调用 `init_data_dir`）。
pub fn get_data_dir() -> &'static PathBuf {
    DATA_DIR
        .get()
        .expect("data_dir not initialized — call init_data_dir first")
}

/// 获取已缓存的打包资源根目录（必须先调用 `init_data_dir`）。
///
/// 对应 Tauri bundle 的 resource 根：其下有 `data/.official/`、`data/third_party/`。
/// 便携包与系统安装均可用；系统安装态下该目录通常只读。
pub fn get_resource_dir() -> &'static PathBuf {
    RESOURCE_DIR
        .get()
        .expect("resource_dir not initialized — call init_data_dir first")
}

/// 解析 data 目录路径。
///
/// 优先级：
/// 1. Android：应用专属外部存储
/// 2. iOS：沙盒 Documents 目录
/// 3. 桌面端开发模式（debug）：项目根目录下的 `data/`
/// 4. 桌面端发布模式：见各平台实现（Linux 支持 XDG / 便携双模式）
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

#[cfg(not(any(target_os = "android", target_os = "ios")))]
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
        resolve_desktop_release_data_dir()
    }
}

/// 桌面 release 下的 data 目录（Windows / macOS：exe 旁；Linux：双模式）。
#[cfg(all(
    not(any(target_os = "android", target_os = "ios")),
    not(target_os = "linux")
))]
fn resolve_desktop_release_data_dir() -> PathBuf {
    std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("data")
}

/// Linux release：LINGCHAT_DATA_DIR → 便携 → XDG 系统安装。
#[cfg(all(not(any(target_os = "android", target_os = "ios")), target_os = "linux"))]
fn resolve_desktop_release_data_dir() -> PathBuf {
    let exe_dir = current_exe_dir();
    let env_data = std::env::var_os("LINGCHAT_DATA_DIR");
    let xdg = std::env::var_os("XDG_DATA_HOME");
    let home = std::env::var_os("HOME");
    let portable_marker = exe_dir.join(".portable").exists();
    let data_beside = exe_dir.join("data");
    let data_writable = path_is_writable_dir(&data_beside);

    resolve_linux_release_data_dir(
        &exe_dir,
        env_data.as_deref(),
        xdg.as_deref(),
        home.as_deref(),
        portable_marker,
        data_writable,
    )
}

/// Linux release data 目录判定（纯函数，便于单测）。
///
/// 优先级：
/// 1. `LINGCHAT_DATA_DIR` 环境变量（非空）
/// 2. 便携：`exe_dir/.portable` 存在，或 `exe_dir/data` 存在且可写 → `exe_dir/data`
/// 3. 系统安装：`$XDG_DATA_HOME/<id>/data`，缺省 `~/.local/share/<id>/data`
pub fn resolve_linux_release_data_dir(
    exe_dir: &Path,
    env_data_dir: Option<&std::ffi::OsStr>,
    xdg_data_home: Option<&std::ffi::OsStr>,
    home: Option<&std::ffi::OsStr>,
    portable_marker_exists: bool,
    exe_data_writable: bool,
) -> PathBuf {
    if let Some(raw) = env_data_dir {
        if !raw.is_empty() {
            return PathBuf::from(raw);
        }
    }

    if is_linux_portable_layout(portable_marker_exists, exe_data_writable) {
        return exe_dir.join("data");
    }

    linux_xdg_data_dir(xdg_data_home, home)
}

/// 便携布局判定：`.portable` 标记，或旁路 `data/` 已存在且可写。
pub fn is_linux_portable_layout(portable_marker_exists: bool, exe_data_writable: bool) -> bool {
    portable_marker_exists || exe_data_writable
}

/// 与 Tauri `app_data_dir` 一致的 XDG 用户数据根下的 `data/` 子目录。
pub fn linux_xdg_data_dir(
    xdg_data_home: Option<&std::ffi::OsStr>,
    home: Option<&std::ffi::OsStr>,
) -> PathBuf {
    let base = if let Some(xdg) = xdg_data_home.filter(|v| !v.is_empty()) {
        PathBuf::from(xdg)
    } else if let Some(h) = home.filter(|v| !v.is_empty()) {
        PathBuf::from(h).join(".local").join("share")
    } else {
        PathBuf::from("/tmp").join(APP_IDENTIFIER)
    };
    base.join(APP_IDENTIFIER).join("data")
}

/// 解析打包资源根目录。
///
/// 优先级：`LINGCHAT_RESOURCE_DIR` → exe 真实路径父目录（含 data/.official 或 data/third_party）
/// → Tauri `resource_dir()` → exe 父目录兜底。
/// 兼容 deb（`/usr/lib/LingChat`）与 AUR（`/usr/lib/lingchat`）。
fn resolve_resource_dir(app: &tauri::AppHandle) -> PathBuf {
    resolve_resource_dir_impl(app)
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn resolve_resource_dir_impl(app: &tauri::AppHandle) -> PathBuf {
    use tauri::Manager;
    app.path()
        .resource_dir()
        .unwrap_or_else(|_| get_data_dir().clone())
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn resolve_resource_dir_impl(app: &tauri::AppHandle) -> PathBuf {
    if let Some(raw) = std::env::var_os("LINGCHAT_RESOURCE_DIR") {
        if !raw.is_empty() {
            return PathBuf::from(raw);
        }
    }

    let exe_parent = current_exe_dir();
    if looks_like_resource_root(&exe_parent) {
        return exe_parent;
    }

    use tauri::Manager;
    if let Ok(res) = app.path().resource_dir() {
        if looks_like_resource_root(&res) || res.exists() {
            return res;
        }
    }

    exe_parent
}

/// 判断目录是否像打包资源根（含 `data/.official` 或 `data/third_party`）。
pub fn looks_like_resource_root(dir: &Path) -> bool {
    let data = dir.join("data");
    data.join(".official").exists() || data.join("third_party").exists()
}

fn current_exe_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    let resolved = exe.canonicalize().unwrap_or(exe);
    resolved
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(target_os = "linux")]
fn path_is_writable_dir(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    // 尝试在目录内创建临时文件探测可写（比 metadata 权限位更贴近真实挂载）
    let probe = path.join(".lingchat-write-probe");
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        },
        Err(_) => false,
    }
}

/// `.official` 资源来源：便携可写 vs 系统安装只读。
#[derive(Debug, Clone)]
pub enum OfficialSource {
    /// `data_dir/.official`，同步后可删除
    Writable(PathBuf),
    /// `resource_dir/data/.official`，禁止删除
    Readonly(PathBuf),
}

impl OfficialSource {
    pub fn path(&self) -> &Path {
        match self {
            Self::Writable(p) | Self::Readonly(p) => p,
        }
    }

    pub fn is_readonly(&self) -> bool {
        matches!(self, Self::Readonly(_))
    }
}

/// 定位 `.official` 目录。
///
/// 便携 / 传统布局优先 `data_dir/.official`；
/// Linux 系统安装再回退 `resource_dir/data/.official`（只读）。
pub fn resolve_official_source() -> Option<OfficialSource> {
    let data_dir = get_data_dir();
    let beside = data_dir.join(".official");
    if beside.exists() {
        return Some(OfficialSource::Writable(beside));
    }

    #[cfg(target_os = "linux")]
    {
        // 仅 Linux 系统安装需要从只读 resource 读 .official；
        // 其他桌面平台保持「无 .official 即无更新」原行为。
        if !cfg!(debug_assertions) {
            let from_res = get_resource_dir().join("data").join(".official");
            if from_res.exists() {
                return Some(OfficialSource::Readonly(from_res));
            }
        }
    }

    let _ = data_dir;
    None
}

/// 解析 third_party 下相对路径：优先用户 data_dir，Linux 再回退只读 resource。
///
/// `relative` 形如 `emotion_model_19emo` 或 `asr_vad/silero-vad.onnx`。
/// 均不存在时返回 `None`。
pub fn resolve_third_party(relative: impl AsRef<Path>) -> Option<PathBuf> {
    let relative = relative.as_ref();
    let user = get_data_dir().join("third_party").join(relative);
    if user.exists() {
        return Some(user);
    }

    #[cfg(target_os = "linux")]
    {
        let bundled = get_resource_dir()
            .join("data")
            .join("third_party")
            .join(relative);
        if bundled.exists() {
            return Some(bundled);
        }
    }

    None
}

/// 首次启动时将内嵌资源播种到 data 目录。
///
/// - 移动端（android/ios）：从 APK 中解压 data.7z
/// - 桌面端：从安装包的 `.official/` 复制 default 资源
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
/// - `.seeded` 不存在 → 首次启动，全量 seed；可写源删除 .official，只读源保留
/// - `.seeded` 存在 + `.official` 存在 → 更新待同步（留给前端 check_resource_sync）
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn seed_desktop(
    _app: &tauri::AppHandle,
    data_dir: &std::path::Path,
    seeded: &std::path::Path,
) -> anyhow::Result<()> {
    let Some(official) = resolve_official_source() else {
        return Ok(());
    };

    if !seeded.exists() {
        tracing::info!(
            "First launch — seeding from {} (readonly={})",
            official.path().display(),
            official.is_readonly()
        );
        crate::resource_sync::sync::seed_full_from_official(data_dir, official.path())?;
        std::fs::write(seeded, b"")?;
        if let OfficialSource::Writable(path) = &official {
            std::fs::remove_dir_all(path)?;
            tracing::info!("Seed complete, .official removed");
        } else {
            tracing::info!("Seed complete, readonly .official kept");
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn linux_portable_by_marker() {
        assert!(is_linux_portable_layout(true, false));
        assert!(is_linux_portable_layout(false, true));
        assert!(!is_linux_portable_layout(false, false));
    }

    #[test]
    fn linux_env_overrides_all() {
        let exe = Path::new("/usr/lib/lingchat");
        let got = resolve_linux_release_data_dir(
            exe,
            Some(OsStr::new("/custom/data")),
            Some(OsStr::new("/xdg")),
            Some(OsStr::new("/home/u")),
            true,
            true,
        );
        assert_eq!(got, PathBuf::from("/custom/data"));
    }

    #[test]
    fn linux_portable_uses_exe_data() {
        let exe = Path::new("/opt/LingChat");
        let got = resolve_linux_release_data_dir(
            exe,
            None,
            Some(OsStr::new("/xdg")),
            Some(OsStr::new("/home/u")),
            true,
            false,
        );
        assert_eq!(got, PathBuf::from("/opt/LingChat/data"));
    }

    #[test]
    fn linux_system_uses_xdg_data_home() {
        let exe = Path::new("/usr/lib/lingchat");
        let got = resolve_linux_release_data_dir(
            exe,
            None,
            Some(OsStr::new("/xdg")),
            Some(OsStr::new("/home/u")),
            false,
            false,
        );
        assert_eq!(
            got,
            PathBuf::from("/xdg")
                .join(APP_IDENTIFIER)
                .join("data")
        );
    }

    #[test]
    fn linux_system_falls_back_to_home_local_share() {
        let exe = Path::new("/usr/lib/LingChat");
        let got = resolve_linux_release_data_dir(
            exe,
            None,
            None,
            Some(OsStr::new("/home/alice")),
            false,
            false,
        );
        assert_eq!(
            got,
            PathBuf::from("/home/alice/.local/share")
                .join(APP_IDENTIFIER)
                .join("data")
        );
    }

    #[test]
    fn looks_like_resource_root_checks_children() {
        // 无真实目录时为 false
        assert!(!looks_like_resource_root(Path::new("/no/such/lingchat-root")));
    }
}
