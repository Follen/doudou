# Outcome

交付可独立安装的 `douyin-message` Codex Skill 与确定性 Node.js CLI。Skill 安装在代理的 Skill 目录，但所有登录态、消息、发送回执与诊断数据均写入调用者当前项目；支持群聊和私聊的发现、消息拉取、普通协议发送、协议引用回复及页面契约诊断。

# Scope

- 提供标准 `SKILL.md`、`agents/openai.yaml`、锁定依赖的 Node.js CLI、参考文档与自动化测试。
- doctor 检测 Node.js >= 20、npm、精确 Playwright `1.62.0`、匹配的 Playwright-managed Chromium、浏览器实际启动能力和项目目录写权限。
- setup 在 Skill 安装目录安装 npm 依赖和匹配 Chromium，完成后允许恢复原任务。
- Profile 在用户当前项目的 `Profile/<profile>/` 中创建、登录、校准和复用；登录、校准、人工验证与协议抓包使用有头浏览器，其余任务默认无头。
- 无头任务重放同一 Profile 持久化的有头指纹契约，包括 UA/UA-CH、`navigator.webdriver`、Canvas、WebGL 及相关浏览器可见表面。
- 发现群聊和私聊，优先使用 Protobuf `conversation_type`，仅以有界、明确的 DOM 属性和头像结构作 fallback。
- 按对话与 RFC3339 时间范围拉取消息，规范化文本、图片、系统消息、稳定消息 ID、发送者与时间字段，并增量合并。
- 通过页面 IM SDK 生成的新鲜签名请求执行普通发送或引用回复；发送前生成确认计划，确认后验证 HTTP、服务端结果、client ID 回显和目标会话。
- 提供有头协议抓包和只读 page doctor，输出脱敏请求/响应、字段清单和契约漂移证据。
- 生产记录写入用户项目的 `message/profiles/<profile>/`，包括会话、Run、发送回执和可选诊断。
- CLI stdout 使用 NDJSON，Skill 根据机器可读事件呈现对话列表、进度、预览、错误和结果。

# Non-goals

- 不在交付 Skill 中包含或创建用户 Profile、消息记录、发送回执、抓包、`analyze/`、`node_modules/` 或浏览器二进制。
- 不把研发仓库的研究过程、验证中间体或开发者登录数据作为用户运行时目录。
- 不深遍历 React Fiber、React DOM 内部对象或任意页面对象图。
- 不从群名文本推断群聊/私聊类型，不把显示名称当作稳定身份。
- 不自动处理验证码，不在无明确确认时发送真实消息。
- 不承诺页面或协议变化后无需重新抓包、诊断或更新字段映射。

# Acceptance examples

- 在任意用户项目调用已安装 Skill 的 doctor 时，仅按需创建该项目的 `Profile/` 与 `message/` 根目录；Skill 安装目录保持无运行数据。
- 缺少 Playwright 或匹配 Chromium时，doctor 返回稳定 `setup_required` 诊断；setup 安装精确依赖后，doctor 能实际启动并关闭 Chromium。
- 新用户创建命名 Profile 后，有头登录状态和指纹基线保存在该用户项目；跨进程、重启和普通跨日的无头任务继续复用，不生成新的设备身份。
- 用户刷新全部会话时，结果同时包含群聊和私聊；`conversation_type=2` 为群聊，`conversation_type=1` 为私聊，标题中出现“群”字不改变类型。
- 用户按同一次刷新结果选择 `1,3-5` 或“全部”时，CLI 只解析该快照中的编号，并回显稳定 conversation ID。
- 用户对一个或多个群聊/私聊指定时间范围后，系统逐个拉取消息；单个对话失败不阻断其余只读拉取，最终 Run 记录新增、重复、冲突、跳过和失败数量。
- 用户发普通消息或引用回复时，第一次调用只返回精确预览和 plan ID；只有使用该 plan ID 明确确认后才发送，并生成二进制请求、响应、receipt 和 verification。
- 用户执行协议 capture 时，有头浏览器允许其完成一次真实页面发送；保存的 manifest 脱敏 URL query 与敏感 header，原始 body 只保存在用户项目诊断目录。
- 页面字段或 bundle 漂移时，page doctor 给出字段级新增、删除、改型与脱敏样本，不发送消息，也不自动改写正式映射。
- 打包 Skill 时，清单只包含 `agents/`、`bin/`、`references/`、`scripts/`、`src/`、`test/`、`.gitignore`、`npm-shrinkwrap.json`、`package.json` 和 `SKILL.md`。

# Constraints and invariants

- 默认 data root 是调用者的 `process.cwd()`；只有用户显式指定时才使用 `--data-root` 或 `DOUYIN_MESSAGE_HOME`。
- Skill 代码、依赖安装位置与用户运行数据严格分离；Profile 和 message 必须由用户项目的版本控制忽略。
- Profile 同时最多由一个浏览器上下文持有；锁冲突必须验证 owner，终止 owner 需要明确确认。
- 会产生外部消息的操作严格串行；目标会话和引用消息不能唯一确定时失败关闭。
- 指纹文件缺失、损坏、schema/Chromium 契约不兼容或探针不一致时，在访问目标页面前要求有头校准；自动 fallback 只对当前任务预授权一次。
- 协议发送必须基于页面当次生成的动态签名、Cookie、设备上下文和序列号，不能把读取 Protobuf 结构直接反向当作发送请求。
- 未知字段保留可审计 field inventory 与脱敏样本；关键字段未知或改型时不得静默猜测。
- 消息和会话 ID 使用字符串保存；JSON/JSONL 写入保持完整、带 schema，并对损坏或未知 schema 失败关闭。
- 日志、manifest、回执和诊断不得输出 Cookie、token、认证 query 或完整敏感 header。

# Decisions

- 最终交付目录和 Skill 名称为 `douyin-message`；现有 canonical capability ID `douyin-group-message` 保持稳定，由本 change 替换其完整目标规格。
- Skill 可安装在 `~/.agents/skills/douyin-message/` 等代理目录；用户在其他项目调用时，运行数据归属于该项目而非 Skill 安装目录。
- 用户生产目录固定为 `Profile/<profile>/` 与 `message/profiles/<profile>/`；`analyze/` 仅可作为本研发仓库的研究区，不属于分发物或用户运行契约。
- 生产消息目录按 conversation 而非仅按 group 组织，kind 为 `group` 或 `direct`，稳定 ID 负责身份隔离。
- 会话分类顺序为 Protobuf 类型优先、有界 DOM fallback；明确禁止 React/Fiber 深遍历。
- 接收与发送均通过 CLI；协议普通发送与引用回复是正式能力，网页 UI 发送只用于捕获新的协议契约。
- 所有发送采用两步确认计划，并以服务端回执验证，不依赖 DOM 可见文本声称成功。
- 自动化交付验证只做预览和 fixture/协议契约测试，不发送真实消息。
- 用户已确认以上完整 Shape 契约并同意进入 Build 核对现有实现。

# Open questions

- 无。

# Verification expectations

- 运行 Skill 标准校验、`npm ci`、完整 Node 测试和 CLI help 检查。
- 在隔离安装副本验证 doctor、setup、精确 Playwright/Chromium 版本和浏览器启动。
- 在空用户项目验证仅创建 `Profile/` 与 `message/`，且 Skill 内无运行数据、凭据、`analyze/` 或 `node_modules/`。
- 覆盖群聊/私聊分类、编号快照选择、消息字段规范化、增量合并、损坏数据、Profile 锁和指纹持久化测试。
- 覆盖普通协议发送与引用回复的请求改写、确认计划完整性、服务端回执验证、字段 inventory 和脱敏抓包测试。
- 验证无 React/Fiber 深遍历路径，DOM fallback 只搜索有界消息祖先与显式可见动作。
- 记录未执行的真实登录、线上拉取或真实发送；没有真实证据时不得报告为通过。
