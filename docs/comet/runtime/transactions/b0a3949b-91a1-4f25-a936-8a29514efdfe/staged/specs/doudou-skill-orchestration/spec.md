# doudou-skill-orchestration

## 正式入口与环境门禁

Doudou Skill 必须从用户当前项目运行，当前目录即业务数据根。PATH 中存在 `doudou` 时使用 `doudou`；否则使用 `npx --yes @follenfang/doudou`。Skill 不得指示 Agent 或用户直接执行 Skill 内 Python 文件、历史 Node 入口或项目 `.agents/` 中的旧副本。

每个任务必须先运行 doctor。返回 `setup_required` 时必须运行 setup 并重跑 doctor；只有 `status=ok` 才能继续。随后必须列出 Profile，同一任务复用已选 Profile；没有既定选择时，只能从 `login_status=valid`、`lock_status=free` 的 Profile 中选择 `last_login_verified_at` 最新者。没有可用 Profile 时进入当前 Profile 创建或登录流程，不得并行启动第二个 broker 或浏览器。

### 场景：无全局命令

- Given PATH 中没有 `doudou`
- When Agent 开始 Doudou 任务
- Then 使用 `npx --yes @follenfang/doudou doctor`
- And 后续命令保持同一入口和当前工作目录

### 场景：确定性 Profile 选择

- Given 多个 Profile 同时存在
- When 用户没有指定 Profile
- Then 只考虑登录有效且锁空闲的 Profile
- And 选择最近完成登录验证的一个

## NDJSON 与敏感数据投影

CLI stdout 必须按 NDJSON 逐行捕获和解析。Agent 默认只向用户报告状态、计数、分页、稳定错误码、匿名化目标槽位和产物路径，不得原样回显完整 Profile、conversation、message 或 diagnostic 对象。Cookie、token、`sec_uid`、完整会话 ID、成员列表和消息正文只有在用户明确要求对应业务记录且符合任务范围时才可最小化展示。

### 场景：Profile 列表输出

- Given `profile list` 返回包含多个完整 Profile 的 result envelope
- When Agent 选择执行目标
- Then Agent 从 `result.profiles[]` 解析登录、锁和最近验证字段
- And 对话中只使用 `profile_slot` 和必要状态，不展开账号资料

## 当前 CLI 契约

命令和参数必须以随包 `references/cli.md` 与 `<cli> <command> -h` 为准。`messages collect` 每次只能接收一个 `--conversation`；多个会话必须逐个调用并分别保留退出码与结果，一个目标失败不得覆盖其他目标结果。

普通会话读取可以复用两小时内缓存。群聊 command 301 或 NetworkError 的标准诊断必须执行 `conversations refresh --profile <name> --kind all --force`，要求 exit 0 且 `source=live`，再按结构化 `kind=group` 选择目标。固定同一 RFC3339 时间窗逐个采集并重复至少两轮，记录 exit、耗时、NetworkError、HTTP/Protobuf 状态、分页、received、added、duplicates 和 conflicts；第二轮 duplicates 用于证明协议结果进入增量存储。

### 场景：两个会话采集

- Given 用户要求采集两个会话
- When Agent 构造命令
- Then 串行执行两次带单个 `--conversation` 的 `messages collect`
- And 不构造不存在的 `--conversations`

### 场景：强制 live 诊断

- Given 会话索引可能命中本地缓存
- When Agent 验证群聊 command 301
- Then 会话刷新必须带 `--force`
- And 只有结构化结果为 `source=live` 才进入消息采集结论

## 读取、写入与错误分层

读取必须使用页面上下文中的结构化 HTTP/Protobuf，不得遍历聊天正文 DOM、React/Fiber 或任意对象图。消息、成员、系统事件、引用、视频作者和评论用户使用稳定身份字段；缺失值保持 null，不得用昵称冒充抖音号。

写入必须先生成 plan，展示精确预览并取得明确确认；确认后只能执行返回的 plan ID 一次。页面控件必须唯一、可见、启用并满足局部几何契约。`WRITE_RESULT_UNKNOWN` 必须先只读核对，不得自动重试。

CLI/参数、Profile 锁、登录/验证、IM 初始化、页面 NetworkError、HTTP/Protobuf 协议和存储合并错误必须分别保留稳定分类，不得相互覆盖。当前证据未复现历史故障时必须报告当前未复现，不得复制旧结论。

### 场景：页面 NetworkError

- Given 消息采集明确返回 `NetworkError`
- When Agent 形成诊断
- Then 记录目标槽位、轮次和有界重试结果
- And 不把该错误改写为登录失败、Protobuf schema 漂移或缓存 fallback

## 参考资料按需加载

Skill 主文件必须保持任务导向和简洁，把 CLI 流程、live 诊断、runtime 错误、数据布局和 Protobuf 字段分别路由到对应 reference。Agent 只读取当前任务需要的参考，不一次加载全部资料。Skill metadata、Agent 默认提示、引用文件和 npm skill manifest 必须保持一致并由自动化测试校验。
