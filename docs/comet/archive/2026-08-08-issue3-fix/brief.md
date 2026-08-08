# Outcome

修复 GitHub Issue #3：`video collect` 不再因唯一、可见、可用的展开回复按钮仅轻微越出评论面板局部边界而误报 `outside_local_scope`，同时保留对真正越界、歧义或不可用目标的拒绝。

# Scope

- 调整 `VideoCommentsPanelComponent.trigger_reply_template` 的滚动与几何确认顺序。
- 为该能力使用有界的 4px 组件级局部容差，不改变全局默认容差。
- 增加 2.1px 轻微越界、滚动后点击成功，以及超过 4px 仍失败的自动化回归。
- 使用已登录 `Follen-doudou` Profile 对 Issue 视频 `7665571513114194618` 执行修改前后正式 CLI 实测。
- 完成 PR、独立 Agent 审核、合并、CI、tag 和 npm 发布。

# Non-goals

- 不放宽其他组件或其他写操作的几何约束。
- 不在目标不唯一、不可见、禁用、语义不匹配或明显越界时点击。
- 不改变评论协议分页、增量合并、身份解析或存储格式。

# Acceptance examples

- 给定展开回复按钮唯一、可见、可用，且其边界仅比评论面板多出 2.1px，组件先滚动该按钮，再以 4px 容差确认并点击，不再返回 `outside_local_scope`。
- 给定按钮滚动后仍超出评论面板 4px 以上，组件返回 `PAGE_ADAPTER_DRIFTED/outside_local_scope`，且不点击。
- 给定按钮第一次点击失败，现有有界重试仍生效；达到重试上限后返回包含尝试次数的稳定错误。
- 给定 Issue 视频和已登录 Profile，修改后的正式 CLI 采集成功，根评论与多页回复均达到 `has_more=false`，正式评论 CID 唯一且历史 CID 不丢失。

# Constraints and invariants

- 目标仍必须唯一、可见、可用，并具有“展开/查看回复”语义。
- 4px 容差只应用于 `video.comments.reply-template`，全局默认值保持 2px。
- 滚动、几何验证和点击均保持有界；失败结果不得被解释为采集完成。
- 真实测试证据只记录计数、分页摘要、哈希和退出状态，不提交 Cookie 或评论正文。

# Decisions

- Issue #3 真实性已确认：2.1px 越界输入在 `0.0.6` 当前实现中稳定复现报告的完整错误字段。
- 采用“先 `scroll_into_view_if_needed`，再以 4px 组件级容差调用 `require_target`”的最小修复。
- Issue 视频修改前实测本次页面布局未触发越界，正式 CLI 成功采集 753 个唯一 CID；该结果作为真实基线，不否定已确定的边界分支缺陷。
- 用户已明确要求在确认 Issue 后完成修复、实测、PR、独立 Agent 审核、合并和 npm 发布。

# Open questions

无。

# Verification expectations

- 定向几何与视频分页测试通过。
- 全量 Python、Node、Ruff、manifest、打包和安装门禁通过。
- 修改后的源码通过正式 CLI 重采 Issue 视频，退出码为 0；根评论与所有回复线程协议终止，CID 唯一且历史零丢失。
- PR 由未参与实现的独立 Agent 审核，无未解决阻塞 finding 后才合并。
