import { ref } from "vue";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { i18n } from "@/locales";
import { isLinux } from "@/utils/platform";

/** GitHub Release 最新页；Linux 无应用内二进制更新，引导用户来此下载或看包管理器说明 */
export const GITHUB_RELEASES_LATEST_URL = "https://github.com/SlimeBoyOwO/LingChat/releases/latest";

const GITHUB_LATEST_API = "https://api.github.com/repos/SlimeBoyOwO/LingChat/releases/latest";

// ─── 类型定义 ────────────────────────────────────────────────

export interface ResourceSyncInfo {
  available: boolean;
  newVersion: number;
  currentVersion: number;
  filesToAdd: SyncFileEntry[];
  filesToModify: SyncFileEntry[];
  totalSize: number;
}

export interface SyncFileEntry {
  path: string;
  sha256: string;
  size: number;
  changeType: "add" | "modify";
}

export interface ResourceSyncResult {
  success: boolean;
  filesSynced: number;
  message: string;
}

export type UpdatePhase =
  "idle" | "checking" | "app-update-available" | "downloading" | "complete" | "error";

// ─── 共享状态 ────────────────────────────────────────────────

const phase = ref<UpdatePhase>("idle");
const appVersion = ref("");
const appReleaseNotes = ref("");
const errorMessage = ref("");
const downloadProgress = ref(0); // 0-100

// 资源同步独立状态
const resourceSyncInfo = ref<ResourceSyncInfo | null>(null);
const resourceSyncPhase = ref<"idle" | "review" | "syncing" | "complete" | "error">("idle");
const resourceSyncError = ref("");

/** 简易 semver 比较：a>b 返回正数。忽略前缀 v，非数字段按 0。 */
function compareSemver(a: string, b: string): number {
  const parse = (v: string) =>
    v
      .replace(/^v/i, "")
      .split(/[.+-]/)
      .map((p) => {
        const n = parseInt(p, 10);
        return Number.isFinite(n) ? n : 0;
      });
  const pa = parse(a);
  const pb = parse(b);
  const len = Math.max(pa.length, pb.length);
  for (let i = 0; i < len; i++) {
    const da = pa[i] ?? 0;
    const db = pb[i] ?? 0;
    if (da !== db) return da - db;
  }
  return 0;
}

/** Linux：用 GitHub Releases API 比版本，不走 updater（无 Linux 平台条目，会报错刷日志） */
async function checkLinuxUpdateViaGithub(): Promise<boolean> {
  const current = await getVersion();
  const res = await fetch(GITHUB_LATEST_API, {
    headers: { Accept: "application/vnd.github+json" },
  });
  if (!res.ok) {
    throw new Error(`GitHub API ${res.status}`);
  }
  const data = (await res.json()) as { tag_name?: string; body?: string };
  const latest = String(data.tag_name ?? "").replace(/^v/i, "");
  if (!latest) {
    phase.value = "idle";
    return false;
  }
  if (compareSemver(latest, current) > 0) {
    appVersion.value = latest;
    appReleaseNotes.value = String(data.body ?? "");
    phase.value = "app-update-available";
    return true;
  }
  phase.value = "idle";
  return false;
}

// ─── 导出 composable ─────────────────────────────────────────

export function useUpdater() {
  /** 检查 app 更新，返回是否有可用更新 */
  async function checkForUpdates(): Promise<boolean> {
    phase.value = "checking";
    errorMessage.value = "";

    try {
      // Linux 关闭应用内二进制更新：不调用 updater.check，避免无平台条目时的 ERROR 噪声
      if (isLinux()) {
        return await checkLinuxUpdateViaGithub();
      }

      const update = await check();
      if (update?.available) {
        appVersion.value = update.version ?? "";
        appReleaseNotes.value = update.body ?? "";
        phase.value = "app-update-available";
        return true;
      }
      // 无更新
      phase.value = "idle";
      return false;
    } catch (e) {
      const msg = String(e);
      console.error("[Updater] App update check failed:", msg);
      phase.value = "error";
      // 提取有意义的部分（去掉冗长的 URL 和底层错误堆栈）
      if (msg.includes("error sending request") || msg.includes("Failed to fetch")) {
        errorMessage.value = i18n.global.t("stores.updater.networkError");
      } else {
        errorMessage.value = msg.length > 200 ? msg.slice(0, 200) : msg;
      }
      return false;
    }
  }

  /** 安装 app 更新并重启（带进度追踪）。Linux 不支持，请走 openReleasePage。 */
  async function installAppUpdate(): Promise<void> {
    if (isLinux()) {
      errorMessage.value = i18n.global.t("stores.updater.linuxNoInAppUpdate");
      phase.value = "error";
      return;
    }

    try {
      const update = await check();
      if (!update?.available) {
        errorMessage.value = i18n.global.t("stores.updater.noUpdateAvailable");
        phase.value = "error";
        return;
      }

      phase.value = "downloading";
      downloadProgress.value = 0;

      let contentLength: number | undefined;
      let downloaded = 0;

      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          contentLength = event.data.contentLength;
        } else if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
          if (contentLength) {
            downloadProgress.value = Math.round((downloaded / contentLength) * 100);
          }
        }
      });

      phase.value = "complete";
      await relaunch();
    } catch (e) {
      console.error("[Updater] App update install failed:", e);
      phase.value = "error";
      errorMessage.value = String(e);
    }
  }

  /** 打开 GitHub Release 最新页（Linux 手动更新入口） */
  async function openReleasePage(): Promise<void> {
    await openUrl(GITHUB_RELEASES_LATEST_URL);
  }

  /** 重置状态 */
  function reset() {
    phase.value = "idle";
    errorMessage.value = "";
  }

  // ─── 资源同步 ──────────────────────────────────────────────

  /** 检查数据资源更新（对比 .official manifest） */
  async function checkResourceSync(): Promise<boolean> {
    try {
      const info = await invoke<ResourceSyncInfo>("check_resource_sync");
      resourceSyncInfo.value = info;
      if (info.available) {
        resourceSyncPhase.value = "review";
        return true;
      }
      return false;
    } catch (e) {
      resourceSyncPhase.value = "error";
      resourceSyncError.value = String(e);
      return false;
    }
  }

  /** 应用选中的资源文件同步 */
  async function applyResourceSync(selectedFiles: string[]): Promise<void> {
    resourceSyncPhase.value = "syncing";
    try {
      const result = await invoke<ResourceSyncResult>("apply_resource_sync", {
        selectedFiles,
      });
      if (result.success) {
        resourceSyncPhase.value = "complete";
      } else {
        resourceSyncPhase.value = "error";
        resourceSyncError.value = result.message;
      }
    } catch (e) {
      resourceSyncPhase.value = "error";
      resourceSyncError.value = String(e);
    }
  }

  /** 获取本地数据版本号 */
  async function getDataVersion(): Promise<number> {
    try {
      return await invoke<number>("get_data_version");
    } catch {
      return 0;
    }
  }

  /** 重置资源同步状态 */
  function resetResourceSync() {
    resourceSyncInfo.value = null;
    resourceSyncPhase.value = "idle";
    resourceSyncError.value = "";
  }

  return {
    // App 更新
    phase,
    appVersion,
    appReleaseNotes,
    errorMessage,
    downloadProgress,
    checkForUpdates,
    installAppUpdate,
    openReleasePage,
    reset,
    // 资源同步
    resourceSyncInfo,
    resourceSyncPhase,
    resourceSyncError,
    checkResourceSync,
    applyResourceSync,
    getDataVersion,
    resetResourceSync,
  };
}
