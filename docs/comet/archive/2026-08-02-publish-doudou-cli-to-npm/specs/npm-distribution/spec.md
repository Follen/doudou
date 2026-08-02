# npm-distribution

## npm CLI 交付

系统必须发布 npm package `@follenfang/doudou`，并在 `package.json.bin` 中暴露不带作用域的 `doudou` 命令。npm 代码只能负责 Skill 安装/升级、Python 环境发现和进程启动；不得复制 Python CLI 的业务逻辑、浏览器管理、协议解析、写操作或存储语义。

npm package 必须同时支持全局安装与 npx。Windows 必须可通过 npm 自动生成的 cmd 和 PowerShell shim 传递完整参数、stdout/stderr 和子进程退出码；Linux 必须可通过 shebang 入口完成同样语义。

### 场景：全局安装后运行

给定用户通过 npm 全局安装发布包，当其在任意项目运行 `doudou doctor` 时，命令先确保指定版本 Skill 可用，再从该 Skill 启动 Python CLI；当前工作目录保持为用户项目。

### 场景：npx 与禁用 lifecycle scripts

给定 npm/npx 安装阶段使用 `--ignore-scripts`，当首次调用 `doudou` 时，启动器必须在前台 ensure Skill 并继续原命令，不得依赖 `postinstall` 已经执行。

## Skill 安装与原子升级

默认 Skill 目标必须是 `~/.agents/skills/doudou`。`DOUDOU_AGENT_HOME` 覆盖 Agent home 时，目标必须是 `<DOUDOU_AGENT_HOME>/skills/doudou`；`--skill-dir <path>` 必须把完整 Skill 目标覆盖为 `<path>`，并优先于环境变量。这两个 npm 启动器选项必须在转发 Python 之前被移除，不得泄漏到 Python 业务命令。

发布包必须携带完整、已校验的 `doudou` Skill 交付树和包版本 manifest。每次 npm CLI 启动必须先使用版本与内容 manifest 做轻量 ensure：目标已匹配时不复制；缺失或版本/内容不匹配时，先将新 Skill 复制到同文件系统临时目录，验证 `SKILL.md`、manifest 和必需入口，再使用原子重命名替换。

替换失败时必须恢复旧 Skill，不得留下指向半成品的目标路径。启动器只能替换自己管理且 manifest 身份匹配的 `doudou` Skill；目标存在但不能证明为可管理 doudou Skill 时必须停止并给出冲突路径。

### 场景：轻量 ensure

给定已安装 Skill 的 manifest 与 npm 包完全匹配，当用户连续运行多条 doudou 命令时，启动器不重新复制 Skill、不创建 venv、不下载浏览器且不启动额外浏览器。

### 场景：升级失败

给定旧 Skill 可用，当新 Skill 复制、验证或原子切换失败时，目标路径仍指向旧 Skill，启动器返回稳定错误并不执行原 Python 业务命令。

## Python 运行时 bootstrap

npm 启动器必须发现满足 `>=3.10,<4.0` 的系统 Python，Windows 按稳定顺序尝试 `py -3`、`python3`、`python`，Linux 按稳定顺序尝试 `python3`、`python`，并通过真实版本探测而非命令名判定。无匹配 Python 时必须返回可操作错误和非零退出码，不创建部分 venv。

当用户级 doudou 运行时不存在或版本不匹配时，启动器必须先调用 Skill Python CLI 的 `setup`；setup 成功后再用原始工作目录、环境和参数执行用户原命令。setup 失败时必须透传输出和退出码，不执行原命令。用户显式运行 `setup` 时不得先递归调用另一次 setup。

### 场景：首次业务命令

给定 Skill 已 ensure 但用户级 venv/Camoufox 不存在，当用户直接运行业务命令时，启动器先完成 setup 的临时安装、真实启动验证和原子切换，成功后恢复且只执行一次原命令。

## 数据与包边界

npm package 必须使用 `package.json.files` 白名单。白名单只能包含 npm 启动/安装代码、包 manifest、必需授权/说明和完整 Skill 交付树。包内不得出现 `profile/`、`message/`、`analyze/`、`browser-data/`、Cookie、Storage、登录态、消息、诊断、测试中间体、venv、Python packages、Camoufox/Firefox 二进制或依赖缓存。

npm 启动器必须保留用户当前工作目录。Python CLI 仍把该目录作为默认 data root，只在当前项目创建 `profile/` 和 `message/`；Skill 目录、npm 目录和用户级运行时不得接收业务数据。

### 场景：pack 白名单

给定仓库内存在开发文档、测试和本地业务数据，当运行 `npm pack --dry-run` 与检查实际 tarball 时，包文件必须与白名单完全一致，凭据、Profile、消息、缓存、浏览器二进制与研发中间体扫描为空。

## Git tag 发布

私有 GitHub 仓库必须在 `v*` Git tag push 事件上运行 npm 发布 workflow。Workflow 必须使用最小权限 `contents: read` 和 `id-token: write`，先校验去除 `v` 前缀的 tag、`package.json.version` 与 Skill 版本完全一致，再执行 Windows/Linux 测试、npm 测试、pack inventory 和凭据门禁，最后使用 npm Trusted Publishing/OIDC 发布。仓库、workflow 和文档不得要求长期 `NPM_TOKEN`。首个发布 tag 必须是当前 Skill 版本对应的 `v2.1.1`。仓库保持 private 时，workflow 不得强制 npm 仅支持公开仓库的 provenance；将来仓库公开后可恢复自动 provenance。

首版发布如果 npm 平台要求先存在 package，可以由维护者在本机完成一次手工发布，随后立即在 npm 绑定该 GitHub 仓库和 workflow 为 Trusted Publisher。工作流在绑定完成前必须明确失败，不得回退读取 token。

### 场景：tag 发布

给定 npm Trusted Publisher 已绑定，当 `v2.1.1` 被 push 到 GitHub 时，workflow 从该 tag checkout，验证 tag、`package.json.version` 与 Skill 版本一致，全部门禁通过后使用 OIDC 发布；任一门禁失败时不调用 npm publish。仓库为 private 时发布成功不依赖 provenance。
