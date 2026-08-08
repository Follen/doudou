# Outcome

修复 GitHub Issue #5 在当前 0.0.7 中仍真实存在的陌生人消息请求缺陷：1001/1002 页面协议请求遇到瞬态 NetworkError 时可在有界时间内恢复；已补齐身份的 request 会话可按对方昵称解析；1002 空响应时，将网页实际展示的 1001 `last_message` 作为显式标记的会话摘要消息采集，不伪装成 1002 历史。保持 70 个陌生会话的身份补齐为有界批量操作，避免回退到逐人查询和 90 秒超时。

# Scope

- 为陌生人 1001/1002 的页面上下文 protobuf POST 增加只针对瞬态网络错误的有界重试。
- 刷新 request 会话身份后，使用非本账号参与者的 `remark_name`/`nickname`/`display_name` 更新会话 `display_name`，使 resolve/collect 可按昵称工作。
- 1002 采集页分别记录协议返回数量、1001 摘要数量、collection source 和 fallback 状态；真实 1002 消息优先，1002 为空时允许将网页可见的 1001 `last_message` 作为独立标记的摘要消息交给去重存储。
- 保持身份补齐按批次请求，并增加 70 个陌生会话不会逐人请求的回归测试。
- 更新自动化测试、Skill 参考文档和一份不含正文/凭据的真实验证记录。

# Non-goals

- 不改变陌生人会话只读边界，不增加发送、回复、点赞或批量写入。
- 不把 1001 `last_message` 删除出会话索引；允许它作为网页可见摘要被显式采集，但禁止冒充 1002 历史。
- 不承诺上游 1002 返回其未提供的旧历史，不用 DOM 滚动或页面文本补齐协议历史。
- 不修改普通 established 会话的 301 采集契约。
- Issue #4 已在 0.0.7 修复并完成真实增量验证，本 change 不重复修改评论存储。

# Acceptance examples

- Given 1001 首次页面 fetch 抛出 NetworkError、后续一次成功，When 强制刷新 request inbox，Then 命令成功且只发布完整索引。
- Given 1001/1002 连续达到重试上限仍失败，When 命令结束，Then 返回稳定错误且总耗时受限，不写入部分索引或伪造消息。
- Given request 会话的双方身份已补齐且能识别本账号，When 保存会话索引，Then `display_name` 使用对方的 remark/name/nickname，按昵称 resolve 唯一命中。
- Given 1002 HTTP 200/status 0 但协议消息列表为空且 1001 有网页可见的 `last_message`，When 采集，Then `protocol_received=0`、`summary_received=1`、`collection_source=conversation_summary`、`fallback_used=false`，并按 message ID 去重持久化该摘要消息。
- Given 1002 返回一条或多条真实消息，When 采集，Then 协议计数、去重存储、发送者昵称和本地持久化与真实消息一致。
- Given 70 个陌生会话含待补齐 sec_uid，When 刷新，Then 用户信息查询按固定批量执行，调用次数随批次数增长而非逐人增长，并在 broker 命令预算内完成。

# Constraints and invariants

- 继续使用 command 1001/field 1000、command 1002/field 1001 和 `reset_unread_count=false`。
- 重试只覆盖可识别的瞬态页面网络异常；HTTP/Protobuf 业务错误、验证网关和 schema 漂移保持立即失败。
- 所有真实证据只记录状态、计数、耗时、哈希、重试与分页摘要，不提交 Cookie、sec_uid、消息正文、Profile 或 message 数据。
- Profile、message、analyze 和本地运行时不得进入 Git 或 npm 包。
- 兼容 Windows/Linux，本地发布门禁与 npm 包内容约束保持不变。

# Decisions

- 用户于 2026-08-08 明确确认按本 brief 与完整目标规范进入 Build。
- 用户于 2026-08-08 再次确认：1002 空响应时，将网页可见的 1001 `last_message` 作为 `stranger-conversation-summary` 单独计数并按 message ID 去重存储，不把它标记为 1002 历史；1002 非空时仍优先使用真实协议消息。
- 以当前 0.0.7 的五次连续刷新为真实性基线：前四次成功，第五次复现 `BROKER_INTERNAL_ERROR: Page.evaluate: NetworkError when attempting to fetch resource.`。
- 1001 已返回并补齐对方昵称但会话 `display_name` 仍为 conversation ID，属于当前真实缺陷。
- 真实页面点击陌生会话时只观察到 1001 刷新和 1006 标记已读，没有网页原生 1002 请求；修复不猜测未知字段，保留 1002 探测，并把网页实际显示的 1001 `last_message` 作为独立来源提供可观测、可去重的采集。
- Issue #5 中 0.0.5 的逐人补齐实现已被 20 个一批的实现替代；本 change 用 70 人回归验证并保持该有界行为。

# Open questions

- 无。

# Verification expectations

- 自动化：陌生协议、聊天集成、身份补齐、broker/session、CLI/存储相关测试；完整 Python pytest、Ruff、npm test 与发布门禁。
- 真实 Profile：至少连续刷新 request inbox 5 次；验证发生过一次可识别的重试或用受控瞬态注入证明同一真实会话恢复；按昵称 resolve；点击网页 request 会话抓取实际网络序列并核对 1001 `last_message` 与 DOM；执行 CLI 采集两次，记录 HTTP/status、payload 字节、协议消息数、摘要消息数、collection source、fallback 标志、去重新增/重复计数和匿名化哈希。
- PR 后由未参与实现的独立子 Agent 审核改动与测试；无阻塞发现后合并 main。
- 发布下一未占用 SemVer，推 annotated `vX.Y.Z` tag，等待 GitHub Actions 全绿，确认 npm `latest`，再从 registry 实测全局安装和 npx。
