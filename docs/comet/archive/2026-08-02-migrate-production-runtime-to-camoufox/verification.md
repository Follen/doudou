# Acceptance evidence

<!-- comet-native:acceptance-evidence:start -->
[
  {
    "acceptance_id": "acceptance-01be6892e728a3c585f30013b0c3ca37bf5c50f4ad47a62c0a5be015b7917495",
    "evidence_refs": [
      "docs/verification/camoufox-migration-20260802.md",
      "doudou/runtime-manifest.json",
      "doudou/src/doudou/runtime/environment.py"
    ]
  },
  {
    "acceptance_id": "acceptance-04a022259fb773c560c94f36487b4eb7dc66ca1f0414e85e2733e49240deb1d5",
    "evidence_refs": [
      "docs/verification/camoufox-migration-20260802.md",
      "doudou/src/doudou/runtime/broker/server.py",
      "doudou/tests/test_broker.py"
    ]
  },
  {
    "acceptance_id": "acceptance-0661fae1dc656a5330a25ead13aa63f4e80ecbc2d7b15acc77e64798a31f38ea",
    "evidence_refs": [
      "docs/verification/camoufox-migration-20260802.md",
      "doudou/src/doudou/runtime/browser/session.py",
      "doudou/src/doudou/runtime/profile.py"
    ]
  },
  {
    "acceptance_id": "acceptance-52abe96a90f2164dd495e571b0360bfbf26ee687ddec1a0f51b3d5924d3b139d",
    "evidence_refs": [
      "docs/verification/camoufox-migration-20260802.md",
      "doudou/src/doudou/features/video/service.py",
      "doudou/tests/test_video_writes.py"
    ]
  },
  {
    "acceptance_id": "acceptance-530f3f6aa6eec07cf0dd089a96a66f9f28b3b3247b7c65ad4c5099be2352f415",
    "evidence_refs": [
      "docs/verification/camoufox-migration-20260802.md",
      "doudou/src/doudou/features/chat/service.py",
      "doudou/src/doudou/ui/components/chat.py"
    ]
  },
  {
    "acceptance_id": "acceptance-799f625af9c06f7b4242acfbb63b8aeb47fdbcf85ae20840fa6cc64f998c74ad",
    "evidence_refs": [
      "docs/verification/camoufox-migration-20260802.md",
      "doudou/src/doudou/runtime/paths.py",
      "doudou/tests/test_profile.py"
    ]
  },
  {
    "acceptance_id": "acceptance-ac100d3e335b9d4dfc9e08fd9eb7c0c7c488bb463df0d89d42dec9b5fda31d98",
    "evidence_refs": [
      "docs/verification/camoufox-migration-20260802.md",
      "doudou/src/doudou/features/chat/service.py",
      "doudou/tests/test_batch.py"
    ]
  },
  {
    "acceptance_id": "acceptance-ba9da21feb6c122494f56c5de642ede9e2e2ef8d343f6190e4bdc8f741eea586",
    "evidence_refs": [
      "docs/verification/camoufox-migration-20260802.md",
      "doudou/src/doudou/reads/video.py",
      "doudou/src/doudou/ui/adapters.py"
    ]
  },
  {
    "acceptance_id": "acceptance-bbf7228fb96f6afdc32a421b707c71f208ae11d48e828649da4af78571c51c4a",
    "evidence_refs": [
      "docs/verification/camoufox-migration-20260802.md",
      "doudou/src/doudou/runtime/platform/windows.py",
      "doudou/tests/test_windows_process.py"
    ]
  },
  {
    "acceptance_id": "acceptance-c8e019690dfdbacdc50892912860b904a338bda77f5271416dadab8180b2a550",
    "evidence_refs": [
      "docs/verification/camoufox-migration-20260802.md",
      "doudou/src/doudou/protocol/im.py",
      "doudou/tests/test_protocol_storage.py"
    ]
  },
  {
    "acceptance_id": "acceptance-ced90b49dffb60f74c28b62a803ce46316ac52577cc0245998cf6fcc0c6f8778",
    "evidence_refs": [
      "docs/verification/camoufox-migration-20260802.md",
      "doudou/src/doudou/reads/chat.py",
      "doudou/src/doudou/storage/media.py",
      "doudou/tests/test_protocol_storage.py"
    ]
  },
  {
    "acceptance_id": "acceptance-e01285efec2c532f541e805168b4d2dd594d360fccbec0e7a19a20296da5f63b",
    "evidence_refs": [
      "docs/verification/camoufox-migration-20260802.md",
      "doudou/SKILL.md",
      "doudou/src/doudou/runtime/paths.py"
    ]
  },
  {
    "acceptance_id": "acceptance-e1b7346c74cb284ade9490771403ea9199a2dc6a12b06510563b54e513b8dc93",
    "evidence_refs": [
      "docs/verification/camoufox-migration-20260802.md",
      "doudou/src/doudou/runtime/browser/session.py",
      "doudou/tests/test_broker.py"
    ]
  }
]
<!-- comet-native:acceptance-evidence:end -->

# Commands and results

- `python -m pytest -q`：70 passed，退出 0。
- `python -m ruff check src tests`：All checks passed，退出 0。
- `python -m pip wheel . --no-deps`：wheel 构建成功，退出 0。
- 仓库外项目的 `doctor -> setup -> doctor -> profile create -> profile list`：全部退出 0。
- 真实群聊、私聊、图片、系统事件、身份、视频列表、详情、封面、评论与回复读取：全部成功。
- 授权目标上的八类写操作均只执行一次，并通过网络回执和只读协议复查。
- 源 Skill 与安装 Skill 的 76 文件清单逐项相同；凭据扫描 245 文件、0 命中。

# Skipped checks

无。Linux Xvfb 行为由平台分支和单元契约验证；当前实际主机的真实窗口验收为 Windows。

# Spec consistency

实现与当前 brief、完整拟议规格一致。生产运行时仅保留 Python/Camoufox；读取使用结构化协议，写入使用确认 plan 后的一次正常页面动作；Profile、message 和研发证据边界保持隔离。

# Known limitations and risks

页面和协议仍可能随上游版本变化；版本化 Protocol/UI/Component Adapter 会分别返回稳定漂移错误。线上写操作遇到未知结果不会自动重放。

# Conclusion

通过。全部验收项均有正式代码、自动化测试和脱敏真实回归证据支持，可以进入 Archive。
