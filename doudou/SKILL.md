---
name: doudou
description: 使用项目本地 Camoufox Profile 获取抖音群聊、私聊、图片、系统事件、用户视频与评论，并通过常驻后台有头浏览器执行一次正常页面发送、回复、@、图片、点赞和评论操作。适用于登录、拉取消息、发送或回复、采集用户视频/评论、诊断登录或页面协议变化。
---

# Doudou

从用户当前项目运行 `python <skill-directory>/bin/doudou.py ...`。不要切换到 Skill 目录；当前目录就是默认数据根。stdout 是 NDJSON。

## 每次任务

1. 运行 `python <skill>/bin/doudou.py doctor`。
2. 返回 `setup_required` 时运行 `python <skill>/bin/doudou.py setup`，再运行 doctor。
3. 运行 `python <skill>/bin/doudou.py profile list`。
4. 没有 Profile 时创建并登录：

```text
python <skill>/bin/doudou.py profile create --profile <name>
python <skill>/bin/doudou.py profile login --profile <name>
```

登录窗口会保持到 Cookie 与 IM 初始化都通过。遇到扫码、快捷登录后的手机验证码、普通短信验证码、滑块或页面验证时，broker 会显示同一 Profile 的现有窗口让用户接管；不要新开浏览器，也不要提前关闭窗口。`LOGIN_REQUIRED` 表示登录 Cookie 缺失；`VERIFICATION_REQUIRED` 表示 Cookie 仍在但页面验证或 IM 初始化未通过；`WRITE_VERIFICATION_REQUIRED` 表示写操作触发验证，先只读确认原写入结果，不要自动重发。

Windows 默认不显示 broker 的命令行窗口，也不显示浏览器。只有上述人工接管状态或显式 `profile login` 才显示浏览器；正常命令不要主动运行 `browser.visible`。

5. 后续命令复用该 Profile 的常驻 broker。broker 首条业务命令内部执行一次 `ensure_ready`，其余命令只做健康检查。不要运行预热命令；正式 CLI 没有 `warmup`。broker 空闲 20 分钟退出，下次命令自动重新启动并初始化一次。

## 会话与消息

```text
python <skill>/bin/doudou.py conversations refresh --profile <name> --kind all
python <skill>/bin/doudou.py conversations resolve --profile <name> --target <ID或名称>
python <skill>/bin/doudou.py messages collect --profile <name> --conversation <ID或名称>
```

消息默认拉取本地当天零点到当前时间，也可传 `--from`、`--to`。群聊和私聊只按 Protobuf `conversation_type` 分类：`1` 私聊、`2` 群聊。不要依据标题中的“群”判断，不要遍历 React/Fiber 或任意对象图。

拉取使用页面上下文 HTTP/Protobuf，不打开或滚动聊天消息 DOM。图片自动下载并校验 AES-256-GCM 标签；严格识别完整 32 位 MD5，根据资源字段实际口径校验加密体或明文，再校验明文大小与文件格式。进群消息结构化为 `system_event.kind=member_joined`；未知字段进入 inventory。

全部页面读协议只在页面读取 HTTP 状态、Content-Type、Retry-After 和原始文本，由 Python 统一分类。`RATE_LIMITED`、`UPSTREAM_UNAVAILABLE`、`*_RESPONSE_EMPTY`、`*_RESPONSE_INVALID`、`*_COLLECT_FAILED` 与 `PROTOCOL_SCHEMA_DRIFTED` 必须原样报告；仅 5xx 自动重试一次，其他错误不要循环重试。错误记录只含响应长度和 SHA-256，不保存正文。

## 发送、回复、@、图片

先生成计划：

```text
python <skill>/bin/doudou.py protocol send --profile <name> --recipient <会话> --text <正文>
python <skill>/bin/doudou.py protocol send --profile <name> --recipient <群聊> --text <正文> --at <成员>
python <skill>/bin/doudou.py protocol send --profile <name> --recipient <会话> --image <文件>
python <skill>/bin/doudou.py protocol reply --profile <name> --recipient <会话> --message <消息ID或文本> --text <正文>
```

展示返回的 `plan` 并获得明确确认。确认后只需要计划 ID：

```text
python <skill>/bin/doudou.py protocol send --profile <name> --confirm --plan-id <id>
python <skill>/bin/doudou.py protocol reply --profile <name> --confirm --plan-id <id>
```

计划确认后，broker 使用真实正文、成员、引用目标或图片执行一次正常页面动作，并监听 `/v1/message/send` 与服务端回执，再按服务端消息 ID、会话、类型、引用和 @ UID 做只读协议复查。协议负责目标解析、成员解析、计划和校验；页面动作负责最终提交。不要直接构造或重放发送请求，不要使用临时正文，不要根据页面提示文字声称发送成功。回执或协议复查不完整时返回 `WRITE_RESULT_UNKNOWN`，不要自动重试。

批量发送也先预览再确认：

```text
python <skill>/bin/doudou.py protocol send-batch --profile <name> --conversation-ids <ID1,ID2> --text <正文>
python <skill>/bin/doudou.py protocol send-batch --profile <name> --confirm --plan-id <id>
```

同一 broker 严格串行执行，相邻尝试至少间隔 5 秒；首个失败后停止，剩余项标记为 `skipped`。

聊天点赞同样先预览再确认：

```text
python <skill>/bin/doudou.py chat like --profile <name> --conversation <会话> --message <消息ID或文本>
python <skill>/bin/doudou.py chat like --profile <name> --confirm --plan-id <id>
```

## 用户视频与评论

“抖音号”使用 `unique_id`，主页 `/user/<值>` 使用 `sec_uid`；不要把昵称、uid、sec_uid 和抖音号混为一个字段。

```text
python <skill>/bin/doudou.py identity resolve --profile <name> --douyin-id <抖音号>
python <skill>/bin/doudou.py videos collect --profile <name> --douyin-id <抖音号>
python <skill>/bin/doudou.py video collect --profile <name> --sec-uid <sec_uid> --aweme-id <视频ID>
```

视频列表、SSR 详情、封面、根评论和回复使用结构化协议数据。视频列表翻页由版本化 Profile Component 的受控滚动触发页面生成新请求，数据仍只取协议 JSON，不拼接或复用过期签名 URL。评论、回复评论和评论点赞先生成计划，再用 `--confirm --plan-id <id>` 执行一次正常页面操作；成功还必须通过 CID、父 CID、正文或 `user_digged` 的只读协议复查：

评论采集每个根页和每个完成的回复线程都会写 `comment-checkpoint.json` 与 `comments.partial.jsonl`；中断后再次执行相同命令会从服务端游标继续并按 CID 去重。`PAGE_ADAPTER_DRIFTED` 只影响依赖该页面控件的写操作，协议读取仍可继续。

```text
python <skill>/bin/doudou.py video comment --profile <name> --sec-uid <sec_uid> --aweme-id <视频ID> --text <正文>
python <skill>/bin/doudou.py video reply --profile <name> --sec-uid <sec_uid> --aweme-id <视频ID> --comment-id <cid> --text <正文>
python <skill>/bin/doudou.py video like --profile <name> --sec-uid <sec_uid> --aweme-id <视频ID> --comment-id <cid>
```

## 诊断

```text
python <skill>/bin/doudou.py session status --profile <name>
python <skill>/bin/doudou.py session refresh --profile <name>
python <skill>/bin/doudou.py page-doctor --profile <name>
python <skill>/bin/doudou.py protocol capture --profile <name>
```

`session refresh` 只在同一浏览器中强制刷新会话索引，不是业务前置步骤。`protocol capture` 打开同一 Profile 的有头窗口，等待用户正常发送一次并保存脱敏证据。

`page-doctor` 按 Component Adapter 报告页面版本、路径、是否 exercised、候选数、唯一性、可见/启用状态、定位策略和受影响能力。没有选择会话时聊天组件是 `not_exercised`，不是漂移。`PAGE_ADAPTER_DRIFTED` 表示页面定位契约变化，不等于登录失效；`PROTOCOL_SCHEMA_DRIFTED` 表示接口字段契约变化。两者都应由维护者更新对应 adapter，不要修改业务模型或深遍历 React/Fiber。

## 数据边界

- Profile：`profile/<name>/`
- 生产数据：`message/profiles/<name>/`
- 用户级共享 venv 与浏览器：操作系统用户缓存
- Skill 包不包含 Profile、消息、Cookie、venv、浏览器或 `analyze/`
- `profile list` 只报告版本、登录/锁状态、最近索引、聊天/内容计数和最近写入时间，不汇总消息正文或 Cookie。
- 旧 Chromium Profile 首次使用时原子备份为 `profile/<name>__chromium-backup-<UTC>/`，新建同名 Camoufox Profile；不要合并浏览器数据库。

文件结构见 [references/layout.md](references/layout.md)，IM 字段见 [references/protobuf-message-fields.md](references/protobuf-message-fields.md)。
