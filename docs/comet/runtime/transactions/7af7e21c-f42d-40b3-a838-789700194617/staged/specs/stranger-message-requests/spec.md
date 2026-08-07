# Stranger message requests

## Purpose

通过抖音网页 IM 的 stranger 协议发现并只读采集消息请求，同时保持普通会话、身份补齐、存储和写入边界可预测。

## Requirements

### Stranger envelope

系统必须支持 command 1001 使用 RequestBody/ResponseBody field 1000，以及 command 1002 使用 field 1001，不得假设 command 恒等于 body field。

#### Scenario: 构造 1001 列表请求

- Given 已捕获可用的 IM 初始化 envelope
- When 系统构造陌生人列表请求
- Then outer command 为 1001
- And RequestBody 中使用 field 1000
- And inbox type 延续页面已验证的值

#### Scenario: 解析 1002 消息响应

- Given HTTP 200 且 protobuf status 0 的 1002 响应
- When 系统提取 ResponseBody
- Then 从 field 1001 取得 stranger messages body

### 列表分页

系统必须通过 `/v1/stranger/get_conversation_list` 读取 `cursor/count/show_total_unread`，并解析 `next_cursor/has_more/total_unread/conversation_list`。

#### Scenario: 多页消息请求

- Given 1001 首页面返回 `has_more=true` 和新的 next cursor
- When 系统刷新消息请求
- Then 使用 next cursor 继续请求
- And 合并后的会话按 conversation ID 去重
- And 直到 `has_more=false` 才完成

#### Scenario: 分页不前进

- Given 上游连续返回相同 cursor 且 `has_more=true`
- When 系统尝试继续分页
- Then 明确报告协议分页漂移
- And 不进入无限循环

### 会话模型与存储

陌生人记录必须保留 conversation ID、short ID、未读数、最后消息、participants 和 ext，并以 `inbox=request` 表明其 message-request inbox。普通会话使用 `inbox=established`。`kind` 继续只表示 `direct|group`。

#### Scenario: 默认合并会话

- Given 普通会话和陌生人会话都成功刷新
- When 用户运行 `conversations refresh --kind all`
- Then 返回值默认包含两类会话
- And 普通会话标记为 `inbox=established`
- And 陌生人会话标记为 `inbox=request`
- And 两类会话都可通过既有 resolve/select 流程选择

#### Scenario: 空消息请求

- Given 1001 返回空 conversation_list、`has_more=false` 和 total_unread 0
- When 系统刷新会话
- Then 返回成功的空消息请求集合
- And 普通会话索引保持可用

#### Scenario: Stranger refresh failure is atomic

- Given ordinary conversation initialization succeeds
- And the stranger list request fails or reaches a verification gateway
- When the combined conversation refresh runs
- Then the refresh returns an error
- And the previous conversation index remains unchanged
- And no partial combined index is published

### 只读消息采集

对 message-request inbox 会话，系统必须调用 `/v1/stranger/get_messages`，使用 command 1002/field 1001，并固定 `reset_unread_count=false`。

#### Scenario: 采集陌生人消息

- Given 已选择一条陌生人会话
- When 用户运行消息采集
- Then 系统按 short ID 请求 1002
- And 不重置未读数
- And 复用现有 MessageBody 标准化、去重、冲突和媒体存储契约

### 发送者昵称补齐

陌生人 participants 和消息发送者必须进入现有身份缓存与用户信息补齐链，输出统一身份字段；上游缺失值保持 null。

#### Scenario: 用户信息返回昵称

- Given stranger participant 或消息发送者具有可查询的 uid/sec_uid
- And 用户信息协议返回 nickname 或 remark name
- When 会话或消息被标准化和持久化
- Then 对应 identity 包含 nickname、remark_name 和 display_name
- And unique_id/sec_uid/uid 按各自来源保存

#### Scenario: 用户信息缺少昵称

- Given 用户信息响应没有昵称
- When 系统保存陌生人消息
- Then nickname 和 display_name 按现有身份规则保持 null
- And 不使用会话 ID、uid 或其他标签伪造昵称

### 写入边界

陌生人会话不得进入现有发送、回复、点赞或批量写入计划。

#### Scenario: 尝试向陌生人会话写入

- Given 目标会话属于 message-request inbox
- When 用户请求任一现有聊天写操作
- Then 系统在生成 plan 或执行页面动作前返回明确的不支持错误
- And 不产生网络写请求

### 真实验证

实现必须以真实 Camoufox Profile 验证 1001；非空条件下还必须验证 1002、发送者昵称补齐和本地存储，不保存凭据或消息正文到可提交证据。
