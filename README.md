# Doudou

[English](#english) | [中文](#中文)

---

<a id="english"></a>

## English

Doudou is a Rust CLI and self-describing Agent Skill for Douyin. It drives your local Chrome / Chromium / Edge browser over native WebSocket CDP (without `Runtime.enable`) to read conversations, messages, images, system events, user videos, and comments via structured page protocols, and performs two-phase confirmed write actions through controlled UI interactions in a serialized per-profile broker.

### Requirements & Installation

- Local Chrome, Chromium, or Edge installed on the machine (auto-discovered, or override via `DOUDOU_CHROME_PATH`)
- **Via npm (recommended, Node.js `>= 18.18`)** — automatically installs the Agent Skill into `~/.agents/skills/doudou/` and downloads the SHA-256 verified native Rust binary for Windows x64, Linux x64, macOS arm64, or macOS x64 from GitHub Releases (zero runtime dependencies, no Python or Camoufox):

```bash
npm install -g @follenfang/doudou
doudou doctor --launch-probe
```

Or run without a global install via `npx`:

```bash
npx --yes @follenfang/doudou -- doctor
```

Upgrade or uninstall managed Skill + cached binaries (user profiles and collected messages in AppData are preserved):

```bash
doudou update --check
doudou update
doudou uninstall
```

- **Or build from source (Rust `>= 1.85`, edition 2024)**:

```bash
cargo install --path crates/doudou-cli
```

### Quick Start

```bash
# 1. Verify Chrome discovery, writable roots, and optional headless CDP probe
doudou doctor --launch-probe

# 2. Create a profile in OS AppData and sign in interactively
doudou profile create --profile work
doudou session login --profile work

# 3. Discover any command, flag, event, or error code directly from the binary
doudou cli search "send"
doudou cli schema protocol send
```

### Storage Boundary

Profiles, browser state, broker metadata, and collected records all default to standard OS AppData:

- **Windows**: `%LOCALAPPDATA%\doudou\`
- **macOS**: `~/Library/Application Support/doudou/`
- **Linux**: `${XDG_DATA_HOME:-~/.local/share}/doudou/`

Layout & overrides:

- **Home (`--home` / `DOUDOU_HOME`)**: stores `profiles/<name>/` and `brokers/`.
- **Data root (`--data-root` / `DOUDOU_DATA_ROOT`)**: defaults to `<doudou_home>`, storing `message/profiles/<name>/` (conversations, messages, decrypted images, videos, comments, write plans, receipts, and diagnostics). Pass `--data-root <dir>` whenever you want project-isolated output.

### How It Works

- **Self-describing CLI**: `doudou cli search <query>` and `doudou cli schema [command]` introspect the live `clap` command tree directly — no external markdown CLI reference to drift out of sync.
- **Stealth CDP transport**: Never enables `Runtime.enable` or `Console.enable`; evaluates scripts in an isolated world (`Page.createIsolatedWorld`), launches without `--enable-automation`, and strips `HeadlessChrome` user-agent leaks.
- **Per-profile single-flight broker**: Each profile runs one loopback TCP JSON-line broker (16 MiB frame limit, constant-time auth token, idle auto-exit) shared across working directories. When login or verification appears, the broker switches the same profile to a visible headed Chrome window for human takeover and returns to headless mode afterward.
- **Protocol-first reads**: Conversations, stranger message requests, messages, images (AES-256-GCM decrypted + MD5 verified), user identities (`/aweme/v1/web/search/sug/`), user video lists, and checkpointed comment/reply trees are captured from structured HTTP/Protobuf responses.
- **Two-phase write safety**: Every write command (`chat`/`protocol send`, `reply`, `send-batch`, `like`, `video comment`, `reply`, `like`) first generates a preview plan and executes once with `--confirm --plan-id <id>`. The strict `ConfirmationRequired -> Executing -> Sent | Failed | Unknown` state machine and RAII execution guards prevent crash-retry duplicate sends (`PLAN_ALREADY_USED`). On `WRITE_RESULT_UNKNOWN`, never auto-retry — run a read-only check first.
- **Drift-safe UI adapters**: Controls are verified for semantics, local component structure, visibility, enabled state, and geometry/occlusion before clicking; ambiguous or drifted controls fail closed with `PAGE_ADAPTER_DRIFTED`.

### Common Commands

```bash
doudou conversations refresh --profile work --kind all
doudou messages collect --profile work --conversation "<conversation-name-or-id>"
doudou chat send --profile work --recipient "<conversation-name-or-id>" --text "Hello"
doudou chat send --profile work --confirm --plan-id <plan-id>
doudou chat batch --profile work --conversation-ids <ID1,ID2> --text "Hello"
doudou chat like --profile work --conversation "<conversation-name-or-id>" --message <msg-id>
doudou identity resolve --profile work --douyin-id Rurutia.wow
doudou videos collect --profile work --douyin-id Rurutia.wow
doudou video collect --profile work --sec-uid <sec_uid> --aweme-id <aweme-id>
doudou page-doctor --profile work --conversation "<conversation-name-or-id>"
```

See [`doudou/SKILL.md`](doudou/SKILL.md) for the agent orchestration rules and [`PORTING.md`](PORTING.md) for the 7-crate workspace architecture and legacy Python → Rust mapping.

### Development Verification

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release -p doudou-cli
```

---

<a id="中文"></a>

## 中文

Doudou 是基于 Rust + 原生 Chrome CDP 的抖音 CLI 与自解释 Agent Skill。通过本机 Chrome / Chromium / Edge（原生 WebSocket CDP，禁止 `Runtime.enable`）读取抖音群聊、私聊、陌生人消息请求、图片、系统事件、用户视频与全量评论，并通过串行单飞行 broker 执行经确认的受控页面写操作。

### 环境要求与安装

- 本机已安装 Chrome、Chromium 或 Edge（自动探测，也可用 `DOUDOU_CHROME_PATH` 指定）
- **通过 npm 安装（推荐，需 Node.js `>= 18.18`）**——自动将精简 Agent Skill 安装到 `~/.agents/skills/doudou/`，并从 GitHub Releases 下载经 SHA-256 校验的原生 Rust 二进制（支持 Windows x64 / Linux x64 / macOS arm64 / macOS x64，零 npm 依赖，无需 Python 或 Camoufox）：

```bash
npm install -g @follenfang/doudou
doudou doctor --launch-probe
```

或使用 `npx` 免全局安装运行：

```bash
npx --yes @follenfang/doudou -- doctor
```

升级与卸载（保留 AppData 中的 Profile 与聊天/视频采集数据）：

```bash
doudou update --check
doudou update
doudou uninstall
```

- **或从源码编译安装（Rust `>= 1.85`，edition 2024）**：

```bash
cargo install --path crates/doudou-cli
```

### 快速开始

```bash
# 1. 检查本机 Chrome 探测、目录权限及无头 CDP 探针
doudou doctor --launch-probe

# 2. 在系统标准 AppData 创建 Profile 并扫码登录
doudou profile create --profile work
doudou session login --profile work

# 3. 直接向 CLI 检索命令、参数、事件和错误码（无需翻文档）
doudou cli search "send"
doudou cli schema protocol send
```

### 数据与目录边界

Profile、Chrome 用户数据、broker 状态以及采集记录默认统一存放在系统标准 AppData 目录：

- **Windows**：`%LOCALAPPDATA%\doudou\`
- **macOS**：`~/Library/Application Support/doudou/`
- **Linux**：`${XDG_DATA_HOME:-~/.local/share}/doudou/`

目录职责与覆盖方式：

- **Home（`--home` / `DOUDOU_HOME`）**：存放 `profiles/<name>/` 与 `brokers/`。
- **Data Root（`--data-root` / `DOUDOU_DATA_ROOT`）**：默认指向 `<doudou_home>`，存放 `message/profiles/<name>/`（会话、消息、解密图片、视频、评论、写计划、回执与诊断）；需要按项目单独存放时显式传入 `--data-root <目录>` 即可。

### 核心机制

- **自解释 CLI**：`doudou cli search <query>` 与 `doudou cli schema [command]` 直接从运行时的 `clap` 命令树实时生成帮助和 JSON Schema，单一事实源不漂移。
- **隐身 CDP 传输**：运行时强制禁止 `Runtime.enable` 与 `Console.enable`，通过 `Page.createIsolatedWorld` 隔离执行脚本，启动不带 `--enable-automation`，并剥离 `HeadlessChrome` UA 特征。
- **单飞行本机 Broker**：每个 Profile 维护一个 loopback TCP JSON-line broker（16 MiB 帧上限、常时间 token 校验、空闲自动退出），可跨项目目录复用；遇到登录、短信、滑块或 Passkey 验证时切换同一 Profile 到可见有头窗口人工接管，完成后恢复无头模式。
- **协议优先读取**：会话列表、陌生人消息请求、消息、图片（AES-256-GCM 解密 + MD5 校验）、抖音号解析（`/aweme/v1/web/search/sug/`）、作者视频列表及带断点的全量评论/子回复均通过结构化 HTTP/Protobuf 响应读取。
- **两阶段写安全**：所有写命令先生成一次性预览计划，经人工确认后带 `--confirm --plan-id <id>` 执行一次。严格单次状态机（`ConfirmationRequired -> Executing -> Sent | Failed | Unknown`）与 RAII 守卫确保崩溃或重试不会二次外发（`PLAN_ALREADY_USED`）；出现 `WRITE_RESULT_UNKNOWN` 时必须先只读复查，严禁自动重试。
- **防漂移页面组件**：每次写点击前校验稳定语义、组件结构、唯一可见启用状态与几何/遮挡边界，异常时直接返回 `PAGE_ADAPTER_DRIFTED`，不猜测点击。

### 常用命令

```bash
doudou conversations refresh --profile work --kind all
doudou messages collect --profile work --conversation "<会话名称或ID>"
doudou chat send --profile work --recipient "<会话名称或ID>" --text "你好"
doudou chat send --profile work --confirm --plan-id <plan-id>
doudou chat batch --profile work --conversation-ids <ID1,ID2> --text "你好"
doudou chat like --profile work --conversation "<会话名称或ID>" --message <msg-id>
doudou identity resolve --profile work --douyin-id Rurutia.wow
doudou videos collect --profile work --douyin-id Rurutia.wow
doudou video collect --profile work --sec-uid <sec_uid> --aweme-id <视频ID>
doudou page-doctor --profile work --conversation "<会话名称或ID>"
```

Agent 编排约束见 [`doudou/SKILL.md`](doudou/SKILL.md)，7-crate 架构与原 Python 版本映射见 [`PORTING.md`](PORTING.md)。

### 开发验证

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release -p doudou-cli
```
