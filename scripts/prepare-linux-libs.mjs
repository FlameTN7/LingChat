import { execFileSync } from "node:child_process";
import { chmodSync, copyFileSync, existsSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(__dirname, "..");

const RUNTIME_LIBS = ["libwebgpu_dawn.so"];

function findFileRecursive(dir, fileName, maxDepth = 4) {
  if (maxDepth < 0 || !existsSync(dir)) return undefined;
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return undefined;
  }
  for (const entry of entries) {
    const full = join(dir, entry.name);
    if (
      entry.name === fileName &&
      (entry.isFile() || (entry.isSymbolicLink() && existsSync(full)))
    ) {
      return full;
    }
  }
  for (const entry of entries) {
    if (entry.isDirectory()) {
      const found = findFileRecursive(join(dir, entry.name), fileName, maxDepth - 1);
      if (found) return found;
    }
  }
  return undefined;
}

export function copyRuntimeLibs(profileDir, destDir, expectedRunpath) {
  const binary = join(profileDir, "ling_chat");
  const dynamic = execFileSync("readelf", ["-W", "-d", binary], {
    encoding: "utf8",
    env: { ...process.env, LC_ALL: "C" },
  });
  const needed = Array.from(dynamic.matchAll(/\(NEEDED\)[^\n]*?\[([^\]]+)\]/g), (m) => m[1]);
  const runpath =
    dynamic.match(/\(RUNPATH\)[^\n]*?\[([^\]]*)\]/)?.[1] ??
    dynamic.match(/\(RPATH\)[^\n]*?\[([^\]]*)\]/)?.[1] ??
    "";
  const searchPaths = runpath.replaceAll("${ORIGIN}", "$ORIGIN").split(":");
  for (const name of RUNTIME_LIBS) {
    if (!needed.includes(name)) continue;
    if (!searchPaths.includes(expectedRunpath)) {
      throw new Error(`${binary} 依赖 ${name}，但运行时搜索路径不含 ${expectedRunpath}`);
    }
    const direct = join(profileDir, name);
    const source = existsSync(direct) ? direct : findFileRecursive(join(profileDir, "build"), name);
    if (!source) throw new Error(`${binary} 依赖 ${name}，但在 ${profileDir} 内未找到该库`);
    const destination = join(destDir, name);
    copyFileSync(source, destination);
    chmodSync(destination, 0o755);
    console.log(`[linux-libs] 已复制 ${source} → ${destination}`);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  if (process.platform !== "linux") process.exit(0);
  const releaseDir = join(projectRoot, "target", "release");
  const debugDir = join(projectRoot, "target", "debug");
  const useDebug =
    process.env.TAURI_ENV_DEBUG === "true" ||
    (process.env.TAURI_ENV_DEBUG === undefined && !existsSync(join(releaseDir, "ling_chat")));

  const destDir = join(projectRoot, "src-tauri", "linux-libs");
  rmSync(destDir, { recursive: true, force: true });
  mkdirSync(destDir, { recursive: true });
  copyRuntimeLibs(useDebug ? debugDir : releaseDir, destDir, "$ORIGIN/../lib/LingChat/lib");
}
