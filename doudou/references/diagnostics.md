# Live diagnostics

## Evidence hygiene

- 从用户项目运行 `<cli>`，先捕获 stdout/stderr，再逐行解析 NDJSON。不要把完整原始对象直接回显。
- 默认只保留版本、Profile 槽位、退出码、事件名、稳定错误码、耗时、HTTP/协议状态、分页和去重计数。
- 不展示 Cookie、token、`sec_uid`、完整会话 ID、成员列表或消息正文；确需定位目标时使用匿名化槽位或哈希。
- 只报告实际执行的命令和真实返回。没有输出字段或代码路径证据时，不声称使用了“直连”“兜底”或缓存。

## Entry drift

先确认正式入口：

```text
<cli> --version
<cli> doctor
<cli> profile list
```

只运行 `<cli>`，不直接执行 npm 内部的 `scripts/doudou.py`。`LEGACY_UNSUPPORTED`、CLI usage/exit 2、缺少参数或未知参数都属于入口、当前格式或命令契约问题，尚未进入浏览器协议层。

不确定参数时运行 `<cli> <command> -h`。当前 `messages collect` 只接受一个 `--conversation`；多个目标逐个执行，不使用 `--conversations`。

## Profile selection

同一任务复用已选 Profile。没有既定选择时，从 `login_status=valid`、锁空闲的 Profile 中选择 `last_login_verified_at` 最新者。诊断多个 Profile 时给它们分配 `profile_slot=1..N`，不要在报告中展开账号资料。

遇到 `PROFILE_IN_USE` 时报告脱敏 owner 并按返回的恢复契约处理。登录、可见验证和 IM 初始化是独立状态；按 [runtime.md](runtime.md) 的稳定错误分类处理，不用重新登录掩盖协议错误。

## Group message protocol

群聊正式消息读取使用页面上下文 command 301 和 `/v1/message/get_by_conversation`。验证标准链路：

1. 运行 `<cli> conversations refresh --profile <name> --kind all --force`，要求 `source=live`、exit 0。诊断标准链路必须绕过两小时会话缓存；普通读取才可省略 `--force`。
2. 从结构化结果按 `kind=group` 选目标，不根据群名猜测类型。
3. 固定同一 RFC3339 时间窗，逐个运行：

   ```text
   <cli> messages collect --profile <name> --conversation <id> --from <time> --to <time> --max-pages 2 --skip-images
   ```

4. 每轮记录目标槽位、exit、耗时、error code、是否包含 `NetworkError`、page count、`received`、HTTP status、protobuf status、message count、added、duplicates 和 conflicts。
5. 需要判断连续性时轮换目标重复至少两轮。第二轮应产生稳定 duplicates；它证明协议数据确实进入存储合并，而不只是会话 lookup 成功。

`warmup_source=stale_cache` 只表示目标 lookup 使用了两小时内快照，不代表消息协议走了兜底。消息采集仍必须建立业务会话并取得真实 command 301 响应。

## Failure classification

- **CLI/entry**：usage、exit 2、`LEGACY_UNSUPPORTED`、未知参数；协议未开始。
- **Profile lock**：`PROFILE_IN_USE`；不要并行启动第二个 browser/broker。
- **Authentication**：`LOGIN_REQUIRED` 或 `VERIFICATION_REQUIRED`；与 NetworkError 分开报告。
- **IM initialization**：`IM_CAPTURE_MISSING` 或 `IM_INITIALIZATION_MISSING`；保持登录状态诊断初始化。
- **Page network**：错误明确包含 `NetworkError`；记录轮次和有界重试结果，不泛化为 Protobuf schema 错误。
- **Protocol**：HTTP 非 200、protobuf status 非 0、`PROTOCOL_SCHEMA_DRIFTED` 或分页不前进。
- **Storage**：协议成功但 merge 产生冲突或计数异常；单独检查去重和来源优先级。

真实诊断结束后报告成功/总轮次、NetworkError 数、失败数和一个代表性 page 元数据。无法复现历史说法时明确写“当前未复现”，不要把旧报告复制成当前结论。
