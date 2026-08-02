# Commands and results

- `npm test`：通过，25 tests。
- `npm run build:manifest`、`npm run check:manifest`、`npm run check:release -- v0.0.2`：通过。
- `npm run test:pack`：通过，88 files，136577 bytes。
- `npm run test:install`：通过；覆盖隔离全局安装、npx、禁用 lifecycle scripts、自卸载及项目数据保留。
- `$env:PYTHONPATH='doudou\scripts'; python -m pytest docs\verification\tests -q`：通过，85 tests、5 subtests。
- `python -m ruff check doudou\scripts docs\verification\tests`：通过。
- `quick_validate.py doudou`：通过，Skill is valid。
- 凭据模式扫描：零命中。

# Skipped checks

- 本地未重复执行 Linux 真实安装；对应检查由 tag 发布 workflow 的 Linux matrix 执行。

# Spec consistency

- 16 个验收场景均映射到自动化证据；实现与 `managed-uninstall` 规格一致。
- Skill 和 README 均只引导 `doudou uninstall`，没有宣称直接 npm 卸载会清理运行文件。

# Known limitations and risks

- npm 7+ 不执行被卸载包的 uninstall lifecycle；完整卸载入口必须使用 `doudou uninstall`。
- 本地只在 Windows 执行真实打包安装回归；Linux 由 tag 发布 workflow 的 matrix 门禁执行。

# Conclusion

通过。`doudou uninstall` 的受管清理、自删除、失败闭合和数据保留契约均有自动化覆盖；版本 `0.0.2` 的发布包、Skill 格式和凭据门禁通过。

# Acceptance evidence

<!-- comet-native:acceptance-evidence:start -->
[
  {
    "acceptance_id": "acceptance-073661c71cd3040877ea6099f410cda49a6b724ae596f0467d4d2020bae8f704",
    "evidence_refs": [
      "docs/verification/tests/test_broker_registration.py",
      "npm/test/uninstall.test.mjs"
    ]
  },
  {
    "acceptance_id": "acceptance-10fdd637c44660c2650d4fcecda5aa6084ebd4f4f97ce57577b123af0ea7b467",
    "evidence_refs": [
      "npm/test/managed-install.test.mjs",
      "npm/test/uninstall.test.mjs"
    ]
  },
  {
    "acceptance_id": "acceptance-24b063ad6c75bbc4673cf18a242a5fac916ed08413da4c5e059fb181bc48c477",
    "evidence_refs": [
      "npm/test/self-remove.test.mjs",
      "npm/test/uninstall.test.mjs"
    ]
  },
  {
    "acceptance_id": "acceptance-33a17aaf94061fd84e401fd342178969811c1610cef73fb6edc5c6ae3fff517e",
    "evidence_refs": [
      "npm/scripts/test-install.mjs",
      "npm/test/uninstall.test.mjs"
    ]
  },
  {
    "acceptance_id": "acceptance-374771d57c372059bfc8c0c7c34104b5db40e1d0a95f2214234851e20fd3d730",
    "evidence_refs": [
      "npm/test/uninstall.test.mjs"
    ]
  },
  {
    "acceptance_id": "acceptance-7213c7eedc927239c2d3dc5778f5b00a07cf6ef41deaa0732b754aa40bd20961",
    "evidence_refs": [
      "npm/test/uninstall.test.mjs"
    ]
  },
  {
    "acceptance_id": "acceptance-7480038ce474b2a9781a4010d6eeed6c0c1090e6c751ec6ee2c3c896de218a86",
    "evidence_refs": [
      "npm/scripts/test-install.mjs",
      "npm/test/uninstall.test.mjs"
    ]
  },
  {
    "acceptance_id": "acceptance-80191a5df88c288f8ef2e92ab6b577951a72cd57c9fa084e499d00d051578c34",
    "evidence_refs": [
      "npm/test/uninstall.test.mjs"
    ]
  },
  {
    "acceptance_id": "acceptance-960cf3a99bc850b600c9874aae34ece674894305af963bfe568034167ff18459",
    "evidence_refs": [
      "npm/scripts/test-install.mjs",
      "npm/test/uninstall.test.mjs"
    ]
  },
  {
    "acceptance_id": "acceptance-a3a82b50f334cb9146288a1512b422106735c4602cfd40f6834478af37d1997b",
    "evidence_refs": [
      "npm/test/uninstall.test.mjs"
    ]
  },
  {
    "acceptance_id": "acceptance-bbb672ca489023e157950dc3ff43329d383196567c0a853f821fe847bbac381a",
    "evidence_refs": [
      "npm/scripts/test-install.mjs",
      "npm/test/self-remove.test.mjs"
    ]
  },
  {
    "acceptance_id": "acceptance-d320216f82f4faa990ae887c28e68024dade151f75431b12d7a64bf6bb8a3551",
    "evidence_refs": [
      "docs/verification/tests/test_broker_registration.py",
      "npm/test/uninstall.test.mjs"
    ]
  },
  {
    "acceptance_id": "acceptance-d610973de18d1592fb5af0b42940ed85725a0e3c76fafa812a9ab7244c31cc06",
    "evidence_refs": [
      "npm/test/uninstall.test.mjs"
    ]
  },
  {
    "acceptance_id": "acceptance-eae48b297a00e9ddfa6a0372c01cccf8cac2d82660c536728f4bfd5dd4cfa453",
    "evidence_refs": [
      "npm/scripts/test-install.mjs",
      "npm/test/uninstall.test.mjs"
    ]
  },
  {
    "acceptance_id": "acceptance-eecacb0ba20117979196adf3f7ca1e3099b515aeb478e9f6ef525e47fb104739",
    "evidence_refs": [
      "npm/test/uninstall.test.mjs"
    ]
  },
  {
    "acceptance_id": "acceptance-fc3f12405aa07de4880873f7ca13c4ad42addbff75130849b4442efa007f6ada",
    "evidence_refs": [
      "npm/scripts/test-install.mjs",
      "npm/test/uninstall.test.mjs"
    ]
  }
]
<!-- comet-native:acceptance-evidence:end -->
