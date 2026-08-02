import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  compareVersions,
  latestVersion,
  resolveNpmCli,
  updateInstallation,
} from "../lib/update.mjs";

function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "doudou-update-test-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const globalNodeModules = path.join(root, "global", "node_modules");
  const packageRoot = path.join(globalNodeModules, "@follenfang", "doudou");
  fs.mkdirSync(packageRoot, { recursive: true });
  fs.writeFileSync(path.join(packageRoot, "package.json"), JSON.stringify({ version: "0.0.3" }));
  const runtimeManifest = path.join(packageRoot, "doudou", "scripts", "runtime-manifest.json");
  fs.mkdirSync(path.dirname(runtimeManifest), { recursive: true });
  fs.writeFileSync(runtimeManifest, JSON.stringify({ runtime_contract: "old" }));
  const cli = path.join(root, "fake-npm.cjs");
  fs.writeFileSync(
    cli,
    `const fs=require('node:fs'); const path=require('node:path'); const args=process.argv.slice(2); if(args[0]==='view') console.log(JSON.stringify(process.env.FAKE_LATEST)); else if(args[0]==='root') console.log(process.env.FAKE_ROOT); else if(args[0]==='install') { const target=args.at(-1); const contract=target.endsWith('@'+process.env.FAKE_LATEST)?process.env.FAKE_NEW_CONTRACT:process.env.FAKE_OLD_CONTRACT; const file=path.join(process.env.FAKE_PACKAGE_ROOT,'doudou','scripts','runtime-manifest.json'); fs.mkdirSync(path.dirname(file),{recursive:true}); fs.writeFileSync(file,JSON.stringify({runtime_contract:contract})); } else process.exit(2);`,
  );
  const env = { ...process.env, FAKE_LATEST: "0.0.3", FAKE_ROOT: globalNodeModules, FAKE_PACKAGE_ROOT: packageRoot, FAKE_OLD_CONTRACT: "old", FAKE_NEW_CONTRACT: "new" };
  return { root, packageRoot, cli, env };
}

test("update --check reports latest without modifying installation", async (t) => {
  const fixtureValue = fixture(t);
  let ensured = false;
  const result = await updateInstallation({
    packageRoot: fixtureValue.packageRoot,
    currentVersion: "0.0.2",
    check: true,
    env: fixtureValue.env,
    npmCli: fixtureValue.cli,
    ensureSkill() { ensured = true; },
  });
  assert.equal(result.status, "available");
  assert.equal(result.changed, false);
  assert.equal(ensured, false);
});

test("npm CLI resolves beside Node when lifecycle variables are absent", (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "doudou-node-layout-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const execPath = path.join(root, process.platform === "win32" ? "node.exe" : "node");
  const npmCli = path.join(root, "node_modules", "npm", "bin", "npm-cli.js");
  fs.mkdirSync(path.dirname(npmCli), { recursive: true });
  fs.writeFileSync(npmCli, "");
  assert.equal(resolveNpmCli({ execPath, env: {} }), npmCli);
});

test("npm lifecycle path takes precedence over the adjacent installation", () => {
  assert.equal(
    resolveNpmCli({
      execPath: path.join("missing", "node"),
      env: { npm_execpath: path.join("custom", "npm-cli.js") },
    }),
    path.join("custom", "npm-cli.js"),
  );
});

test("npm launch failure is reported without dereferencing missing output", () => {
  assert.throws(
    () => latestVersion({ npmCli: path.join("missing", "npm-cli.js") }),
    error => error instanceof Error && !error.message.includes("trim"),
  );
});

test("update comparison never treats a newer local version as downgradeable", () => {
  assert.equal(compareVersions("0.0.1", "0.0.2"), -1);
  assert.equal(compareVersions("1.0.0", "1.0.0-beta.2"), 1);
  assert.equal(compareVersions("1.0.0-beta.10", "1.0.0-beta.2"), 8);
});

test("update --check reports current when local is newer than registry", async (t) => {
  const fixtureValue = fixture(t);
  fixtureValue.env.FAKE_LATEST = "0.0.1";
  const result = await updateInstallation({
    packageRoot: fixtureValue.packageRoot,
    currentVersion: "0.0.2",
    check: true,
    env: fixtureValue.env,
    npmCli: fixtureValue.cli,
  });
  assert.equal(result.status, "current");
  assert.equal(result.changed, false);
});

test("global update upgrades npm, Skill, and a changed runtime contract", async (t) => {
  const fixtureValue = fixture(t);
  let source;
  let runtimeSource;
  const result = await updateInstallation({
    packageRoot: fixtureValue.packageRoot,
    currentVersion: "0.0.2",
    env: fixtureValue.env,
    npmCli: fixtureValue.cli,
    ensureSkill(packageRoot) { source = packageRoot; return { status: "upgraded" }; },
    ensureRuntime(packageRoot) { runtimeSource = packageRoot; return { status: "installed" }; },
  });
  assert.equal(result.status, "updated");
  assert.equal(result.latest, "0.0.3");
  assert.equal(source, fixtureValue.packageRoot);
  assert.equal(runtimeSource, fixtureValue.packageRoot);
  assert.equal(result.runtime.status, "installed");
});

test("runtime update failure restores the previous npm and Skill contract", async (t) => {
  const fixtureValue = fixture(t);
  let skillCalls = 0;
  await assert.rejects(
    updateInstallation({
      packageRoot: fixtureValue.packageRoot,
      currentVersion: "0.0.2",
      env: fixtureValue.env,
      npmCli: fixtureValue.cli,
      ensureSkill() { skillCalls += 1; return { status: "upgraded" }; },
      ensureRuntime() { throw new Error("runtime probe failed"); },
    }),
    /runtime probe failed/,
  );
  assert.equal(skillCalls, 2);
  const manifest = JSON.parse(
    fs.readFileSync(
      path.join(fixtureValue.packageRoot, "doudou", "scripts", "runtime-manifest.json"),
      "utf8",
    ),
  );
  assert.equal(manifest.runtime_contract, "old");
});

test("npx update only reports and never creates a global install", async (t) => {
  const fixtureValue = fixture(t);
  const npxRoot = path.join(fixtureValue.root, "cache", "_npx", "abc", "node_modules", "@follenfang", "doudou");
  const result = await updateInstallation({
    packageRoot: npxRoot,
    currentVersion: "0.0.2",
    env: fixtureValue.env,
    npmCli: fixtureValue.cli,
    ensureSkill() { throw new Error("must not run"); },
  });
  assert.equal(result.mode, "npx");
  assert.equal(result.changed, false);
});
