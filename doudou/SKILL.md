---
name: doudou
description: 使用项目本地 Camoufox Profile 读取抖音群聊、私聊、图片、系统事件、用户、视频和评论，并通过受控页面动作发送、回复、图片、点赞及评论。用于环境安装与升级、登录或验证码接管、聊天和内容采集、经确认的写操作、多 Profile 数据管理，以及 NetworkError、Protobuf、页面组件或运行时漂移诊断。
---

# Doudou

始终从用户当前项目运行；当前目录就是数据根。不要切换到 Skill 目录，也不要把 Profile、消息或运行时写进 Skill。

## 入口与门禁

PATH 中存在 `doudou` 时令 `<cli>=doudou`；否则令 `<cli>=npx --yes @follenfang/doudou`。只使用 `<cli>`；`scripts/doudou.py` 是 npm launcher 的内部 bootstrap，不是用户入口。

每个任务按顺序执行：

1. 运行 `<cli> doctor`。
2. 返回 `setup_required` 时运行 `<cli> setup`，然后重跑 doctor；只有 `status=ok` 才继续。
3. 运行 `<cli> profile list`。优先复用本轮已选 Profile；否则在 `login_status=valid` 且锁空闲的 Profile 中选 `last_login_verified_at` 最新者。没有 Profile 时创建并登录。
4. 业务命令自行完成 readiness 检查；不要额外启动第二个 broker、浏览器或独立 warmup。

环境、下载、broker、登录状态和错误契约见 [runtime.md](references/runtime.md)。

版本与升级：`<cli> --version` 不触发 setup；`<cli> update --check` 只读检查；`<cli> update` 只升级现有全局 npm 安装。卸载使用 `<cli> uninstall`，保留调用项目的 `profile/` 和 `message/`。

## 输出与数据

- stdout 是 NDJSON。先捕获并逐行解析，再向用户投影所需字段；不要原样回显完整 Profile、conversation、message 或 diagnostic 对象。
- 默认只报告状态、计数、分页、错误码、匿名化目标槽位和产物路径。除非用户明确要求具体记录，不展示 Cookie、token、`sec_uid`、消息正文或完整成员列表。
- Profile 和业务记录只写入 `profile/<name>/` 与 `message/profiles/<name>/`；这些目录、本地 `analyze/`、浏览器、venv 和登录态不得进入 Git、npm 包或 Skill。

## 核心约束

- 读取使用结构化 HTTP/Protobuf；不要遍历聊天正文 DOM、React/Fiber 或任意对象图。
- 消息、会话成员、进群事件、引用关系、视频作者和评论用户统一保存 `uid/sec_uid/unique_id/nickname/remark_name/display_name`；缺失值保持 null，不用昵称冒充抖音号。
- 命令和参数以当前 [cli.md](references/cli.md) 与 `<cli> <command> -h` 为准，不从旧 Skill 猜测参数。`messages collect` 每次只接受一个 `--conversation`。
- 写入先生成 plan，展示精确预览并取得明确确认；确认后只执行返回的 plan ID 一次。`WRITE_RESULT_UNKNOWN` 先只读核对，不自动重试。
- 写控件必须唯一、可见、启用并满足局部几何契约；否则返回稳定错误，不猜测点击。
- 保留真实错误分类。不要把参数错误、Profile schema 漂移、登录/验证、缓存 lookup 或页面 `NetworkError` 混写成协议失败。

## 功能路由

按任务读取以下参考，不要一次加载全部：

- 命令参数和常见流程：[cli.md](references/cli.md)
- 真实读取、群聊 301、NetworkError 或入口漂移诊断：[diagnostics.md](references/diagnostics.md)
- setup、broker、读取/写入和稳定错误：[runtime.md](references/runtime.md)
- Profile 与生产记录结构：[layout.md](references/layout.md)
- IM Protobuf 字段：[protobuf-message-fields.md](references/protobuf-message-fields.md)
