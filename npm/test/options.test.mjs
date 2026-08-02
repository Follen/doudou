import assert from "node:assert/strict";
import path from "node:path";
import test from "node:test";

import {
  detectCliCommand,
  parseLauncherOptions,
  resolveSkillDirectory,
} from "../lib/options.mjs";

test("launcher options are removed before Python forwarding", () => {
  const result = parseLauncherOptions([
    "--data-root",
    "project",
    "--skill-dir",
    "custom-skill",
    "doctor",
  ]);
  assert.equal(result.skillDirectory, "custom-skill");
  assert.deepEqual(result.forwarded, ["--data-root", "project", "doctor"]);
});

test("explicit Skill directory wins over the Agent home", () => {
  const result = resolveSkillDirectory({
    explicit: "explicit",
    env: { DOUDOU_AGENT_HOME: "agent" },
    home: "home",
  });
  assert.equal(result, path.resolve("explicit"));
});

test("Agent home resolves to skills/doudou", () => {
  const result = resolveSkillDirectory({
    env: { DOUDOU_AGENT_HOME: path.join("root", "agent") },
    home: "home",
  });
  assert.equal(result, path.resolve("root", "agent", "skills", "doudou"));
});

test("missing --skill-dir value is rejected", () => {
  assert.throws(() => parseLauncherOptions(["doctor", "--skill-dir"]));
});

test("top-level command detection skips the data root value", () => {
  assert.equal(detectCliCommand(["--data-root", "setup", "profile", "list"]), "profile");
  assert.equal(detectCliCommand(["--data-root=doctor", "messages", "collect"]), "messages");
});

test("unknown first positional argument is not mistaken for a later command", () => {
  assert.equal(detectCliCommand(["workspace", "setup"]), undefined);
});
