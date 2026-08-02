import assert from "node:assert/strict";
import path from "node:path";
import test from "node:test";

import {
  installationPrefix,
  removeCurrentNpmPackage,
} from "../lib/self-remove.mjs";

test("derives Windows and Linux global npm prefixes", () => {
  assert.equal(
    installationPrefix("C:\\tools\\node_modules\\@follenfang\\doudou", path.win32),
    path.win32.resolve("C:\\tools"),
  );
  assert.equal(
    installationPrefix("/opt/doudou/lib/node_modules/@follenfang/doudou", path.posix),
    path.posix.resolve("/opt/doudou"),
  );
});

test("self-removal invokes npm-cli without a shell and with the current prefix", () => {
  let invocation;
  const prefix = path.join(path.parse(process.cwd()).root, "doudou-self-remove");
  const packageRoot = path.join(
    prefix,
    ...(process.platform === "win32" ? [] : ["lib"]),
    "node_modules",
    "@follenfang",
    "doudou",
  );
  const result = removeCurrentNpmPackage({
    packageRoot,
    platform: process.platform,
    npmCli: "C:\\npm\\npm-cli.js",
    run(command, args, options) {
      invocation = { command, args, options };
      return { status: 0, stdout: "removed", stderr: "" };
    },
  });
  assert.equal(invocation.command, process.execPath);
  assert.deepEqual(invocation.args, [
    "C:\\npm\\npm-cli.js",
    "uninstall",
    "-g",
    "--ignore-scripts",
    "--prefix",
    path.resolve(prefix),
    "@follenfang/doudou",
  ]);
  assert.equal(invocation.options.shell, false);
  assert.equal(result.status, "removed");
});

test("self-removal reports npm failure", () => {
  const prefix = path.join(path.parse(process.cwd()).root, "doudou-self-remove");
  const packageRoot = path.join(
    prefix,
    ...(process.platform === "win32" ? [] : ["lib"]),
    "node_modules",
    "@follenfang",
    "doudou",
  );
  assert.throws(
    () =>
      removeCurrentNpmPackage({
        packageRoot,
        platform: process.platform,
        npmCli: "/usr/lib/node_modules/npm/bin/npm-cli.js",
        run: () => ({ status: 1, stdout: "", stderr: "permission denied" }),
      }),
    /permission denied/,
  );
});
