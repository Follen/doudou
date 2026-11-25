<comet-ambient-resume>
<!-- Managed by Comet. Edits inside this block may be replaced by comet init/update. -->
<!-- Contract: comet.resume_probe.v2 -->

## Comet Ambient Resume

在这个仓库中，开始处理需要改动或调查的任务前，如果可能存在活跃 Comet workflow，把当前用户请求传入只读探针：`comet resume-probe . --stdin --json`。

- 如果用户通过宿主明确调用任意 Comet Skill（例如 `@comet`、`/comet`、`@comet-native` 或 `/comet-hotfix`），显式调用优先于本恢复协议；不要运行 resume probe，直接进入被调用的 Skill。
- 只信任返回的 `workflow`、`skill` 和 `entrySource`；它们只由项目配置或无配置兼容回退决定。不得扫描或切换另一套 workflow。
- 如果 probe 返回 `auto_resume`，简短说明选中的 active change，并进入 `nextCommand` 指向的永久入口。不要把状态命令当作恢复入口直接推进。
- 如果 probe 返回 `ask_user`，只问一个简短问题并等待用户回复。
- 如果当前请求未明确调用 Comet Skill，且 probe 返回 `out_of_scope` 或 `none`，不要进入 Comet workflow。
- 如果配置或状态无效且没有 `nextCommand`，停止并报告原因；不要猜测另一个 workflow。
- 不能只因为存在 active change 就把无关任务挂到该 change。Native 的未提交改动由 Native 入口检查，不由探针自动归因。
</comet-ambient-resume>

# Doudou 项目约定

## 项目简介

Doudou 是基于 Rust + 原生 Chrome CDP 的抖音 CLI 与自解释 Agent Skill，使用 7-crate Cargo workspace 构建。

- 读取能力通过页面上下文中的结构化 HTTP / Protobuf 协议完成。
- 写入能力通过受控页面组件执行，采用两阶段单次写计划（`ConfirmationRequired -> Executing -> Sent | Failed | Unknown`），并使用网络回执与只读协议复查。
- CDP 传输层严禁启用 `Runtime.enable` 或 `Console.enable`，脚本评估在隔离世界（`Page.createIsolatedWorld`）完成。
- 仓库与 Skill 均不得包含 Profile、消息记录、Cookie、登录态、浏览器数据或研发抓包数据。

## 版本与 CI 门禁

- 正式版本遵循 SemVer，workspace 版本定义在根 `Cargo.toml` 的 `[workspace.package].version`，Git tag 固定使用 `vX.Y.Z`。
- CI workflow 位于 `.github/workflows/ci.yml`，在 `ubuntu-latest`、`windows-latest`、`macos-latest` 上执行格式化检查、Clippy、全量单测/集成测试、release 编译及 CLI 自省冒烟检查。
- 每次提交或发布前必须通过本地门禁：

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release -p doudou-cli
```

## 目录位置

仓库内受版本控制内容：

- `Cargo.toml`、`Cargo.lock`、`crates/`：7-crate Rust workspace（`doudou-core`、`doudou-protocol`、`doudou-storage`、`doudou-cdp`、`doudou-ui`、`doudou-runtime`、`doudou-cli`）。
- `doudou/`：精简后的自解释 Agent Skill（`SKILL.md` 与 `agents/openai.yaml`）；命令、参数、事件与错误码由 `doudou cli search` / `doudou cli schema` 实时输出。
- `PORTING.md`：7-crate 架构说明与原 Python 版本模块/测试映射。
- `docs/`：协议字段参考、Comet 规格归档与历史验证记录。
- `.github/workflows/`：GitHub Actions Rust CI workflow。
- `README.md`、`AGENTS.md`：双语使用说明与仓库约定。

本地运行数据与研发目录（默认由 `.gitignore` 忽略，不得进入 Git 远端）：

- `target/`：Cargo 构建产物。
- `message/profiles/<profile>/`：当前项目目录内按 Profile 隔离的会话、消息、媒体、计划、回执、内容采集和诊断记录。
- `analyze/`：研发抓包、协议实验和中间分析材料。
- `.agents/`、`.codex/`、`.worktree/`：本地 Agent 配置、验证 skill 与 Git worktree。

用户级系统数据位置（标准 OS AppData，可用 `--home` 或 `DOUDOU_HOME` 覆盖）：

- Windows：`%LOCALAPPDATA%\doudou\`
- macOS：`~/Library/Application Support/doudou/`
- Linux：`${XDG_DATA_HOME:-~/.local/share}/doudou/`
- Profile 与浏览器数据：`<doudou_home>/profiles/<profile>/`
- 全局 Broker 注册表：`<doudou_home>/brokers/`

业务命令的数据根默认是执行命令时的当前项目目录（可用 `--data-root` 覆盖），Profile 与浏览器状态始终保存在 `<doudou_home>/profiles/<profile>/`。
