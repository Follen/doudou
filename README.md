# Doudou

可交付的 Codex Skill 与 Python CLI，用项目本地 Camoufox Profile 读取抖音聊天、图片、系统事件、用户视频和评论，并通过常驻后台有头浏览器执行受控写操作。

## 安装

需要 Node.js `>=18.18` 和 Python `>=3.10,<4.0`。npm 包只携带启动器和 Skill 源码，不携带 venv、Python 包或浏览器。

```powershell
npm install -g @follenfang/doudou
doudou doctor
```

`doudou` 首次运行会把版本匹配的 Skill 原子安装到 `~/.agents/skills/doudou/`。首次业务命令发现运行时缺失时，会自动执行 setup：在用户缓存创建共享 venv、安装锁定依赖、下载并验证 Camoufox `152.0.4-beta.28`，然后继续原命令。也可以提前执行：

```powershell
doudou setup
doudou profile create --profile work
doudou profile login --profile work
```

不希望全局安装时可使用 npx；即使 npm lifecycle scripts 被禁用，首次命令仍会补齐 Skill：

```powershell
npx --yes @follenfang/doudou -- doctor
npx --yes @follenfang/doudou -- profile list
```

Skill 目录可用 `--skill-dir <完整路径>` 临时覆盖，或用 `DOUDOU_AGENT_HOME` 把默认位置改为 `<home>/skills/doudou`。Profile 与业务记录始终留在运行命令时的当前项目：

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
doudou conversations refresh --profile work --kind all
doudou messages collect --profile work --conversation "会话名称"
doudou protocol send --profile work --recipient "会话名称" --text "你好"
doudou protocol send --profile work --confirm --plan-id <id>
doudou protocol send-batch --profile work --conversation-ids <ID1,ID2> --text "你好"
doudou videos collect --profile work --douyin-id Rurutia.wow
doudou video collect --profile work --sec-uid <sec_uid> --aweme-id <视频ID>
```

完整 Agent 编排见 [`doudou/SKILL.md`](doudou/SKILL.md)，文件结构见 [`doudou/references/layout.md`](doudou/references/layout.md)。

## 升级与卸载

```powershell
npm install -g @follenfang/doudou@latest
doudou uninstall
```

`doudou uninstall` 会停止受管 broker，删除全部 npm 管理的 Skill 和共享 venv/runtime，再删除 npm 包自身。项目中的 `profile/`、`message/` 和共享 Camoufox 浏览器缓存会保留。npm 7+ 不执行卸载 lifecycle，直接使用 `npm uninstall` 不会清理这些运行文件。

## 故障诊断

- `PYTHON_VERSION_UNSUPPORTED`：安装 Python `>=3.10,<4.0` 后重试。
- `SKILL_INSTALL_FAILED`：目标 Skill 目录存在未知内容；换用 `--skill-dir`，或先人工核对该目录。
- `RUNTIME_SETUP_FAILED`：运行 `doudou setup` 查看依赖或浏览器下载阶段的原始错误。
- 登录、短信、滑块或写验证：运行对应命令后，由同一 broker 显示浏览器供人工接管。

## 发布

GitHub Actions 只在 `v*` tag 推送时发布。tag 去掉 `v` 后必须与 `package.json.version` 和 Python Skill 版本完全一致。npm Trusted Publisher 需要绑定仓库 `Follen/doudou` 和 workflow `publish-npm.yml`，发布使用 GitHub OIDC，不使用长期 `NPM_TOKEN`。npm 目前不为私有 GitHub 仓库生成 provenance；仓库保持私有时仍使用 OIDC 发布，但发布记录不带 provenance。

```powershell
git tag -a vX.Y.Z -m "Doudou X.Y.Z"
git push origin vX.Y.Z
```

## 开发验证

```powershell
$env:PYTHONPATH = "doudou\scripts"
python -m pytest docs\verification\tests -q
```

发布 tarball 包含 npm 启动器和完整 Skill 源码，但不包含 venv、Python packages、浏览器二进制、Profile、消息或 `analyze/` 数据。

批量发送一次确认后复用同一 broker，严格串行，相邻尝试至少间隔 5 秒；首个失败后其余项目标记为 `skipped`，不会继续写入。
