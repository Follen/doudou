# Outcome

将 Douyin Message 的运行入口统一为 `https://www.douyin.com/chat`，以页面上下文的 IM 协议读取和发送替代首页消息面板流程。群聊、私聊、消息采集、普通发送和引用回复保持可用，并以服务端 Protobuf 回执而不是 DOM 文本确认写入结果。

# Scope

- 所有 headless 业务命令从 `/chat` 进入，不再导航首页或点击首页消息入口。
- warmup、会话发现和消息采集优先使用 `/chat` 已启动的 IM 协议请求；会话类型以 Protobuf `conversation_type` 为准。
- 普通发送和引用回复在 `/chat` 页面中取得本次 SDK 生成的动态请求上下文，再由页面上下文发出最终 Protobuf 请求。
- 发送产物保留请求/响应校验所需的最小证据，并对签名 URL、Cookie、令牌、消息正文和会话标识脱敏。
- 更新已安装 Skill 与项目交付目录，使说明和运行行为一致。

# Non-goals

- 不实现全局用户搜索、创建新私聊或绕过登录/验证。
- 不承诺直接调用未公开的页面 SDK 对象，也不遍历 React/Fiber 或任意 DOM 对象图。
- 当前 change 不承诺消除 SDK bootstrap 的最小编辑框动作；该动作只用于生成本次动态上下文，bootstrap 请求不得被送达服务端。

# Acceptance examples

- 对已登录 Profile 执行 `warmup` 时，最终页面路径为 `/chat`，且没有访问首页或首页消息入口。
- 刷新会话索引后，群聊和私聊由结构化 `conversation_type` 分类；名称只用于显示和匹配。
- 向已存在私聊或群聊发送普通文本时，最终请求由 `/chat` 页面上下文发出，服务端 HTTP 200、Protobuf 状态 0、会话和客户端消息 ID 校验均通过。
- 触发发送时 SDK bootstrap 请求被拦截取消，只有改写后的最终协议请求可到达服务端。
- 发送回执和诊断报告不含完整签名 URL、Cookie、令牌、原始消息正文或原始会话/消息 ID。
- 协议漂移或未捕获 bootstrap 时，命令失败并输出稳定错误码，不发送替代消息。

# Constraints and invariants

- 复用项目本地 Profile、已校准指纹和页面登录态；动态签名与请求上下文只在当前浏览器页内存中使用。
- 保持既有确认式发送流程：预览不写入，只有带确认计划的命令才允许网络写入。
- 读操作不得触发 `mark_read` 等附带写入的端点。
- `profile/`、`message/`、`analyze/` 均保持项目本地且被 Git 忽略。

# Decisions

- 入口固定为 `/chat`，不保留首页 fallback。
- 优先策略为协议读取和页面上下文直发；最小 SDK bootstrap 可保留，直到发现稳定且可验证的内部 signer。
- 服务端 Protobuf 回执是发送成功的唯一判据。
- 用户于 2026-07-31 确认按本 brief 和完整规格实施。

# Open questions

无。

# Verification expectations

- 对 warmup、会话刷新、消息采集、私聊普通发送、群聊普通发送和引用回复分别运行针对性测试。
- 记录 `/chat` 路径、拦截/直发顺序、HTTP 状态、Protobuf 状态和脱敏产物检查；不记录动态值。
- 运行现有 Node 测试与 `doctor`，并检查交付目录和安装 Skill 的同步结果。
