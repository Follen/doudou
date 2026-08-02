# Outcome

将 doudou 交付为可通过 npm 全局安装或 npx 运行的 `doudou` 命令。npm 包只负责安装/升级 Skill 和启动 Python CLI；现有 Python/Camoufox 仍是真实运行时。

# Scope

- 新增 npm package、`bin` 启动器、Skill 原子安装/升级器与轻量 `ensure` 流程。
- 默认安装到 `~/.agents/skills/doudou`；支持 `--skill-dir` 与 `DOUDOU_AGENT_HOME` 覆盖。
- 首次命令自动检查 Python，并通过现有 setup 在用户缓存创建锁定 venv、安装 Python 依赖、下载 Camoufox、真实启动验证后原子切换。
- 支持 `npm install -g`、`npx`、Windows npm cmd/PowerShell shim 和 Linux。
- 新增 Git tag 触发的 GitHub Actions，使用 npm Trusted Publishing/OIDC。
- 更新 README 的安装、首次 setup、升级、卸载、数据位置和故障诊断。

# Non-goals

- 不用 JavaScript 重写 Python 业务、broker、协议或 UI adapter。
- npm 包不携带 venv、Python packages、Camoufox 浏览器、Profile、Cookie、消息、`analyze/` 或测试中间体。
- 卸载 npm 包时不自动删除用户项目中的 `profile/`、`message/` 或用户级运行时。
- 不将 npm token 长期保存到仓库或 GitHub Secrets。

# Acceptance examples

- 用户全局安装后在任意项目执行 `doudou doctor`，启动器先把与 npm 版本匹配的 Skill 原子安装到用户 Skill 目录，再使用系统 Python 启动 Skill CLI。
- 用户通过 npx 首次运行业务命令，即使 lifecycle scripts 被禁用，CLI 也会先补齐 Skill，再完成 venv/Camoufox setup 后恢复原命令。
- npm 版本升级后，下一次 `doudou` 命令在临时目录验证新 Skill，成功后原子替换；失败时保留旧 Skill 可用。
- 从任意项目运行后，业务数据仍只出现在当前项目的 `profile/` 和 `message/`，不出现在 npm 或 Skill 安装目录。
- `npm pack --dry-run` 和实际 tarball inventory 只包含白名单启动器、manifest 与 Skill 交付树，凭据扫描为空。
- Git tag `v2.1.1` 作为首个发布 tag 触发 Actions，Windows/Linux 测试、pack 和安装回归全部通过后，通过 OIDC 发布 npm；私有 GitHub 仓库按 npm 平台限制不生成 provenance。

# Constraints and invariants

- npm 启动器每次运行只做版本/manifest 的轻量 ensure，已匹配时不重复复制 Skill、不启动浏览器、不重建 venv。
- `postinstall` 只能 best effort；所有正确性由每次 CLI 启动时的 ensure 保证。
- npm 包使用 `files` 白名单，实际 tarball inventory 是发布门禁。
- Windows 必须通过 npm 生成的 `.cmd`/PowerShell shim 运行，不额外弹黑框或修改当前工作目录。
- Python 版本不满足 `>=3.10,<4.0` 时，在安装 Skill 后返回稳定、可操作的环境错误，不部分创建 venv。

# Decisions

- npm package 名称为 `@follenfang/doudou`，通过 `bin` 对外提供不带作用域的 `doudou` 命令。
- npm 包是安装器/启动器，Python CLI 是唯一真实业务运行时。
- Skill 默认持久化到 `~/.agents/skills/doudou`；Profile 和消息仍在调用者当前项目。
- `DOUDOU_AGENT_HOME` 表示 Agent home，Skill 目标为 `<DOUDOU_AGENT_HOME>/skills/doudou`；`--skill-dir` 表示完整 Skill 目标路径并且优先级更高。
- npm 卸载不自动删除持久 Skill、用户级 Python/Camoufox 运行时或业务数据；README 提供显式清理命令。
- 发布 workflow 由 `v*` Git tag push 触发，使用 `contents: read` 和 `id-token: write`；首个发布 tag 为当前 Skill 版本 `v2.1.1`。
- 仓库保持 private；npm Trusted Publishing 继续使用 OIDC，但 npm 官方不为私有 GitHub 仓库生成 provenance，因此 workflow 不强制 `--provenance`。

# Open questions

无。用户已确认整体范围，并追加确认使用 tag 发布与首个 tag `v2.1.1`。

# Verification expectations

- Node 单元测试覆盖路径解析、manifest 对比、原子替换与回滚、Python 发现和参数透传。
- 现有 Python 全量测试、Ruff 和 Skill Creator 校验继续通过。
- 在 Windows 与 Linux 分别验证 npm 全局安装、npx、`--ignore-scripts`、首次 setup、升级、失败回滚、命令参数/退出码传递。
- 运行 `npm pack --dry-run` 与实际 tarball inventory，确认白名单和包大小；扫描凭据、Cookie、Profile、消息、缓存和浏览器二进制。
- 静态检查 GitHub Actions 权限、`v*` tag 触发、版本一致门禁、OIDC 与无 `NPM_TOKEN`；确认 private 仓库未错误强制 provenance。在实际 npm 发布前必须完成 Trusted Publisher 绑定或记录首版手工发布的外部前置。
