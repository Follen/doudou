# Doudou 仓库约定

Doudou 是基于 Rust（7-crate workspace）+ 原生 Chrome CDP 的抖音 CLI 与自解释 Agent Skill。

## 核心约束

- **CDP 隐身边界**：严禁启用 `Runtime.enable` / `Console.enable`，脚本评估走隔离世界（`Page.createIsolatedWorld`），启动不带 `--enable-automation`。
- **两阶段写安全**：所有写操作先生成一次性计划，经用户确认后凭 `--confirm --plan-id <id>` 执行一次（`ConfirmationRequired -> Executing -> Sent | Failed | Unknown`）；遇到 `WRITE_RESULT_UNKNOWN` 先只读复查，禁止自动重试。
- **自解释 CLI**：`doudou/SKILL.md` 保持精简，命令/参数/错误码由 `doudou cli search` 与 `doudou cli schema` 实时提供，不单独维护冗余 CLI 文档。
- **存储边界**：Profile、Broker 与采集数据（`message/profiles/<name>/`）默认统一存在系统标准 AppData，可分别通过 `--home`（`DOUDOU_HOME`）和 `--data-root`（`DOUDOU_DATA_ROOT`）覆盖。
- **隐私卫生**：仓库与提交中严禁包含 Profile、消息记录、Cookie、登录态、浏览器数据或研发抓包文件。

## 提交前门禁

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release -p doudou-cli
```
