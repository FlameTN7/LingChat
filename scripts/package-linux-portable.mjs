/**
 * 构建 Linux 便携版压缩包：target/<profile>/bundle/portable/LingChat-<version>-linux-<arch>.tar.gz
 *
 * 目录结构：
 *   LingChat-<version>-linux-<arch>/
 *   ├── ling_chat
 *   ├── libwebgpu_dawn.so      (x86_64 存在时附带)
 *   ├── .portable              (触发便携模式：读写同级 data/)
 *   ├── data/
 *   │   ├── .official/
 *   │   │   ├── game_data/
 *   │   │   └── data_manifest.json
 *   │   └── third_party/
 *   ├── icons/
 *   ├── install-desktop.sh     (可选：安装用户级 .desktop 快捷方式)
 *   └── README.txt
 *
 * 用法：
 *   node scripts/package-linux-portable.mjs [--debug]
 */

import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  cpSync,
  existsSync,
  lstatSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { copyRuntimeLibs } from "./prepare-linux-libs.mjs";

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(__dirname, "..");

if (process.platform !== "linux") {
  console.log("[portable] 非 Linux 平台，跳过");
  process.exit(0);
}

const isDebug = process.argv.includes("--debug");
const profile = isDebug ? "debug" : "release";
const archMap = {
  x64: "x86_64",
  arm64: "aarch64",
};
const arch = archMap[process.arch] ?? process.arch;

const tauriConf = JSON.parse(
  readFileSync(join(projectRoot, "src-tauri", "tauri.conf.json"), "utf-8"),
);
const version = tauriConf.version;
if (!version) {
  console.error("❌ src-tauri/tauri.conf.json 缺少 version");
  process.exit(1);
}

const targetProfileDir = join(projectRoot, "target", profile);
const binPath = join(targetProfileDir, "ling_chat");
if (!existsSync(binPath)) {
  console.error(`❌ 未找到可执行文件: ${binPath}，请先运行 cargo build / pnpm tauri build`);
  process.exit(1);
}

const bundledResources = join(projectRoot, "src-tauri", ".bundled_resources");
if (!existsSync(join(bundledResources, "data_manifest.json"))) {
  console.log("[portable] 未检测到 .bundled_resources，正在执行 prepare-desktop-resources.mjs...");
  execFileSync(process.execPath, [join(__dirname, "prepare-desktop-resources.mjs")], {
    cwd: projectRoot,
    stdio: "inherit",
  });
}

const bundleName = `LingChat-${version}-linux-${arch}`;
const outDir = join(targetProfileDir, "bundle", "portable");
const stageRoot = join(outDir, "_staging");
const pkgRoot = join(stageRoot, bundleName);

rmSync(stageRoot, { recursive: true, force: true });
mkdirSync(pkgRoot, { recursive: true });

// 1. 主程序与执行权限
const destBin = join(pkgRoot, "ling_chat");
copyFileSync(binPath, destBin);
chmodSync(destBin, 0o755);

copyRuntimeLibs(targetProfileDir, pkgRoot, "$ORIGIN");

// 3. 便携模式标记文件
writeFileSync(
  join(pkgRoot, ".portable"),
  "# Presence of this file enables portable data directory mode (./data).\n",
  "utf-8",
);

// 4. 官方只读基准资源与第三方模型
const dataDir = join(pkgRoot, "data");
const officialDir = join(dataDir, ".official");
mkdirSync(officialDir, { recursive: true });

const gameDataSrc = join(bundledResources, "game_data");
if (existsSync(gameDataSrc)) {
  cpSync(gameDataSrc, join(officialDir, "game_data"), { recursive: true, dereference: true });
}
const manifestSrc = join(bundledResources, "data_manifest.json");
if (existsSync(manifestSrc)) {
  copyFileSync(manifestSrc, join(officialDir, "data_manifest.json"));
}
const thirdPartySrc = join(bundledResources, "third_party");
if (existsSync(thirdPartySrc)) {
  cpSync(thirdPartySrc, join(dataDir, "third_party"), { recursive: true, dereference: true });
}

// 5. 多尺寸图标（供桌面快捷方式与系统包复用）
const iconsDest = join(pkgRoot, "icons");
mkdirSync(iconsDest, { recursive: true });
for (const iconName of ["32x32.png", "128x128.png", "128x128@2x.png", "icon.png"]) {
  const srcIcon = join(projectRoot, "src-tauri", "icons", iconName);
  if (existsSync(srcIcon)) {
    copyFileSync(srcIcon, join(iconsDest, iconName));
  }
}

// 6. 可选用户级桌面快捷方式安装脚本
const desktopInstaller = `#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
DATA_HOME="\${XDG_DATA_HOME:-$HOME/.local/share}"
APP_DIR="$DATA_HOME/applications"
ICON_DIR="$DATA_HOME/icons/hicolor/256x256/apps"

mkdir -p "$APP_DIR" "$ICON_DIR"

if [ -f "$SCRIPT_DIR/icons/128x128@2x.png" ]; then
  cp "$SCRIPT_DIR/icons/128x128@2x.png" "$ICON_DIR/lingchat.png"
elif [ -f "$SCRIPT_DIR/icons/icon.png" ]; then
  cp "$SCRIPT_DIR/icons/icon.png" "$ICON_DIR/lingchat.png"
fi

cat > "$APP_DIR/lingchat.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=LingChat
Comment=AI Galgame Engine
Exec="$SCRIPT_DIR/ling_chat" %U
Icon=lingchat
Terminal=false
Categories=Game;Chat;Utility;
StartupWMClass=ling_chat
EOF

echo "已安装用户级桌面快捷方式: $APP_DIR/lingchat.desktop"
`;
const installerPath = join(pkgRoot, "install-desktop.sh");
writeFileSync(installerPath, desktopInstaller, "utf-8");
chmodSync(installerPath, 0o755);

// 7. 说明文档
writeFileSync(
  join(pkgRoot, "README.txt"),
  `LingChat ${version} (Linux Portable / ${arch})

运行方式：
  ./ling_chat

数据目录说明：
- 本压缩包附带 .portable 标记，游戏存档与角色数据保存在同级 data/，应用设置仍保存在系统用户目录。
- 同级 data/ 可写时，删除 .portable 后仍使用便携数据目录。
- 如需使用 XDG 目录，每次以 LINGCHAT_DATA_DIR="\${XDG_DATA_HOME:-$HOME/.local/share}/com.noiq.ling-chat/data" ./ling_chat 启动。
- 如需保留已有存档，先退出程序，将原 data/ 的内容复制到选定的新数据目录。
- 运行 ./install-desktop.sh 可在 ~/.local/share/applications 注册桌面启动项（无需 root）。
`,
  "utf-8",
);

// 8. 打包前校验零符号链接
function findSymlinks(dir, results = []) {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    const st = lstatSync(full);
    if (st.isSymbolicLink()) {
      results.push(full);
    } else if (st.isDirectory()) {
      findSymlinks(full, results);
    }
  }
  return results;
}

const symlinks = findSymlinks(pkgRoot);
if (symlinks.length > 0) {
  console.error(`❌ 便携包中不允许存在符号链接: ${symlinks.join(", ")}`);
  process.exit(1);
}

// 9. 打包 .tar.gz 与 .sha256
const archiveName = `${bundleName}.tar.gz`;
const archivePath = join(outDir, archiveName);
rmSync(archivePath, { force: true });
execFileSync("tar", ["-czf", archivePath, "-C", stageRoot, bundleName], { stdio: "inherit" });

const sha256 = createHash("sha256").update(readFileSync(archivePath)).digest("hex");
writeFileSync(`${archivePath}.sha256`, `${sha256}  ${archiveName}\n`, "utf-8");

rmSync(stageRoot, { recursive: true, force: true });

console.log(`✅ 便携包已生成: ${archivePath}`);
console.log(`   SHA-256: ${sha256}`);
