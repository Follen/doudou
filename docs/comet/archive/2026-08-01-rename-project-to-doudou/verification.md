# Acceptance evidence

<!-- comet-native:acceptance-evidence:start -->
[
  {
    "acceptance_id": "acceptance-0758e472777fe8c598eb5e8fc3714ba9a5d8a6e0b9f6132a42d7ec307a1ab1a9",
    "evidence_refs": [
      "README.md",
      "doudou/SKILL.md",
      "doudou/bin/doudou.js",
      "doudou/src/cli.js"
    ]
  },
  {
    "acceptance_id": "acceptance-a703ed31c35aecb34ac2bf23bd1ec0dd159fcad6626e648cc0aa702ea5735f85",
    "evidence_refs": [
      "README.md"
    ]
  },
  {
    "acceptance_id": "acceptance-ac796d1b978efc2d055465193cdafac4c2b155a94df7809ef93e4d579c9960e6",
    "evidence_refs": [
      "doudou/src/fingerprint.js",
      "doudou/src/profile.js",
      "doudou/src/protocol.js",
      "doudou/src/schemas.js",
      "doudou/src/storage.js"
    ]
  },
  {
    "acceptance_id": "acceptance-bdf665e9203b9274d93f64f56865705e41f04e0de8881f2d8b18a01aae87d395",
    "evidence_refs": [
      "README.md",
      "doudou/SKILL.md",
      "doudou/agents/openai.yaml",
      "doudou/package.json"
    ]
  },
  {
    "acceptance_id": "acceptance-c17ec144d0dae3345fb45890942a1b1379ebb2a54f0e2968f435bee923b9fdd0",
    "evidence_refs": [
      "doudou/bin/doudou.js",
      "doudou/npm-shrinkwrap.json",
      "doudou/package.json"
    ]
  }
]
<!-- comet-native:acceptance-evidence:end -->

# Commands and results

- `npm test` in `doudou/`: exit `0`; 62 tests passed, 0 failed, 0 skipped.
- Initial `npm test` in `.agents/skills/doudou/`: exit `1`; 5 tests failed because the newly installed Skill had no local Playwright dependency. This was an expected first-install state, not reported as success.
- `npm ci && npm test` in `.agents/skills/doudou/`: exit `0`; npm installed 2 packages with 0 vulnerabilities; 62 tests passed, 0 failed, 0 skipped.
- `node .agents/skills/doudou/bin/doudou.js help`: exit `0`; heading, usage and every command use `doudou`.
- `npm pack --dry-run --json` in `doudou/`: exit `0`; package name `doudou`, 32 entries, bin `bin/doudou.js`; no runtime Profile, messages, analyze data, browser binaries, old bin or old install path.
- Exact old-brand scan across `README.md`, `.gitignore`, `doudou/` and all proposed canonical specs: 0 matches for the two old product names and the old environment variable. Real Douyin platform names, domains and `douyin-web` source metadata remain intentionally.
- SHA-256 comparison between `doudou/` and `.agents/skills/doudou/`, excluding installed `node_modules/`: 33 files checked, 0 mismatches.
- Old schema rejection fixture: `validateRecord({schema: <old-product>.message.v1})` returned `UNKNOWN_SCHEMA`; no migration or compatibility path ran.
- `comet native check rename-project-to-doudou`: exit `0`; receipt `runtime/evidence/check-receipts/325f85238d217da329ac2472fe9e6644425705599609b36eaa0f17a160fe87d1.json`.
- GitHub REST rename: repository result `Follen/doudou`, private `true`, default branch `main`, URL `https://github.com/Follen/doudou`; local fetch and push origin are `https://github.com/Follen/doudou.git`.

# Skipped checks

- No live message was sent and no browser login was repeated because the change only renames project identity and retains protocol behavior; the complete automated suite covers the unchanged workflows.
- Existing project-local `profile/` and `message/` records were not rewritten or deleted, per the confirmed non-migration decision.
- Historical `docs/comet/archive/`, `analyze/`, diagnostics and old receipts were intentionally excluded from product-brand replacement and residual scanning.

# Spec consistency

- The delivery folder, Skill frontmatter, Agent metadata, npm package/bin, CLI help, repair commands, environment variable, schema prefixes, Profile lock owner, migration temporary name, test fixtures, README and install paths use `doudou`.
- The old project CLI, Skill name, bin and environment variable are absent from the active product surface.
- Proposed canonical specs create capability `doudou`, remove the old branded capability, and update the protocol and prewarm capabilities to name Doudou.
- Actual Douyin platform URLs, protocol endpoints, page titles and `douyin-web` source markers remain unchanged.
- The old project-local Skill was moved from `.agents/skills/` into the ignored `analyze/` history area, so it is no longer loadable as an active Skill.

# Known limitations and risks

- Existing old-format Profile metadata, fingerprints, message records and locks are intentionally unsupported. Their files remain untouched; users must regenerate metadata with the new CLI.
- GitHub redirects from the old repository URL may continue to work as a GitHub service behavior, but Doudou does not publish an old CLI or Skill compatibility alias.
- Canonical Comet spec replacement becomes final during Archive; the proposed specs already passed the old-brand residual scan.

# Conclusion

Pass. The active project and Skill identity are consistently `doudou`, the package and installed Skill are runnable, old schemas fail closed, historical evidence remains immutable, and the GitHub private repository plus local origin use the new name.
