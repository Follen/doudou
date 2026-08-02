# Outcome

`doudou uninstall` 一条命令同时清理 Doudou 安装的 Skill、用户级共享 Python venv/runtime 和 npm 包自身；任何调用项目中的 Profile 和消息数据保持原样。

# Scope

- 为 npm 安装版增加 `doudou uninstall` 受控清理入口。
- 删除带有有效 Doudou 管理标记的 Skill。
- 删除 Doudou 用户级共享 `runtimes/`，包括当前 venv、版本化 venv、指针和 setup 中间体。
- 补充卸载单元测试、打包后真实安装/卸载回归和用户文档。
- 明确 Windows 与 Linux 的清理路径和结果输出。

# Non-goals

- 不删除调用项目中的 `profile/`、`message/` 或其他业务文件。
- 不删除未知、无 Doudou 管理标记或属于其他包的 Skill 目录。
- 不在本 change 中改变业务命令、浏览器交互或消息协议。

# Acceptance examples

- 已安装默认 Skill 且已创建共享 venv 时，执行 `doudou uninstall` 后，npm 包、该 Skill 和 Doudou `runtimes/` 均不存在，项目 `profile/` 与 `message/` 内容和哈希不变。
- Skill 目标目录没有有效 Doudou npm 管理标记时，卸载保留该目录并输出明确提示，不把未知目录当作本包文件删除。
- venv/runtime 不存在时，卸载仍成功且结果可重复，不因缺失目录报错。

# Constraints and invariants

- 删除操作必须从受信任的固定根和结构化管理标记解析路径，验证边界后再执行。
- 生命周期脚本不得扫描项目目录，也不得根据当前工作目录推断或删除 `profile/`、`message/`。
- npm 包仍不携带 venv、浏览器或项目数据。
- 已发布的 `0.0.1` 不覆盖；实现发布时使用新的 SemVer 版本。
- npm 11 的真实全局卸载回归证明，被卸载包的 `preuninstall` 不会执行：npm 包和启动器被直接删除，包内清理代码没有运行机会。因此不能把完整清理绑定到 `npm uninstall -g` 本身。

# Decisions

- 用户明确要求 npm 卸载同时删除 Skill 与 venv，项目数据保留。
- “项目数据”包括每个调用项目下的 `profile/` 和 `message/`。
- 用户选择删除所有由 npm 安装器登记的 Skill 路径，包括默认路径、`DOUDOU_AGENT_HOME` 和 `--skill-dir` 目标；每个目标必须仍有有效 Doudou 管理标记，未知目录保留。
- 用户选择保留 venv 之外的 Camoufox 浏览器缓存，避免影响其他 Camoufox 工具；完整卸载仅清理 Doudou 自有 Skill 与 `runtimes/`。
- 用户选择卸载时自动停止从 Doudou 管理 runtime 启动的 broker，确认退出后再删除 venv；不得停止其他 Python、浏览器或 Camoufox 进程。
- 用户选择 `doudou uninstall` 作为单命令完整卸载入口。
- 用户选择清理失败时中止自卸载并保留启动器，输出未清理对象和稳定错误；不得继续形成半卸载状态。
- 用户已确认当前完整范围并同意进入 Build。
- Build 真实回归推翻了“正常 npm 卸载会调用包内 lifecycle”的环境假设；该限制已纳入最终命令契约。
- 用户确认使用 `doudou uninstall` 作为唯一完整卸载入口：先清理受管资源，成功后自动调用 npm 删除当前包；Skill 中只保留简短说明。

# Open questions

无。

# Verification expectations

- Node 单元测试覆盖管理标记校验、默认/自定义路径、缺失路径、未知目录和平台缓存根。
- 打包安装回归在隔离 HOME/缓存和隔离项目数据目录中执行真实 `npm install -g` 与 `doudou uninstall`。
- 回归逐字节确认 `profile/`、`message/` 未变化，并确认 Skill 与共享 `runtimes/` 已删除。
- Windows 与 Linux GitHub Actions 全部通过后才允许发布。
