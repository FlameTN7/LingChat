#!/usr/bin/env node
/**
 * Offline smoke runner for memory-test-api. It builds/tests the feature-gated
 * binary, verifies authentication and loopback API behavior, then shuts it down.
 */
import { spawn, spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const configuredCargo = process.env.CARGO ?? (process.platform === "win32" ? "cargo.exe" : "cargo");
const managedRoot = "D:\\DevEnvs\\Rust";
const managedCargo = `${managedRoot}\\.cargo\\bin\\cargo.exe`;
const managedRustc = `${managedRoot}\\.rustup\\toolchains\\stable-x86_64-pc-windows-msvc\\bin\\rustc.exe`;
const cargo = process.env.CARGO || (process.platform === "win32" && existsSync(managedCargo) ? managedCargo : configuredCargo);
const manifest = join(root, "src-tauri", "Cargo.toml");
const args = ["--manifest-path", manifest, "--lib", "--features", "memory-test-api"];

function run(command, commandArgs) {
  const env = { ...process.env };
  if (!env.RUSTUP_TOOLCHAIN && command === managedCargo) {
    env.RUSTUP_TOOLCHAIN = "stable";
    env.RUSTC = managedRustc;
    env.PATH = `${managedRoot}\\.rustup\\toolchains\\stable-x86_64-pc-windows-msvc\\bin;${managedRoot}\\.cargo\\bin;${env.PATH ?? ""}`;
  }
  const result = spawnSync(command, commandArgs, { cwd: root, stdio: "inherit", env });
  if (result.status !== 0) process.exit(result.status ?? 1);
}

run(cargo, ["test", ...args]);
run(cargo, ["build", "--manifest-path", manifest, "--features", "memory-test-api", "--bin", "memory-test-api"]);

const binary = join(root, "src-tauri", "target", "debug", process.platform === "win32" ? "memory-test-api.exe" : "memory-test-api");
if (!existsSync(binary)) throw new Error(`memory-test-api binary not found: ${binary}`);

const childEnv = { ...process.env };
if (!childEnv.RUSTUP_TOOLCHAIN && cargo === managedCargo) {
  childEnv.RUSTUP_TOOLCHAIN = "stable";
  childEnv.RUSTC = managedRustc;
  childEnv.PATH = `${managedRoot}\\.rustup\\toolchains\\stable-x86_64-pc-windows-msvc\\bin;${managedRoot}\\.cargo\\bin;${childEnv.PATH ?? ""}`;
}
const child = spawn(binary, [], { cwd: root, stdio: ["ignore", "pipe", "inherit"], env: childEnv });
let buffer = "";
const ready = await new Promise((resolveReady, reject) => {
  const timer = setTimeout(() => reject(new Error("memory-test-api ready timeout")), 30_000);
  child.stdout.on("data", (chunk) => {
    buffer += chunk;
    const newline = buffer.indexOf("\n");
    if (newline < 0) return;
    const line = buffer.slice(0, newline).trim();
    clearTimeout(timer);
    try { resolveReady(JSON.parse(line)); } catch (error) { reject(error); }
  });
  child.once("error", reject);
});
if (ready.host !== "127.0.0.1" || !ready.port || !ready.token) throw new Error("invalid ready response");
const base = `http://${ready.host}:${ready.port}`;
const auth = { Authorization: `Bearer ${ready.token}` };

const unauthorized = await fetch(`${base}/health`);
if (unauthorized.status !== 401) throw new Error(`health auth status: ${unauthorized.status}`);
const health = await fetch(`${base}/health`, { headers: auth });
if (health.status !== 200 || !(await health.json()).ok) throw new Error("health failed");

const success = await fetch(`${base}/v1/memory/validate`, {
  method: "POST", headers: { ...auth, "content-type": "application/json" },
  body: JSON.stringify({ scenario: "basic-compression" }),
});
const successBody = await success.json();
if (success.status !== 200 || successBody.outcome !== "succeeded" || successBody.calls !== 4 || !successBody.committed) {
  throw new Error(`successful validation failed: ${JSON.stringify(successBody)}`);
}

const failure = await fetch(`${base}/v1/memory/validate`, {
  method: "POST", headers: { ...auth, "content-type": "application/json" },
  body: JSON.stringify({ scenario: "one-section-fails", fail_section: "promises" }),
});
const failureBody = await failure.json();
if (failure.status !== 200 || failureBody.outcome !== "not_committed" || failureBody.committed || failureBody.last_processed_global_idx !== 0) throw new Error(`failure scenario failed: ${JSON.stringify(failureBody)}`);

const roundTrip = await fetch(`${base}/v1/scenarios/persistence-roundtrip`, {
  method: "POST", headers: { ...auth, "content-type": "application/json" }, body: JSON.stringify({}),
});
const roundTripBody = await roundTrip.json();
if (roundTrip.status !== 200 || roundTripBody.outcome !== "succeeded" || roundTripBody.persistence_roundtrip !== true) throw new Error(`round-trip scenario failed: ${JSON.stringify(roundTripBody)}`);

const shutdown = await fetch(`${base}/shutdown`, { method: "POST", headers: auth });
if (shutdown.status !== 200) throw new Error(`shutdown failed: ${shutdown.status}`);
await new Promise((resolveExit, reject) => {
  const timer = setTimeout(() => { child.kill(); reject(new Error("service shutdown timeout")); }, 5_000);
  child.once("exit", () => { clearTimeout(timer); resolveExit(); });
});
console.log(JSON.stringify({ ok: true, api_version: ready.api_version, scenarios: ["basic-compression", "one-section-fails", "persistence-roundtrip"] }));
