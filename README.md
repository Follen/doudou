# Doudou

可交付的 Codex Skill 与 Python CLI，用项目本地 Camoufox Profile 读取抖音聊天、图片、系统事件、用户视频和评论，并通过常驻后台有头浏览器执行受控写操作。

## 安装

把仓库中的 `doudou/` 复制到 `~/.agents/skills/doudou/`。不要复制项目根的 `profile/`、`message/` 或 `analyze/`。

在实际业务项目中运行：

```powershell
python "$HOME\.agents\skills\doudou\bin\doudou.py" doctor
python "$HOME\.agents\skills\doudou\bin\doudou.py" setup
python "$HOME\.agents\skills\doudou\bin\doudou.py" profile create --profile work
python "$HOME\.agents\skills\doudou\bin\doudou.py" profile login --profile work
```

`setup` 在用户缓存建立共享 venv，锁定 `camoufox==0.5.4` 和官方浏览器 `152.0.4-beta.28`。Profile 与业务记录始终留在当前项目：

```text
profile/<profile>/
message/profiles/<profile>/
```

## 工作方式

- 每个项目和 Profile 只有一个 Python broker。
- Windows 默认隐藏有头；Linux 默认 Xvfb 虚拟有头。
- broker 串行执行命令，空闲 20 分钟退出。
- 首条业务命令内部初始化页面与 IM；没有用户侧预热流程。
- 会话、消息、图片、视频和评论读取使用页面上下文协议。
- 文字、引用、@、图片、聊天点赞、视频评论/回复/点赞通过一次正常页面动作提交，并用网络回执和只读协议复查验证。
- 图片下载先校验 AES-GCM，再按平台字段口径校验加密体或明文 MD5、明文大小与实际文件格式。
- 视频列表从协议响应读取；翻页由版本化 Profile Component 的受控滚动触发，不复用过期签名 URL。
- 页面读协议统一按 HTTP、Content-Type、Retry-After、响应长度/哈希和 JSON 业务状态分类；评论分页逐页保存检查点，可从失败游标恢复。
- 接口路径/字段别名集中在版本化 Protocol Adapter，页面 selector 集中在版本化 UI Adapter。写控件依次校验稳定语义、局部组件结构、唯一可见/启用状态和几何关系；不唯一时在点击前返回 `PAGE_ADAPTER_DRIFTED`，不会猜测点击。接口结构变化返回 `PROTOCOL_SCHEMA_DRIFTED`，不伪装成登录失效。
- 写操作先生成计划，确认后只使用 `--confirm --plan-id <id>`。

## 常用命令

```powershell
python <skill>\bin\doudou.py conversations refresh --profile work --kind all
python <skill>\bin\doudou.py messages collect --profile work --conversation "会话名称"
python <skill>\bin\doudou.py protocol send --profile work --recipient "会话名称" --text "你好"
python <skill>\bin\doudou.py protocol send --profile work --confirm --plan-id <id>
python <skill>\bin\doudou.py protocol send-batch --profile work --conversation-ids <ID1,ID2> --text "你好"
python <skill>\bin\doudou.py videos collect --profile work --douyin-id Rurutia.wow
python <skill>\bin\doudou.py video collect --profile work --sec-uid <sec_uid> --aweme-id <视频ID>
```

完整 Agent 编排见 [`doudou/SKILL.md`](doudou/SKILL.md)，文件结构见 [`doudou/references/layout.md`](doudou/references/layout.md)。

## 开发验证

```powershell
$env:PYTHONPATH = "doudou\src"
python -m pytest doudou\tests -q
```

正式交付树不包含 Node.js、npm、Playwright、浏览器二进制、Profile 或消息数据。

批量发送一次确认后复用同一 broker，严格串行，相邻尝试至少间隔 5 秒；首个失败后其余项目标记为 `skipped`，不会继续写入。
