---
name: doudou
description: 使用项目本地 Camoufox Profile 读取抖音群聊、私聊、图片、系统事件、用户视频和评论，并通过受控页面动作发送、回复、图片、点赞及评论。用于首次安装环境、登录与验证码接管、消息或视频采集、写操作确认、页面/协议诊断，以及多 Profile 项目数据管理。
---

# Doudou

从用户当前项目运行本 Skill。不要切换到 Skill 目录；当前目录是项目数据根。

## 初始化

已通过 npm 安装时，将 `<cli>` 解释为：

```text
doudou
```

PATH 中没有 `doudou` 时，将 `<cli>` 解释为 `npx --yes @follenfang/doudou`。生产 setup 不调用用户电脑上的 Python；无论使用哪个入口，都保持当前目录为用户项目，不能切换到 Skill 目录。

每次任务开始时执行一次环境门禁：

1. 运行 `<cli> doctor`。npm 启动器会先轻量校验并按需原子安装/升级当前版本 Skill；该检查默认不启动浏览器。
2. 若返回 `setup_required`，直接运行 `<cli> setup`，无需再次询问；setup 下载并校验 Doudou 专用 Python 3.12.13，在 `${DOUDOU_HOME:-~/.doudou}` 创建版本化 venv、安装锁定依赖、下载并真实验证 Doudou 专用 Camoufox，完成后重跑 `<cli> doctor`。首次业务命令也会自动完成这一步。
3. 只有 doctor 返回 `status=ok` 才运行业务命令。
4. 运行 `<cli> profile list`，解析用户指定的 Profile；没有 Profile 时创建并登录。

setup 由 npm/Node 启动器下载 `runtime-manifest.json` 固定的 Python 和 Camoufox 资产，支持单实例等待、实时进度、停滞/总超时、断点续传、SHA-256 校验和原子切换。Skill 包不包含 Python、venv、Python 包或浏览器二进制。不要把这些文件安装到 Skill 目录或用户项目。

版本与升级：`<cli> -v`、`<cli> -version`、`<cli> --version` 均不触发 setup；`<cli> update --check` 只读检查，`<cli> update` 只升级现有全局 npm 安装。npx 运行时只报告更新状态。

## 卸载

npm 安装版使用 `doudou uninstall`：它删除全部受管 Skill、整个 `${DOUDOU_HOME:-~/.doudou}`，再删除 npm 包自身；保留项目 `profile/`、`message/`。不要用 Python Skill 入口或直接 `npm uninstall` 代替。

## 登录与接管

创建或登录 Profile：

```text
<cli> profile create --profile <name>
<cli> profile login --profile <name>
```

扫码、快捷登录短信、普通短信、滑块、Passkey 或其他人工验证出现时，由 broker 先关闭后台 context，再用同一 Profile 启动可见有头浏览器让用户接管。不要另建 Profile 或并行启动第二个 broker。按以下错误分类处理：

- `LOGIN_REQUIRED`：页面仍显示登录入口或二维码，执行登录。
- `VERIFICATION_REQUIRED`：页面仍显示短信、滑块或其他验证组件，等待用户完成。
- `IM_CAPTURE_MISSING`：账号已登录，但当前 broker 错过 IM 初始化请求；保持登录状态，刷新会话初始化，不重新登录。
- `IM_INITIALIZATION_MISSING`：账号已登录，但 IM 初始化响应不可用；保持登录状态并诊断初始化链路。
- `WRITE_VERIFICATION_REQUIRED`：写操作触发验证；先只读检查结果，不重发。

broker 通过浏览器网络事件异步重算登录状态。会话列表或有效 IM 会话证明账号已登录后，应立即关闭可见有头 context：Windows 恢复真无头，Linux 回到 Xvfb；Cookie 名称只用于诊断，不能单独决定登录状态。业务读取随后失败时也要保留真实错误分类，不能降级为重新登录。

## 执行规则

- 读取走页面上下文 HTTP/Protobuf，不遍历聊天消息 DOM、React/Fiber 或任意对象图。
- 消息、会话成员、进群事件、引用关系、视频作者和评论用户统一保存 `uid/sec_uid/unique_id/nickname/remark_name/display_name`；缺失值保持 null，不用昵称冒充抖音号。
- 写入先生成 plan，向用户展示计划并取得明确确认；确认后仅以 plan ID 执行一次。
- 写控件按稳定语义、局部组件、唯一可见/启用状态和几何关系定位；非唯一时返回 `PAGE_ADAPTER_DRIFTED`，不猜测点击。
- 写入必须验证唯一网络回执；只读协议复查是补充证据。评论页面回执已确认成功时，慢速或缺失的协议复查不得覆盖成功；`WRITE_RESULT_UNKNOWN` 不自动重试。
- 批量发送严格串行，相邻尝试至少间隔 5 秒；首个失败后停止。
- broker 复用同一 Profile，空闲 20 分钟退出。Windows 日常使用真无头，只有人工认证时切换可见有头；Linux 保留 Xvfb 虚拟有头。
- stdout 是 NDJSON。保留稳定错误码和 details，不用页面文案替代协议结论。

## 功能路由

按任务读取以下参考，不要一次加载全部：

- 命令参数和常见流程：[cli.md](references/cli.md)
- setup、broker、读取/写入及错误契约：[runtime.md](references/runtime.md)
- Profile 与生产记录结构：[layout.md](references/layout.md)
- IM Protobuf 字段：[protobuf-message-fields.md](references/protobuf-message-fields.md)

Profile 和业务记录始终写入调用项目：

```text
profile/<name>/
message/profiles/<name>/
```

不要在 Skill 包内创建 Profile、消息、Cookie、登录态、venv、浏览器或 `analyze/`。
