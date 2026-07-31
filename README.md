# Douyin Message

一个可交付的 Agent Skill 与 Node.js CLI，用于在项目本地管理抖音登录 Profile，并从 `https://www.douyin.com/chat` 完成群聊和私聊的会话发现、消息采集、普通发送与引用回复。

## 特性

- 群聊和私聊统一索引，优先使用 Protobuf `conversation_type` 分类。
- 直接进入 `/chat`，不经过首页消息面板。
- 使用项目本地持久化 Profile，登录后可执行无头预热、采集和发送。
- 发送前生成确认预览；确认后使用页面上下文协议请求，并校验服务端 Protobuf 回执。
- 自动检测 Node.js、npm、Playwright 和对应 Chromium。
- Profile、消息记录与诊断数据保存在调用方项目，不打包进 Skill。
- CLI 标准输出为 NDJSON，方便 Agent 或其他程序解析。

## 环境要求

- Node.js 20 或更高版本
- npm
- 可启动 Playwright Chromium 的桌面或服务器环境
- 首次登录时可打开有头浏览器

## 安装

将仓库中的 `douyin-message/` 安装到 Agent Skill 目录。Windows 示例：

```powershell
git clone https://github.com/Follen/douyin-message.git
Copy-Item -Recurse -Force .\douyin-message\douyin-message "$HOME\.agents\skills\douyin-message"
```

Linux/macOS 示例：

```bash
git clone https://github.com/Follen/douyin-message.git
cp -R ./douyin-message/douyin-message "$HOME/.agents/skills/douyin-message"
```

之后在实际业务项目目录运行 CLI。不要切换到 Skill 安装目录，因为当前工作目录就是默认的数据根目录。

## 快速开始

下面用 `<skill>` 表示已安装的 `~/.agents/skills/douyin-message`，用 `work` 作为 Profile 名称。

### 1. 环境检测与初始化

```bash
node <skill>/bin/douyin-message.js doctor
node <skill>/bin/douyin-message.js setup
node <skill>/bin/douyin-message.js doctor
```

`setup` 会安装锁定版本的 Playwright 依赖和 Chromium。环境已经正常时可以跳过。

### 2. 创建 Profile 并登录

```bash
node <skill>/bin/douyin-message.js profile create --profile work
node <skill>/bin/douyin-message.js profile login --profile work
```

登录命令会打开有头浏览器。完成扫码或验证后，CLI 将登录态和指纹基线保存在当前项目的 `profile/work/`。

### 3. 预热与发现会话

```bash
node <skill>/bin/douyin-message.js warmup --profile work
node <skill>/bin/douyin-message.js conversations refresh --profile work --kind all
node <skill>/bin/douyin-message.js conversations resolve --profile work --target "会话名称"
```

预热会刷新会话索引与必要的页面/网络契约信息，但不会批量拉取所有会话历史。

### 4. 拉取消息

默认拉取本地当天零点至命令执行时刻的消息：

```bash
node <skill>/bin/douyin-message.js messages collect --profile work --conversation "会话名称"
```

指定时间范围：

```bash
node <skill>/bin/douyin-message.js messages collect \
  --profile work \
  --conversation "会话名称" \
  --from "2026-07-31T00:00:00+08:00" \
  --to "2026-07-31T23:59:59+08:00"
```

### 5. 发送消息

先生成预览：

```bash
node <skill>/bin/douyin-message.js protocol send \
  --profile work \
  --recipient "会话名称" \
  --text "消息内容"
```

CLI 返回 `confirmation_required`、`plan_id` 和完整预览。确认无误后，使用原参数加上返回的计划 ID：

```bash
node <skill>/bin/douyin-message.js protocol send \
  --profile work \
  --recipient "会话名称" \
  --text "消息内容" \
  --confirm \
  --plan-id <plan-id>
```

确认后的发送要求本次实时预热成功，不使用过期缓存执行写操作。

同一段文本发送到多个现有会话时，使用批量命令，不要循环调用单条发送：

```bash
node <skill>/bin/douyin-message.js protocol send-batch \
  --profile work \
  --conversation-ids "会话ID1,会话ID2" \
  --text "消息内容"
```

批量命令先返回整批预览，确认后只实时预热一次，并在同一个无头浏览器会话中严格串行发送。相邻两条固定间隔 5 秒；每条消息独立生成签名并校验服务器回执。任一条失败会停止后续发送，并区分已发送、失败和跳过的目标。

### 6. 引用回复

引用回复依赖已经采集到本地的目标消息：

```bash
node <skill>/bin/douyin-message.js protocol reply \
  --profile work \
  --recipient "会话名称" \
  --message-text "目标消息关键词" \
  --text "回复内容"
```

同样需要根据预览返回的 `plan_id` 再次执行，并添加 `--confirm`。

## 数据目录

运行数据始终位于调用方项目：

```text
profile/<profile>/
  profile.json
  fingerprint.json
  browser-data/

message/profiles/<profile>/
  conversations.json
  warmup/
  conversations/
  runs/
  sends/
  diagnostics/
```

- `profile/` 包含登录态，不应提交或分发。
- `message/` 包含会话索引、消息与发送验证记录，不应提交或分发。
- Skill 目录只包含代码、说明和测试，不包含任何用户 Profile。
- 发送回执仅保存脱敏 endpoint、长度、哈希和状态；动态签名只在当前浏览器页内存中使用。

完整文件命名约定见 [`douyin-message/references/layout.md`](douyin-message/references/layout.md)。Protobuf 字段说明见 [`douyin-message/references/protobuf-message-fields.md`](douyin-message/references/protobuf-message-fields.md)。

## 常见恢复流程

- `PROFILE_IN_USE`：确认占用进程后，按 CLI 提示使用 `--terminate-owner` 重试。
- `FINGERPRINT_BASELINE_REQUIRED`：执行 `profile calibrate --profile <name>`。
- `LOGIN_REQUIRED` 或 `VERIFICATION_REQUIRED`：重新执行有头 `profile login`，然后重试原命令。
- 协议字段变化：执行 `protocol capture --profile <name>`，按提示在有头浏览器中完成一次正常发送并关闭浏览器。

## 开发与测试

```bash
cd douyin-message
npm install
npm test
```

当前测试覆盖环境检测、Profile 隔离与锁、指纹持久化、会话解析、消息字段映射、发送请求改写、回执脱敏、存储和 CLI 预览流程。

## 边界

- 只解析和操作现有会话，不提供全局用户搜索或创建新私聊。
- 页面协议和 Protobuf 字段可能随站点更新而变化；CLI 在关键契约漂移时停止写入并返回稳定错误码。
- 登录、校准、人工验证和协议捕获使用有头浏览器；正常预热、采集和协议发送使用无头浏览器。
