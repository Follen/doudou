# Outcome

把文本发送、引用回复、群聊 @、图片发送和带图片消息拉取统一为协议优先实现。业务链路只复用 `/chat` 页面中的登录态、Cookie 和动态安全参数，不再通过输入框、发送按钮、文件控件、会话列表或 React/DOM 对象触发请求。图片拉取后完成下载、AES-GCM 解密、完整性校验和项目本地持久化。

# Scope

- 移除正式文本发送与引用回复现有的 DOM bootstrap；直接构造 Protobuf 并通过页面上下文 `fetch` 调用 `/v1/message/send`。
- 为群聊文本发送增加一个或多个 @ 目标；从协议成员目录解析稳定 UID，编码正文区间、mention metadata 和字体 metadata。
- 为 `protocol send` 增加图片 payload 模式，完成上传配置、Volcengine SigV4 Apply、TOS 二进制上传、Commit 加密、`batch_build_image` 和 `message/send` 全链路。
- 消息拉取遇到图片消息时，解析 `resource_url`、下载密文、使用 `skey` 执行 AES-256-GCM 解密，校验长度、格式和 MD5，再原子写入 conversation 的 media 目录。
- 更新 CLI 帮助、Skill 指令、布局与 Protobuf 字段文档、单元/契约测试和发送/拉取回执。
- 群聊和私聊继续共享相同的会话发现、确认计划、串行发送和服务端回执规则；@ 仅适用于群聊。

# Non-goals

- 不实现创建新私聊、全站用户搜索或邀请成员。
- 不实现视频、语音、文件、贴纸或图片编辑。
- 不实现图片引用回复、图文混合为一条消息，或 @ 私聊对象。
- 不使用 DOM、React Fiber、输入框、按钮、文件 input 或可见文本作为消息业务链路的 fallback。
- 不把 `analyze/` 实验、用户 Profile、消息或媒体文件打进交付 Skill。

# Acceptance examples

- 给定已登录 Profile 和唯一群聊，确认普通文本发送后，CLI 不执行 DOM 消息操作，直接发送 Protobuf，服务端回执中的 HTTP、status、result、client ID 和 conversation 均通过。
- 给定群聊成员“露露缇娅”和正文“很棒”，确认发送后，正文为 `@露露缇娅 很棒`，mention 使用稳定 UID，location/length 按 UTF-16 计算，并且服务端回执通过。
- 给定本地 PNG/JPEG/WebP 图片，确认发送后，CLI 完成协议上传和 `message_type=27`、`aweType=2702` 发送；任一步失败都不报告 sent。
- 给定拉取范围内含图片消息，CLI 保存消息记录并把每张图片解密到该会话的 `media/YYYY-MM-DD/<message-id>/`；记录本地相对路径、格式、字节数、MD5/SHA-256 和下载状态。
- 给定下载、GCM authentication tag、声明长度或 MD5 校验失败，消息文本/协议记录仍可落盘，媒体状态为失败且保留可重试信息；不得保存部分明文文件。
- 给定私聊使用 `--at`、发送 payload 同时包含 `--text` 与 `--image`、目标成员不存在或重名，CLI 在确认前返回稳定错误或候选，不发送消息。

# Constraints and invariants

- 业务操作允许导航到 `/chat`、等待协议初始化并执行页面上下文 `fetch`；不得查询、填写、点击或滚动消息 DOM。
- 页面网络层继续负责 Cookie 与 `a_bogus`、`msToken`、`fp`、`verifyFp` 等动态参数；CLI 不持久化完整签名 URL、Cookie 或临时凭证。
- 图片上传配置通过 `/aweme/v1/web/im/upload/config/v2` 获取并使用 `public_image_config`；Apply/Commit 使用页面实际 SDK 的 AWS4-HMAC-SHA256 规则，TOS 上传使用服务端下发的 Auth 和 storage headers。
- 图片密文格式固定按实验确认的 `12-byte IV + ciphertext + 16-byte tag` 解码，算法为 AES-256-GCM；顶层 `content.md5` 是明文校验值，不能用 `resource_url.md5` 替代。
- 所有写操作继续要求 preview + plan ID + 明确确认；批量发送保持严格串行，成功发送之间等待 5 秒。
- Profile 和 `message/` 保持项目本地、被 Git 忽略；生产媒体不得写入安装后的 Skill 目录。

# Decisions

- 用户明确要求能走协议就走协议，并禁止 DOM 消息操作；文本、回复、@、图片发送和图片拉取采用同一标准。
- 采用最小浏览器边界：浏览器只承载登录态、页面安全环境和必须的页面上下文 fetch；VOD/TOS 上传及签名、图片下载和解密在 Node.js 中完成。
- `protocol send` 使用互斥 payload：`--text` 或 `--image`；`--at` 可重复或接收逗号列表，但仅与群聊文本 payload 一起使用。
- 图片拉取默认随 `messages collect` 执行，不另要求用户运行第二条下载命令；单张媒体失败不阻断同一会话其他消息和其他会话。
- 实验成功后才修改正式代码。已完成的真实实验：零 DOM @ 发送、零 DOM图片发送、协议拉回同一图片并下载解密，三项均通过服务端或完整性校验。
- 用户已于 2026-08-01 确认本 brief 与两份拟议完整规格，批准进入正式实现。

# Open questions

无。

# Verification expectations

- 单元测试覆盖 SigV4 canonical request、图片 content builder、UTF-16 mention 区间、多 @、payload 互斥、群聊限制、AES-GCM 解密、格式探测、原子媒体写入和失败重试状态。
- 契约测试证明发送路径不调用 `locator`、`fill`、`click`、`press`、`setInputFiles` 或 `openConversation`，并验证页面上下文 fetch 的动态 query 与回执判断。
- 集成 fixture 覆盖 PNG/JPEG/WebP、图片消息重复拉取、损坏 tag、长度/MD5 不匹配、过期 URL和批量发送 5 秒间隔。
- 使用现有 Profile 做一次真实群聊 @、一次真实图片发送及同消息拉取闭环；线上证据只记录脱敏 endpoint、hash、状态和项目相对产物路径。
