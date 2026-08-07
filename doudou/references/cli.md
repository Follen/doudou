# CLI reference

将 `<cli>` 替换为 `doudou`；PATH 中尚无全局命令时使用 `npx --yes @follenfang/doudou`。所有命令从用户项目运行。

## Environment and profiles

```text
<cli> doctor
<cli> setup
<cli> update --check
<cli> update
<cli> profile list
<cli> profile create --profile <name>
<cli> profile login --profile <name>
<cli> session status --profile <name>
<cli> session refresh --profile <name>
```

`session status` 会分别报告当前浏览器模式、账号登录、可见验证组件、IM 初始化捕获和人工接管状态。Windows 正常模式为 `windows_headless`，人工接管时为 `headed_takeover`；账号已登录但 IM 捕获缺失时，应刷新当前会话初始化，不要重新登录。

## Conversations and messages

```text
<cli> conversations refresh --profile <name> --kind all
<cli> conversations refresh --profile <name> --kind all --inbox all
<cli> conversations resolve --profile <name> --target <ID-or-name>
<cli> messages collect --profile <name> --conversation <ID-or-name>
```

`conversations refresh --kind all` 默认合并已建立会话和陌生人消息请求。每条记录通过 `inbox=established|request` 区分来源；`--inbox established|request|all` 可筛选来源。消息默认采集本地当天零点到当前时间；需要指定窗口时传 `--from` 和 `--to`。`conversation_type=1` 是私聊，`conversation_type=2` 是群聊；不要根据标题判断。消息请求只读，不能用于发送、回复或点赞。

## Chat writes

先创建 plan：

```text
<cli> protocol send --profile <name> --recipient <conversation> --text <text>
<cli> protocol send --profile <name> --recipient <conversation> --image <path>
<cli> protocol reply --profile <name> --recipient <conversation> --message <message-id-or-text> --text <text>
<cli> chat like --profile <name> --conversation <conversation> --message <message-id-or-text>
```

确认后执行对应 plan：

```text
<cli> protocol send --profile <name> --confirm --plan-id <id>
<cli> protocol reply --profile <name> --confirm --plan-id <id>
<cli> chat like --profile <name> --confirm --plan-id <id>
```

批量发送：

```text
<cli> protocol send-batch --profile <name> --conversation-ids <ID1,ID2> --text <text>
<cli> protocol send-batch --profile <name> --confirm --plan-id <id>
```

## Users, videos, and comments

“抖音号”是 `unique_id`，主页 `/user/<value>` 使用 `sec_uid`；不要混用昵称、uid、sec_uid 和抖音号。

```text
<cli> identity resolve --profile <name> --douyin-id <douyin-id>
<cli> identity migrate --profile <name>
<cli> videos collect --profile <name> --douyin-id <douyin-id>
<cli> video collect --profile <name> --sec-uid <sec-uid> --aweme-id <video-id>
```

评论写操作先创建 plan，再使用返回的 plan ID 确认：

```text
<cli> video comment --profile <name> --sec-uid <sec-uid> --aweme-id <video-id> --text <text>
<cli> video reply --profile <name> --sec-uid <sec-uid> --aweme-id <video-id> --comment-id <cid> --text <text>
<cli> video like --profile <name> --sec-uid <sec-uid> --aweme-id <video-id> --comment-id <cid>
```

```text
<cli> video comment --profile <name> --confirm --plan-id <id>
<cli> video reply --profile <name> --confirm --plan-id <id>
<cli> video like --profile <name> --confirm --plan-id <id>
```

## Diagnostics

```text
<cli> page-doctor --profile <name>
<cli> page-doctor --profile <name> --conversation <会话名称>
<cli> protocol capture --profile <name>
```

`page-doctor` 诊断页面组件契约；传入 `--conversation` 时会选择该会话并实际检查输入框与发送控件，但不会填写或发送消息。`protocol capture` 显示同一 Profile 的窗口，等待一次正常页面发送并保存脱敏请求/响应证据。
