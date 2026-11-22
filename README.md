# Doudou

[English](#english) | [中文](#中文)

---

<a id="english"></a>

## English

Doudou is a distributable Agent Skill and Python CLI for Douyin. It uses a project-local Camoufox browser profile to read conversations, messages, images, system events, user videos, and comments via structured page protocols, and performs confirmed write actions through controlled UI interactions in a persistent broker session.

### Installation

Requires Node.js `>=18.18`. The npm package ships only the launcher and Skill source. Setup downloads and checksum-verifies a dedicated Python runtime without using your system Python, and never bundles Python, virtual environments, dependencies, or browser binaries inside the npm package.

```powershell
npm install -g @follenfang/doudou
doudou doctor
```

On the first run, `doudou` atomically installs the matching Skill into `~/.agents/skills/doudou/`. When a business command runs without a ready runtime, setup runs automatically: it downloads and verifies Python `3.12.13`, creates a versioned virtual environment under `${DOUDOU_HOME:-~/.doudou}`, installs locked dependencies, downloads and verifies Camoufox `152.0.4-beta.28`, and then continues the original command. You can also run setup ahead of time:

```powershell
doudou setup
doudou profile create --profile work
doudou profile login --profile work
```

To run without a global install, use `npx`. Even when npm lifecycle scripts are disabled, the first command installs the Skill automatically:

```powershell
npx --yes @follenfang/doudou -- doctor
npx --yes @follenfang/doudou -- profile list
```

Override the Skill directory per call with `--skill-dir <absolute-path>`, or set `DOUDOU_AGENT_HOME` to change the default location to `<home>/skills/doudou`. `DOUDOU_HOME` overrides only user-level Python, virtual environments, browser assets, and install state. Profiles and collected records always stay inside the current working project:

```text
profile/<profile>/
message/profiles/<profile>/
```

### How it works

- Each project and profile runs one serialized Python broker that exits after 20 minutes of idle time.
- Windows defaults to true headless mode (`windows_headless`). When login, SMS, slider, or passkey verification appears, the same broker closes the background context, opens a visible headed window on the same profile for human takeover, and returns to headless mode after completion. Linux uses Xvfb virtual headed mode.
- The first business command initializes the page and IM capture internally without a separate user warmup step.
- Conversations, messages, images, videos, and comments are read from structured HTTP and Protobuf responses in the page context.
- Text messages, replies, images, chat likes, and video comments, replies, and likes are submitted through one normal page action. Chat text, chat replies, and video comments submit via editor `Enter`, verified by network receipts and read-only protocol rechecks.
- Image downloads verify AES-GCM decryption first, then validate the ciphertext or plaintext MD5, plaintext byte length, and actual file format.
- Video lists are captured from protocol responses. Pagination uses controlled scrolling in a versioned Profile component instead of reusing expired signed URLs.
- Page read protocols classify failures by HTTP status, `Content-Type`, `Retry-After`, response size or hash, and JSON business status. Comment pagination persists checkpoints per page so runs can resume from a failed cursor.
- Endpoint paths and field aliases live in versioned protocol adapters, and selectors live in versioned UI adapters. Every write control is checked for stable semantics, local component structure, unique visible and enabled state, and geometry bounds before any click. Ambiguous or drifted controls return `PAGE_ADAPTER_DRIFTED` without guessing clicks. Schema changes return `PROTOCOL_SCHEMA_DRIFTED` instead of masquerading as login expiry.
- Every write command generates a preview plan first. Execution requires explicit confirmation with `--confirm --plan-id <id>`.
- Batch sends reuse one broker after a single confirmation, run strictly in series with at least 5 seconds between attempts, and mark remaining items `skipped` after the first failure.

### Common commands

```powershell
doudou conversations refresh --profile work --kind all
doudou messages collect --profile work --conversation "<conversation-name-or-id>"
doudou protocol send --profile work --recipient "<conversation-name-or-id>" --text "Hello"
doudou protocol send --profile work --confirm --plan-id <id>
doudou protocol send-batch --profile work --conversation-ids <ID1,ID2> --text "Hello"
doudou videos collect --profile work --douyin-id Rurutia.wow
doudou video collect --profile work --sec-uid <sec_uid> --aweme-id <video-id>
```

See [`doudou/SKILL.md`](doudou/SKILL.md) for full agent orchestration rules, [`doudou/references/cli.md`](doudou/references/cli.md) for CLI parameters, and [`doudou/references/layout.md`](doudou/references/layout.md) for storage layout.

### Upgrade and uninstall

```powershell
doudou update --check
doudou update
doudou uninstall
```

`doudou update` upgrades an existing global npm installation and atomically updates the Skill. If the runtime manifest has not changed, it reuses the active runtime. If the manifest changed, it validates the new runtime side-by-side before switching. In `npx` mode, `update` reports version status without creating a global install.

`doudou uninstall` stops managed brokers, removes all npm-managed Skill installations and `${DOUDOU_HOME:-~/.doudou}`, and removes the npm package itself. Project `profile/` and `message/` directories are preserved. Because npm 7+ does not run uninstall lifecycle scripts, running `npm uninstall` directly leaves those runtime files on disk.

### Troubleshooting

- `SKILL_INSTALL_FAILED`: the target Skill directory contains unrecognized files. Pass a different `--skill-dir` or inspect the directory first.
- `DEPENDENCY_INSTALL_FAILED`: locked dependency installation failed. The next `setup` builds a fresh staging runtime instead of reusing a partial virtual environment.
- `BROWSER_DOWNLOAD_STALLED`: browser download received no new bytes for 90 seconds. Re-running `setup` resumes from a valid `.part` file.
- `BROWSER_DOWNLOAD_CHECKSUM_MISMATCH`: downloaded browser asset failed SHA-256 verification and was not activated.
- `RUNTIME_SETUP_FAILED`: run `doudou doctor` to inspect setup ownership, partial downloads, caches, and the suggested repair command.
- Login, SMS, slider, or passkey challenges: run `doudou profile login --profile <name>` in your local desktop terminal so the broker opens a visible browser window on the same profile for takeover.

### Release

GitHub Actions publishes to npm only when a `v*` tag is pushed. The tag without the `v` prefix must match `package.json`, `package-lock.json`, `doudou/scripts/doudou/__init__.py`, and `npm/skill-manifest.json`. Publishing uses GitHub OIDC Trusted Publishing bound to `Follen/doudou` and `.github/workflows/publish-npm.yml` with provenance enabled and no long-lived `NPM_TOKEN`.

```powershell
git tag -a vX.Y.Z -m "Doudou X.Y.Z"
git push origin vX.Y.Z
```

### Development verification

```powershell
npm test
npm run check:manifest
npm run test:pack
$env:PYTHONPATH = "doudou\scripts"
python -m pytest docs\verification\tests -q
python -m ruff check doudou\scripts docs\verification\tests
```

The published tarball contains the npm launcher and full Skill source, and excludes virtual environments, Python packages, browser binaries, profiles, messages, and `analyze/` data.

---

<a id="中文"></a>

## 中文

Doudou 是可分发的 Agent Skill 与 Python CLI，用项目本地 Camoufox Profile 通过结构化页面协议读取抖音聊天、图片、系统事件、用户视频和评论，并通过常驻浏览器执行经确认的受控页面写操作。

### 安装

只要求 Node.js `>=18.18`。npm 包只携带启动器和 Skill 源码；setup 自行下载校验专用 Python，不使用用户系统 Python，也不把 Python、venv、依赖或浏览器打进 npm 包。

```powershell
npm install -g @follenfang/doudou
doudou doctor
```

`doudou` 首次运行会把版本匹配的 Skill 原子安装到 `~/.agents/skills/doudou/`。首次业务命令发现运行时缺失时，会自动执行 setup：下载校验专用 Python `3.12.13`，在 `${DOUDOU_HOME:-~/.doudou}` 创建版本化 venv、安装锁定依赖、受控下载并验证专用 Camoufox `152.0.4-beta.28`，然后继续原命令。也可以提前执行：

```powershell
doudou setup
doudou profile create --profile work
doudou profile login --profile work
```

不希望全局安装时可使用 `npx`；即使 npm lifecycle scripts 被禁用，首次命令仍会补齐 Skill：

```powershell
npx --yes @follenfang/doudou -- doctor
npx --yes @follenfang/doudou -- profile list
```

Skill 目录可用 `--skill-dir <完整路径>` 临时覆盖，或用 `DOUDOU_AGENT_HOME` 把默认位置改为 `<home>/skills/doudou`。`DOUDOU_HOME` 只覆盖用户级 Python、venv、浏览器和安装状态的位置；Profile 与业务记录始终留在运行命令时的当前项目：

```text
profile/<profile>/
message/profiles/<profile>/
```

### 工作方式

- 每个项目和 Profile 只有一个 Python broker，串行执行命令，空闲 20 分钟自动退出。
- Windows 默认真无头（`windows_headless`）；登录、短信、滑块或 Passkey 等人工认证出现时，同一 broker 关闭后台 context，切换同一 Profile 到可见有头窗口供人工接管，完成后恢复真无头。Linux 保留 Xvfb 虚拟有头。
- 首条业务命令内部初始化页面与 IM 捕获，没有用户侧单独预热流程。
- 会话、消息、图片、视频和评论读取使用页面上下文中的结构化 HTTP 与 Protobuf 协议。
- 文字、引用回复、图片、聊天点赞、视频评论/回复/点赞通过一次正常页面动作提交；聊天文字/引用和视频评论/回复使用编辑器 `Enter`，由网络回执验证写结果，并由只读协议补充复查。
- 图片下载先校验 AES-GCM 解密，再按平台字段口径校验加密体或明文 MD5、明文大小与实际文件格式。
- 视频列表从协议响应读取；翻页由版本化 Profile 组件的受控滚动触发，不复用过期签名 URL。
- 页面读协议统一按 HTTP 状态、Content-Type、Retry-After、响应长度或哈希和 JSON 业务状态分类；评论分页逐页保存检查点，可从失败游标恢复。
- 接口路径与字段别名集中在版本化 Protocol Adapter，页面 selector 集中在版本化 UI Adapter。写控件依次校验稳定语义、局部组件结构、唯一可见与启用状态以及几何边界；不唯一或漂移时在点击前返回 `PAGE_ADAPTER_DRIFTED`，不猜测点击。接口结构变化返回 `PROTOCOL_SCHEMA_DRIFTED`，不伪装成登录失效。
- 写操作先生成预览计划，经明确确认后使用 `--confirm --plan-id <id>` 执行。
- 批量发送一次确认后复用同一 broker，严格串行，相邻尝试至少间隔 5 秒；首个失败后其余项目标记为 `skipped`，不会继续写入。

### 常用命令

```powershell
doudou conversations refresh --profile work --kind all
doudou messages collect --profile work --conversation "<会话名称或ID>"
doudou protocol send --profile work --recipient "<会话名称或ID>" --text "你好"
doudou protocol send --profile work --confirm --plan-id <id>
doudou protocol send-batch --profile work --conversation-ids <ID1,ID2> --text "你好"
doudou videos collect --profile work --douyin-id Rurutia.wow
doudou video collect --profile work --sec-uid <sec_uid> --aweme-id <视频ID>
```

完整 Agent 编排见 [`doudou/SKILL.md`](doudou/SKILL.md)，命令参数见 [`doudou/references/cli.md`](doudou/references/cli.md)，存储结构见 [`doudou/references/layout.md`](doudou/references/layout.md)。

### 升级与卸载

```powershell
doudou update --check
doudou update
doudou uninstall
```

`doudou update` 只升级已有全局 npm 安装并原子更新 Skill；runtime manifest 没变化时复用当前 runtime，变化时并排验证新 runtime 后切换。`npx` 模式只报告版本状态，不创建全局安装。

`doudou uninstall` 会停止受管 broker，删除全部 npm 管理的 Skill、整个 `${DOUDOU_HOME:-~/.doudou}`，再删除 npm 包自身。项目中的 `profile/`、`message/` 会保留。npm 7+ 不执行卸载 lifecycle，直接使用 `npm uninstall` 不会清理这些运行文件。

### 故障诊断

- `SKILL_INSTALL_FAILED`：目标 Skill 目录存在未知内容；换用 `--skill-dir`，或先人工核对该目录。
- `DEPENDENCY_INSTALL_FAILED`：锁定依赖安装失败；再次 setup 会创建新的临时 runtime，不复用半安装 venv。
- `BROWSER_DOWNLOAD_STALLED`：浏览器下载 90 秒无新字节；网络恢复后再次 setup 会续传有效 `.part`。
- `BROWSER_DOWNLOAD_CHECKSUM_MISMATCH`：浏览器资产 SHA-256 哈希不符，损坏文件不会激活。
- `RUNTIME_SETUP_FAILED`：运行 `doudou doctor` 查看 setup owner、下载 partial、缓存和可执行修复命令。
- 登录、短信、滑块或 Passkey：在本机桌面终端运行 `doudou profile login --profile <name>`，由同一 broker 关闭后台 context，再用同一 Profile 显示有头浏览器供人工接管。

### 发布

GitHub Actions 只在 `v*` tag 推送时发布。tag 去掉 `v` 后必须与 `package.json`、`package-lock.json`、`doudou/scripts/doudou/__init__.py` 和 `npm/skill-manifest.json` 版本完全一致。发布通过绑定 `Follen/doudou` 与 `.github/workflows/publish-npm.yml` 的 GitHub OIDC Trusted Publishing 完成（公开仓库自动携带 provenance，不使用长期 `NPM_TOKEN`）。

```powershell
git tag -a vX.Y.Z -m "Doudou X.Y.Z"
git push origin vX.Y.Z
```

### 开发验证

```powershell
npm test
npm run check:manifest
npm run test:pack
$env:PYTHONPATH = "doudou\scripts"
python -m pytest docs\verification\tests -q
python -m ruff check doudou\scripts docs\verification\tests
```

发布 tarball 包含 npm 启动器和完整 Skill 源码，但不包含 venv、Python packages、浏览器二进制、Profile、消息或 `analyze/` 数据。
