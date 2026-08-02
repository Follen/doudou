# Outcome

Doudou 在 Windows 上不再创建透明的有头 Camoufox 窗口。正常登录态下，broker 以真正无头 Camoufox 常驻完成协议读取和页面写入；只有登录、短信验证码、快捷登录验证码、滑块、Passkey 或通用验证确实需要人工处理时，才用同一 Profile 启动用户可见的有头 Camoufox。人工处理完成后关闭可见实例并恢复真无头 broker。Linux 保留 Xvfb 虚拟有头作为日常后台运行方式。

# Scope

- Windows broker 默认模式从 `windows_hidden_headed` 改为真正 `headless`，删除透明窗口、任务栏隐藏、Alt+Tab 隐藏和 Win32 window style 方案及其生产调用。
- Linux broker 保留 Camoufox + Xvfb 虚拟有头模式。
- 用统一认证状态机识别二维码登录、手机短信、快捷登录短信、滑块、Passkey 和通用验证；认证组件必须继续遵循三级 Component Adapter 契约。
- Windows 认证接管通过“安全关闭当前无头 context -> 同一 `browser-data`/指纹配置启动可见有头 -> 验证完成并关闭 -> 重建无头 context”完成，不复制 Profile，不并行打开同一 Profile。
- 登录成功、验证消失、超时、异常、用户关闭窗口和业务失败都必须有明确状态与清理结果；认证仍可见时保留可见窗口供用户继续处理。
- 保留 broker 单实例、Profile 锁、命令串行、20 分钟空闲退出、异步认证重算和原任务参数恢复。
- 所有读取继续走页面上下文协议；所有写入继续走受控页面组件，不回退纯协议写。
- 聊天普通文本和引用回复、视频发表评论和回复评论保留编辑器聚焦确认后按 `Enter` 提交；图片发送和点赞使用各自已验证组件动作。
- 修复视频评论页面成功回执后同步全量分页导致外层 90 秒误报 `WRITE_RESULT_UNKNOWN`：页面业务成功回执先持久化为权威结果，协议复查改为有界、非阻塞补充，失败或取消不能覆盖已确认成功。
- 同步 CLI doctor、broker descriptor、Skill、README、manifest 和测试中的运行模式说明。
- 对现有真实 Profile 执行完整读写业务回归并保存脱敏证据。

# Non-goals

- 不重新引入群聊 @ 能力。
- 不改协议读取的数据模型、消息文件夹或 Profile/消息隔离布局。
- 不用纯协议请求替代页面正常写入，不通过写入探针触发验证码。
- 不在写结果未知时自动重放。
- 不把 Profile、Cookie、message 或 `analyze/` 放入 Git、Skill 或 npm 包。
- 不删除 Linux Xvfb 虚拟有头支持。

# Acceptance examples

1. Windows 有效登录 Profile 启动 broker 时，进程树中不存在可见或透明的 `MozillaWindowClass` 主窗口，descriptor 报告 `headless`；群聊/私聊读取和受控页面写均在同一真无头 context 完成。
2. Windows 无头页面检测到账户未登录或真实短信、滑块、Passkey/二维码组件时，当前命令返回结构化认证状态并显示同一 Profile 的可见有头浏览器；同一时刻没有另一个占用该 Profile 的 browser context。
3. 用户完成认证后，页面事件异步触发最新 generation 重算；可见实例自动关闭，Profile 记录为 valid，无头 context 重建，原只读任务最多恢复一次。已开始的写任务不自动重放。
4. 认证仍可见或用户尚未完成时，可见窗口保持供接管，CLI 不误报登录成功；用户手动关闭后返回稳定取消/仍需认证状态，Profile 不损坏。
5. Linux 日常 broker 继续使用 Xvfb 虚拟有头；认证时可呈现供用户处理，完成后回到虚拟显示。
6. 聊天文字/引用与视频评论/回复在唯一编辑器聚焦、正文匹配且发送状态成立后按一次 `Enter`；服务端只观察到一个写请求。
7. 页面发布评论返回业务成功和稳定评论 ID 后，receipt 与 plan 立即记录 `created`；后续协议复查即使超时、被取消、空响应或排名列表暂未包含新评论，也不得把结果降级为 unknown。
8. 真实回归覆盖群聊/私聊发现、今日消息、图片下载、进群事件、未知字段、身份补齐、视频/详情/评论/回复读取，以及文字、引用、图片、聊天点赞、视频评论、评论回复和评论点赞。每个写动作只执行一次并以网络回执/协议复查记录结果。

# Constraints and invariants

- Windows 无头与可见有头必须复用项目内同一 `profile/<name>/browser-data`、同一 Camoufox 版本和同一持久指纹配置，但不得同时打开该 Profile。
- 模式切换前必须完成 context 关闭并释放文件占用；切换失败保持可诊断状态，不创建临时 Profile，不删除现有登录态。
- `decision=KICK` 只分类为 `WRITE_VERIFICATION_REQUIRED`；没有真实认证组件时不显示浏览器、不自动制造评论或消息来触发验证。
- 登录状态、IM 捕获状态和写验证状态继续独立分类；IM capture 缢失不得降级为未登录。
- DOM 操作与页面状态判断继续执行语义候选、局部组件、状态与几何三级漏斗；不唯一时 fail closed 为 `PAGE_ADAPTER_DRIFTED`。
- 写操作 plan/confirm、单次动作、未知结果不重放和 batch 间隔至少 5 秒保持不变。
- Windows CLI、broker 和辅助进程不得弹黑色控制台窗口。

# Decisions

- Windows 正常业务使用真无头 Camoufox，不保留透明有头窗口作为 fallback。
- Linux Xvfb 虚拟有头保留。
- 人工认证采用同 Profile 串行重启到可见有头，完成后回到平台默认后台模式。
- 聊天与评论可用编辑器 `Enter` 提交的能力继续使用 `Enter`。
- 页面写响应是评论创建/回复的权威成功证据；协议复查是补充证据。
- 群聊 @ 继续不支持，也不出现在 Skill 和 CLI 文档中。

# Open questions

无。

用户已于 2026-08-03 确认按上述完整边界进入 Build。

# Verification expectations

- 单元/集成测试覆盖 Windows headless 启动、Linux Xvfb 保留、模式切换互斥、认证各类型、异步无丢唤醒、可见窗口关闭/保留、Profile 状态和 descriptor 模式。
- 用假浏览器和真实 Camoufox 分别证明 broker 复用、20 分钟边界、无控制台窗口及 Windows 正常模式不存在顶层浏览器窗口。
- 增加“页面发布成功但协议复查超时/取消/空响应”的回归，确认 receipt 与 plan 仍为成功且不重发。
- 运行全部 Python 测试、Node 测试、Ruff、Skill Creator、manifest、npm pack/install、凭据与发布内容检查。
- 使用现有真实 Profile 做全量协议读和已授权真实写验收；每项写操作只执行一次，结果未知时停止。
