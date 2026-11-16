#!/usr/bin/env bash
#
# 打 Linux 便携包（tar.gz + .sha256）。
#
# 布局依据 crates/ling-chat-main/src/data_dir.rs：
# - 旁路 .portable（或可写 data/）→ 用户数据写在 exe 旁 data/
# - 只读资源根为 exe 真实路径父目录（其下需有 data/.official 或 data/third_party）
#
# tauri-build 在编译阶段已把 bundle.resources 复制到 target/<profile>/data/
# （.official 与 third_party），此处收拢为「exe 与 data 同级」再压缩。
#
# 用法:
#   bash scripts/package-linux-portable.sh [target_dir] [out_dir]
# 默认: target/release 与 dist-portable（相对当前工作目录）

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

TARGET_DIR="${1:-target/release}"
OUT_DIR="${2:-dist-portable}"

# 相对路径按调用时的 cwd 解析；绝对路径原样使用
if [[ "${TARGET_DIR}" != /* ]]; then
  TARGET_DIR="$(pwd)/${TARGET_DIR}"
fi
if [[ "${OUT_DIR}" != /* ]]; then
  OUT_DIR="$(pwd)/${OUT_DIR}"
fi

VERSION="$(node -p 'require(process.argv[1]).version' "${REPO_ROOT}/src-tauri/tauri.conf.json")"
PKG_ARCH="$(uname -m)"
PKG_NAME="LingChat-${VERSION}-linux-${PKG_ARCH}"
STAGE="${OUT_DIR}/${PKG_NAME}"

if [[ ! -x "${TARGET_DIR}/ling_chat" ]]; then
  echo "找不到可执行文件 ${TARGET_DIR}/ling_chat（请先执行 tauri build）" >&2
  exit 1
fi
if [[ ! -d "${TARGET_DIR}/data" ]]; then
  echo "找不到 ${TARGET_DIR}/data（bundle.resources 未被复制，便携包会缺少内置资源）" >&2
  exit 1
fi

rm -rf "${STAGE}"
mkdir -p "${STAGE}"

cp "${TARGET_DIR}/ling_chat" "${STAGE}/ling_chat"
chmod +x "${STAGE}/ling_chat"

# 用 -L 解引用：构建产物里可能混入指向构建机缓存的符号链接
cp -aL "${TARGET_DIR}/data" "${STAGE}/data"

# ort webgpu 的 Dawn 动态库（目前主要 linux-amd64 生成）
# ort copy-dylibs 在较新 Cargo 下可能落到 target/<profile>/build/（OUT_DIR 多了一层 crate 名），
# 旧布局则在 target/<profile>/；均为指向 ~/.cache/ort.pyke.io/... 的软链，必须解引用
DAWN_SRC=""
for candidate in \
  "${TARGET_DIR}/libwebgpu_dawn.so" \
  "${TARGET_DIR}/build/libwebgpu_dawn.so"; do
  if [[ -e "${candidate}" ]]; then
    DAWN_SRC="${candidate}"
    break
  fi
done
if [[ -n "${DAWN_SRC}" ]]; then
  cp -Lp "${DAWN_SRC}" "${STAGE}/libwebgpu_dawn.so"
fi

# 便携标记：运行期优先走 exe 旁 data/，不落到 XDG
: > "${STAGE}/.portable"

# 图标（供本包 install-desktop-entry.sh 与 AUR 多尺寸安装）
ICONS_SRC="${REPO_ROOT}/src-tauri/icons"
mkdir -p "${STAGE}/icons"
for size in 32 64 128; do
  src="${ICONS_SRC}/${size}x${size}.png"
  if [[ -f "${src}" ]]; then
    cp "${src}" "${STAGE}/icons/${size}x${size}.png"
  fi
done
# 128@2x 实为 256x256，供 hicolor 256 槽位
if [[ -f "${ICONS_SRC}/128x128@2x.png" ]]; then
  cp "${ICONS_SRC}/128x128@2x.png" "${STAGE}/icons/256x256.png"
elif [[ -f "${ICONS_SRC}/128x128.png" ]]; then
  cp "${ICONS_SRC}/128x128.png" "${STAGE}/icons/128x128.png"
fi

# LICENSE（AUR lingchat-bin 从本包安装到 /usr/share/licenses）
if [[ -f "${REPO_ROOT}/LICENSE" ]]; then
  cp "${REPO_ROOT}/LICENSE" "${STAGE}/LICENSE"
fi

# desktop 模板：Exec/Icon 由 install-desktop-entry.sh 改写为解压目录绝对路径
cat > "${STAGE}/lingchat.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=LingChat
Comment=AI Galgame Engine
Exec=ling_chat
Icon=lingchat
Terminal=false
Categories=Game;
StartupWMClass=LingChat
EOF
cat > "${STAGE}/install-desktop-entry.sh" <<'EOF'
#!/usr/bin/env bash
# 将本便携目录的桌面入口安装到 ~/.local/share/applications
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor"
mkdir -p "${APP_DIR}"

ICON_NAME="lingchat"
BEST_ICON=""
for size in 256 128 64 32; do
  candidate="${ROOT}/icons/${size}x${size}.png"
  if [[ -f "${candidate}" ]]; then
    mkdir -p "${ICON_DIR}/${size}x${size}/apps"
    cp "${candidate}" "${ICON_DIR}/${size}x${size}/apps/${ICON_NAME}.png"
    BEST_ICON="${ICON_DIR}/${size}x${size}/apps/${ICON_NAME}.png"
  fi
done

EXEC_PATH="${ROOT}/ling_chat"
# 优先用已安装的 hicolor 图标名；若未装上则回退到包内绝对路径
if [[ -n "${BEST_ICON}" ]]; then
  ICON_VALUE="${ICON_NAME}"
else
  ICON_VALUE="${ROOT}/icons/128x128.png"
fi

sed \
  -e "s|^Exec=.*|Exec=${EXEC_PATH}|" \
  -e "s|^Icon=.*|Icon=${ICON_VALUE}|" \
  "${ROOT}/lingchat.desktop" > "${APP_DIR}/lingchat.desktop"
chmod 644 "${APP_DIR}/lingchat.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "${APP_DIR}" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1 && [[ -d "${ICON_DIR}" ]]; then
  gtk-update-icon-cache -f -t "${ICON_DIR}" >/dev/null 2>&1 || true
fi

echo "已安装桌面入口: ${APP_DIR}/lingchat.desktop"
echo "启动命令: ${EXEC_PATH}"
EOF
chmod +x "${STAGE}/install-desktop-entry.sh"

cat > "${STAGE}/README.txt" <<'EOF'
LingChat Linux 便携版
=====================

运行：
    ./ling_chat

本目录是便携布局：旁路的 .portable 标记会让程序把数据目录固定为旁边的 data/。
首次启动会从 data/.official/ 播种内置资源到用户 data/；存档、设置、日志也写在
data/ 下。整个目录可随意移动或改名，只要 ling_chat 与 data/ 保持同级即可。

可选：安装桌面快捷方式
    ./install-desktop-entry.sh

系统依赖（发行版包名可能略有差异）：
- webkit2gtk-4.1（Debian/Ubuntu: libwebkit2gtk-4.1-0）
- GTK 3
- libayatana-appindicator（系统托盘）
- PipeWire（屏幕采集相关）
- Vulkan ICD 加载器（WebGPU / GPU 枚举；无独显时仍建议安装）

环境变量（可选）：
- LINGCHAT_DATA_DIR     覆盖数据目录
- LINGCHAT_RESOURCE_DIR 覆盖只读资源根（需含 data/.official 或 data/third_party）
EOF

# 兜底校验一：若随包带了 Dawn，二进制必须带 $ORIGIN RUNPATH
if [[ -e "${STAGE}/libwebgpu_dawn.so" ]] && command -v readelf >/dev/null 2>&1; then
  if ! readelf -d "${STAGE}/ling_chat" | grep -q 'ORIGIN'; then
    echo "ling_chat 缺少 ORIGIN RUNPATH，随包分发的 libwebgpu_dawn.so 将无法加载" >&2
    readelf -d "${STAGE}/ling_chat" | grep -iE 'rpath|runpath' >&2 || echo '  （无 RUNPATH/RPATH）' >&2
    exit 1
  fi
fi

# 兜底校验二：分发内容不允许残留任何符号链接
if find "${STAGE}" -type l | grep -q .; then
  echo "打包内容中残留符号链接，指向构建机路径会在用户机器上失效：" >&2
  find "${STAGE}" -type l -printf '  %p -> %l\n' >&2
  exit 1
fi

mkdir -p "${OUT_DIR}"
TAR_PATH="${OUT_DIR}/${PKG_NAME}.tar.gz"
tar -czf "${TAR_PATH}" -C "${OUT_DIR}" "${PKG_NAME}"

# 旁路输出 sha256，便于 Release / AUR updpkgsums
(
  cd "${OUT_DIR}"
  sha256sum "${PKG_NAME}.tar.gz" > "${PKG_NAME}.tar.gz.sha256"
)

rm -rf "${STAGE}"

echo "已生成 ${TAR_PATH}"
echo "校验和 ${TAR_PATH}.sha256"
ls -lh "${TAR_PATH}" "${TAR_PATH}.sha256"
