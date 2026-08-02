# Acceptance evidence

<!-- comet-native:acceptance-evidence:start -->
[
  {
    "acceptance_id": "acceptance-04d7f6c12c9eb9d14108963d4b8700c750ddee79f7969c7358c7307b6cf5e1a0",
    "evidence_refs": [
      ".github/workflows/publish-npm.yml",
      "package.json"
    ]
  },
  {
    "acceptance_id": "acceptance-39063580f8aba82c22563af428ce1165888ac4b29dcdfba7baf30290960fd8bb",
    "evidence_refs": [
      "npm/bin/doudou.mjs",
      "npm/scripts/test-install.mjs"
    ]
  },
  {
    "acceptance_id": "acceptance-4071d9bd8215924256b5b0a0c06ab1109cdf599f598dada5cbc26f20fb94c436",
    "evidence_refs": [
      "doudou/scripts/doudou/runtime/paths.py",
      "npm/scripts/verify-package.mjs"
    ]
  },
  {
    "acceptance_id": "acceptance-696bb2ab1445b1862f177b8bcd9eb5b44a9df436781b76ba48b8046232a57fc1",
    "evidence_refs": [
      "npm/scripts/verify-package.mjs",
      "npm/skill-manifest.json"
    ]
  },
  {
    "acceptance_id": "acceptance-6d90edce49e421cc359c4ca8fc897f74f7f7b5b4152a3905120473422007f386",
    "evidence_refs": [
      "npm/lib/skill-installer.mjs",
      "npm/test/skill-installer.test.mjs"
    ]
  },
  {
    "acceptance_id": "acceptance-b6c8bffbfb16b8e12bb4ed1b059148a3f6172bf1c4b0a3d787826cb38b84e90c",
    "evidence_refs": [
      "README.md",
      "npm/bin/doudou.mjs",
      "npm/lib/python.mjs"
    ]
  }
]
<!-- comet-native:acceptance-evidence:end -->

# Commands and results

- `npm test` returned 0; 14 tests passed.
- `npm run check:manifest` returned 0; all 65 Skill files match the package manifest.
- `npm run test:pack` returned 0; 80 files and 128666 packed bytes passed the whitelist and credential checks.
- `npm run test:install` returned 0; global install, npx and `--ignore-scripts` recovery passed through the generated Windows shim.
- `npm run check:release -- v2.1.1` returned 0; tag, package and Python Skill versions match.
- `$env:PYTHONPATH='doudou\\scripts'; python -m pytest docs\\verification\\tests -q` returned 0; 83 tests and 5 subtests passed.
- `python -m ruff check doudou\\scripts docs\\verification\\tests` returned 0 with no findings.
- `python C:\\Users\\follen\\.codex\\skills\\.system\\skill-creator\\scripts\\quick_validate.py doudou` returned 0; the Skill is valid.
- Workflow structure validation returned 0; `test` and `publish` jobs exist and publish has `id-token: write`.
- Credential pattern scan returned 0 with no credential-like values.
- `npm view @follenfang/doudou versions dist-tags --json` confirmed `0.0.0-bootstrap.0` under the `bootstrap` dist-tag.
- The user confirmed the npm Trusted Publisher was saved for `Follen/doudou`, workflow `publish-npm.yml`, blank environment and `npm publish` permission only.

# Skipped checks

The `v2.1.1` tag release is an operational step after archive and push; its GitHub Actions run and final registry version are checked immediately after the archived change is committed.

# Spec consistency

The implementation matches the approved private-repository contract. It uses OIDC Trusted Publishing without a long-lived npm token. Provenance is not forced because npm does not support provenance for private GitHub repositories. Profile, message, browser, venv, cache and analyze data remain outside the npm package and installed Skill.

# Known limitations and risks

The bootstrap version temporarily owns the npm `latest` dist-tag because it is the first published version. Publishing `v2.1.1` will move `latest` to the formal release. A failed tag workflow leaves `latest` on bootstrap and must be repaired before announcing the release.

# Conclusion

PASS. The implementation and local distribution gates are complete; archive, repository push, tag push and post-release verification remain as the controlled release sequence.
