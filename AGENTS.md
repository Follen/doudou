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

Doudou 是可分发的 Codex Skill 与 Python CLI。npm 包只负责安装、升级和启动 Skill；业务运行时由 Python、Camoufox 和项目本地数据目录组成。

- 读取能力通过页面上下文中的结构化协议完成。
- 写入能力通过受控页面组件执行，并使用网络回执和只读协议复查。
- npm 包和 Skill 均不得包含 Profile、消息记录、Cookie、登录态、浏览器、venv 或研发抓包数据。

## Tag 与版本标准

- 正式版本遵循 SemVer，Git tag 固定使用 `vX.Y.Z`，例如 `v0.0.1`。
- tag 去掉前缀 `v` 后，必须与以下位置完全一致：
  - `package.json` 的 `version`
  - `package-lock.json` 的根版本
  - `doudou/scripts/doudou/__init__.py` 的 `__version__`
  - `npm/skill-manifest.json` 的 `package_version` 和 `skill_version`
- 发布前运行 `npm run build:manifest`，再运行全部本地门禁。
- tag 只能从干净的 `main` 创建，并且该提交必须已经推送到 `origin/main`。
- 使用 annotated tag：`git tag -a vX.Y.Z -m "Doudou X.Y.Z"`，然后执行 `git push origin vX.Y.Z`。
- 未发布版本的失败 tag 可以在修复后删除并重建；npm 已发布的版本和对应 tag 不得移动、覆盖或复用。
- `latest` 只指向最新正式版；预发布版本使用独立 dist-tag，不得覆盖 `latest`。

## CI 与发布

发布 workflow 位于 `.github/workflows/publish-npm.yml`，只在推送 `v*` tag 时触发。

1. `check:release` 校验 tag、npm package 和 Python Skill 版本一致。
2. Windows 与 Linux matrix 分别运行 Node 测试、Skill manifest 校验、包内容与凭据检查、全局安装/npx/禁用 lifecycle scripts 安装回归、Python 测试和 Ruff。
3. 两个平台全部通过后，publish job 才能运行。
4. npm 发布使用 Trusted Publishing/OIDC，只授予 `contents: read` 和 `id-token: write`，仓库不得保存长期 `NPM_TOKEN`。
5. 发布完成后必须确认 GitHub Actions 全绿、npm `latest` 指向目标版本，并从 registry 实际执行一次全局安装和 npx。

本地发布门禁：

```powershell
npm test
npm run build:manifest
npm run check:manifest
npm run test:pack
npm run test:install
npm run check:release -- vX.Y.Z
$env:PYTHONPATH = "doudou\scripts"
python -m pytest docs\verification\tests -q
python -m ruff check doudou\scripts docs\verification\tests
python "$HOME\.codex\skills\.system\skill-creator\scripts\quick_validate.py" doudou
```

## 目录位置

仓库内可发布内容：

- `doudou/`：最终 Skill；包含 `SKILL.md`、Agent 元数据、参考文档、Python CLI 和锁定依赖。
- `npm/`：npm 安装器、启动器、Skill manifest、Node 测试及发布检查脚本。
- `docs/verification/tests/`：Python 自动化测试。
- `docs/comet/`：Comet change、归档和验证记录。
- `.github/workflows/`：GitHub Actions workflow。
- `package.json`、`package-lock.json`：npm 包元数据和锁文件。
- `README.md`：用户安装、使用、升级、诊断与发布说明。

仓库内本地数据与研发目录：

- `profile/<profile>/`：项目本地 Profile、浏览器数据、指纹配置、锁和 broker 状态。
- `message/profiles/<profile>/`：按 Profile 隔离的会话、消息、媒体、计划、回执、内容采集和诊断记录。
- `analyze/`：研发抓包、协议实验和中间分析材料。
- `.comet/`：本地 Comet workflow 状态。
- `.agents/`、`.codex/`：项目本地 Agent/Codex 配置。

`profile/`、`message/`、`analyze/`、缓存和本地工具状态不得进入 npm 包或 Git 远端。

用户级安装位置：

- Skill：默认 `~/.agents/skills/doudou/`；可用 `--skill-dir` 或 `DOUDOU_AGENT_HOME` 覆盖。
- Windows 运行时与 venv：`%LOCALAPPDATA%/doudou/runtimes/`。
- Linux 运行时与 venv：`${XDG_CACHE_HOME:-~/.cache}/doudou/runtimes/`。
- Camoufox 浏览器：Camoufox 自己的用户级缓存目录，不放入 Skill 或业务项目。

业务命令的数据根默认是执行命令时的当前项目目录。Skill 可以安装在用户目录，但 Profile 和消息记录始终创建在调用项目的 `profile/` 与 `message/` 下。
