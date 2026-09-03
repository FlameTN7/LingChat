#!/usr/bin/env node
/** Full offline memory-test-api verification with a machine-readable report. */
import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const artifacts = join(root, "test", "artifacts");
mkdirSync(artifacts, { recursive: true });
const cargo = process.env.CARGO ?? (process.platform === "win32" ? "cargo.exe" : "cargo");
const manifest = join(root, "src-tauri", "Cargo.toml");
const cargoEnv = { ...process.env };
if (process.platform === "win32" && !cargoEnv.RUSTUP_TOOLCHAIN) {
  cargoEnv.RUSTUP_TOOLCHAIN = "stable";
  cargoEnv.RUSTC ??= "D:/DevEnvs/Rust/.rustup/toolchains/stable-x86_64-pc-windows-msvc/bin/rustc.exe";
  cargoEnv.PATH = `D:/DevEnvs/Rust/.rustup/toolchains/stable-x86_64-pc-windows-msvc/bin;D:/DevEnvs/Rust/.cargo/bin;${cargoEnv.PATH ?? ""}`;
}
const cargoArgs = ["--manifest-path", manifest, "--features", "memory-test-api", "--bin", "memory-test-api"];
const run = (args) => {
  const result = spawnSync(cargo, args, { cwd: root, stdio: "inherit", env: cargoEnv });
  if (result.status !== 0) process.exit(result.status ?? 1);
};

// The package command is intentionally self-contained: prepare first, then compile/test.
const prepare = spawnSync(process.execPath, [join(root, "scripts", "prepare-desktop-resources.mjs")], { cwd: root, stdio: "inherit" });
if (prepare.status !== 0) process.exit(prepare.status ?? 1);
run(["test", ...cargoArgs, "--lib"]);
run(["build", ...cargoArgs]);
const binary = join(root, "src-tauri", "target", "debug", process.platform === "win32" ? "memory-test-api.exe" : "memory-test-api");
if (!existsSync(binary)) throw new Error(`memory-test-api binary not found: ${binary}`);
const child = spawn(binary, [], { cwd: root, stdio: ["ignore", "pipe", "inherit"], env: cargoEnv });
let buffer = "";
const ready = await new Promise((resolveReady, reject) => {
  const timer = setTimeout(() => reject(new Error("memory-test-api ready timeout")), 30_000);
  child.stdout.on("data", (chunk) => {
    buffer += chunk;
    const newline = buffer.indexOf("\n");
    if (newline < 0) return;
    clearTimeout(timer);
    try { resolveReady(JSON.parse(buffer.slice(0, newline).trim())); } catch (error) { reject(error); }
  });
  child.once("error", reject);
});
if (ready.host !== "127.0.0.1" || !ready.port || !ready.token) throw new Error("invalid ready response");
const base = `http://${ready.host}:${ready.port}`;
const auth = { Authorization: `Bearer ${ready.token}` };
const scenarios = [
  "basic-compression", "append-during-update", "one-section-fails", "empty-section-fails",
  "stale-on-rollback", "persistence-roundtrip", "memory-finishes-after-line-save",
];
const results = [];
try {
  for (const name of scenarios) {
    const response = await fetch(`${base}/v1/scenarios/${name}`, {
      method: "POST", headers: { ...auth, "content-type": "application/json" }, body: "{}",
    });
    const body = await response.json();
    if (response.status !== 200) throw new Error(`${name}: HTTP ${response.status} ${JSON.stringify(body)}`);
    results.push({ name, status: response.status, outcome: body.outcome, calls: body.calls, committed: body.committed, duration_ms: body.duration_ms, body });
    if (name === "basic-compression" && (body.outcome !== "succeeded" || body.calls !== 4)) throw new Error(`${name} assertion failed`);
    if (["one-section-fails", "empty-section-fails", "stale-on-rollback"].includes(name) && body.committed) throw new Error(`${name} unexpectedly committed`);
    if (["persistence-roundtrip", "memory-finishes-after-line-save"].includes(name) && body.persistence_roundtrip !== true) throw new Error(`${name} did not round-trip`);
  }
  const unauthorized = await fetch(`${base}/health`);
  if (unauthorized.status !== 401) throw new Error(`health auth status ${unauthorized.status}`);
  const report = { ok: true, api_version: ready.api_version, scenarios: results, generated_at: new Date().toISOString() };
  writeFileSync(join(artifacts, "memory-report.json"), JSON.stringify(report, null, 2) + "\n");
  console.log(JSON.stringify({ ok: true, scenarios: scenarios }));
} finally {
  try { await fetch(`${base}/shutdown`, { method: "POST", headers: auth }); } catch {}
  await new Promise((resolveExit) => {
    const timer = setTimeout(() => { child.kill(); resolveExit(); }, 5_000);
    child.once("exit", () => { clearTimeout(timer); resolveExit(); });
  });
}
