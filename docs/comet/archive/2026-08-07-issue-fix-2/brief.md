# Outcome

修复 GitHub Issue #2 的全部缺陷：broker 能稳定返回超过 64 KiB 的会话结果；Doudou 能发现并只读采集陌生人/消息请求会话及消息，并为陌生人消息发送者补齐可用昵称。

# Scope

- 为 broker 的换行分帧响应增加显式、有界且明显高于 64 KiB 的读取上限，并把超限和断连转换为稳定错误分类。
- 接入 IM command 1001 的陌生人会话列表，支持 cursor 分页、未读数、参与者、最后消息与 ext 解析。
- 接入 IM command 1002 的陌生人消息读取，固定 `reset_unread_count=false`，不因采集改变已读状态。
- 修正 command 与 RequestBody/ResponseBody 字段号不相等的 envelope 映射：1001 对应 field 1000，1002 对应 field 1001。
- 将陌生人会话纳入可解析、可选择和可存储的会话模型，并与普通私聊明确区分读取路由。
- 复用现有身份缓存和用户信息协议，为陌生人参与者及消息发送者补齐 `uid/sec_uid/unique_id/nickname/remark_name/display_name`；缺失值保持 null。
- 增加单元、服务、broker TCP 回归测试，并使用真实 Camoufox Profile 验证 1001、非空条件下的 1002、昵称补齐和超过 64 KiB 的最终 CLI 响应。
- 更新 CLI、布局、协议字段和使用文档。

# Non-goals

- 不实现删除、标记已读、接受消息请求或其他 stranger 写操作。
- 不改变现有聊天发送、回复、点赞的确认与回执契约。
- 不把 Profile、Cookie、token、原始消息或研发抓包提交到 Git/npm 包。
- 不在本 change 中处理与 Issue #2 无关的 Camoufox 指纹告警。

# Acceptance examples

- 当 `conversations refresh` 的 JSON 响应为 128 KiB 以上时，CLI 返回完整 JSON，不再抛出 `LimitOverrunError`。
- 当 broker 响应超过项目定义的最大帧时，读操作返回稳定的响应过大错误；写操作保留结果未知语义且不自动重放。
- 当陌生人列表超过单页时，系统使用 1001/field 1000 按 next cursor 拉完，游标不前进或超过页数上限时明确失败。
- 当陌生人会话被选择采集时，系统使用 1002/field 1001，且请求中的 `reset_unread_count` 为 false。
- 当陌生人消息含发送者 uid/sec_uid 时，输出和持久化记录包含通过现有身份链补齐的 nickname/display_name；上游没有值时对应字段为 null，不以会话 ID 或昵称伪造账号字段。
- 当当前账号没有消息请求时，陌生人列表返回空集合和 total_unread=0，普通会话仍正常可用。
- 当尝试把陌生人会话交给现有写流程时，在生成写 plan 前返回明确的不支持错误。

# Constraints and invariants

- 使用现有 Python/Camoufox 页面上下文 HTTP/Protobuf 路径，不遍历 React/Fiber 或聊天 DOM 获取消息。
- broker 帧仍保持单行 JSON 协议，但读取必须有界；不得采用无限制缓冲。
- 1001 endpoint 为 `/v1/stranger/get_conversation_list`，1002 endpoint 为 `/v1/stranger/get_messages`。
- 真实浏览器验证只保存脱敏报告、大小和哈希；原始二进制仅保留在被 Git/npm 忽略的本地诊断目录。
- 所有实现、测试和 Native 产物都位于 `D:\Code\issue-fix-2` worktree 和 `codex/issue-fix-2` 分支。

# Decisions

- Change 和 worktree 名称为 `issue-fix-2`，Git 分支为 `codex/issue-fix-2`。
- Issue #2 的两个问题必须在同一 change 中修复并完成实测。
- “陌生人发送也要拉昵称”按陌生人消息发送者身份补齐处理，沿用现有身份字段与 null 规则。
- 真实浏览器已确认页面原生 1001 请求为 command 1001、inbox_type 1、body field 1000、cursor 0、count 1、show_total_unread true；复用初始化 envelope 将 count 改为 50 后返回 HTTP 200、protobuf status 0。
- 陌生人/消息请求默认合并进 `conversations refresh --kind all`，与普通会话共用发现、解析和选择流程，并通过 `inbox=established|request` 区分；`kind` 继续只表示 `direct|group`。
- 普通会话成功但陌生人列表请求失败时，整个 refresh 失败并保留旧会话索引；不得返回看似完整的部分结果。

# Open questions

- 用户已确认：默认合并陌生人会话；陌生人列表失败时 refresh 整体失败并保留旧索引；broker 大帧、1001/1002 只读采集、陌生人发送者昵称补齐和真实浏览器实测按本 brief/spec 执行。

# Verification expectations

- 运行相关 Python 单元测试和完整 `docs/verification/tests`。
- 运行 Ruff、Node 测试、manifest、pack/install、release 版本与 Skill 快速校验门禁。
- 使用 fake broker handler 产生至少 128 KiB 的真实 TCP 响应，验证客户端完整解码。
- 使用真实 Camoufox Profile 抓取页面原生 1001，并以 count 50 验证列表请求。
- 使用具有非空消息请求的 Profile 验证 1001 分页、1002 `reset_unread_count=false`、消息持久化和昵称补齐；证据不得包含身份凭据或消息正文。
