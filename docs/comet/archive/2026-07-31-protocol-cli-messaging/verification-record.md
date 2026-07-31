# Verification record

Verified at: 2026-07-31T12:36:00Z

## Automated tests

Command: `cd douyin-message && npm ci && npm test`

Exit status: `0`

Literal summary:

```text
added 2 packages, and audited 3 packages
found 0 vulnerabilities
tests 38
pass 38
fail 0
cancelled 0
skipped 0
todo 0
```

The Protobuf message test covers both `conversation_type=2` and `conversation_type=1`, plus rejection when the decoded type does not match the current conversation.

## Skill validation

Command: `python C:/Users/follen/.codex/skills/.system/skill-creator/scripts/quick_validate.py D:/Code/douyin-group-message/douyin-message`

Exit status: `0`

Literal output: `Skill is valid!`

## Isolated package setup

Commands:

```text
npm pack --pack-destination <TEMP>/pack --json
tar -xzf <TEMP>/pack/douyin-message-1.0.0.tgz -C <TEMP>/extract
node <TEMP>/extract/package/bin/douyin-message.js doctor --data-root <TEMP>/user
node <TEMP>/extract/package/bin/douyin-message.js setup --data-root <TEMP>/user
node <TEMP>/extract/package/bin/douyin-message.js doctor --data-root <TEMP>/user
```

All process exit statuses: `0`.

Literal package results:

```text
PACKAGE_SHA1=082ab927185c308e0f1329fa39b17fb99da0dc4a
PACKAGE_SIZE=59222
PACKAGE_FILES=25
```

Literal doctor/setup results:

```text
before setup: status=setup_required
before setup: playwright.expected_version=1.62.0
before setup: playwright.installed_version=null
setup: npm_ci=completed
setup: chromium_install=completed
after setup: status=ok
after setup: node.version=24.13.1
after setup: npm.version=11.18.0
after setup: playwright.installed_version=1.62.0
after setup: chromium.browser_version=151.0.7922.34
after setup: chromium.launch_ok=true
after setup: issues=[]
```

Literal layout audit:

```text
USER_ROOT=message,Profile
SKILL/Profile=False
SKILL/message=False
SKILL/analyze=False
SKILL/node_modules=False
```

The temporary package, installed dependencies and user project were removed after verification.

## Packaging inventory

`npm pack --dry-run --json` returned 25 files. The archive contains `SKILL.md`, `agents/`, `bin/`, `references/`, `scripts/`, `src/`, `test/`, `npm-shrinkwrap.json` and `package.json`. npm does not include `.gitignore` in the tarball by design; the deliverable source folder contains it. The archive contains no Profile, message, analyze, node_modules or browser binary.

## Live checks not run

No headed login, live conversation collection, live protocol capture or real message send was performed during this verification. Protocol send/reply coverage is limited to preview integrity, Protobuf request rewriting and response-verification logic exercised by automated tests.
