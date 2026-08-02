---
name: doudou
description: 使用项目本地 Camoufox Profile 读取抖音群聊、私聊、图片、系统事件、用户视频和评论，并通过受控页面动作发送、回复、@、图片、点赞及评论。用于首次安装环境、登录与验证码接管、消息或视频采集、写操作确认、页面/协议诊断，以及多 Profile 项目数据管理。
---

# Doudou

从用户当前项目运行本 Skill。不要切换到 Skill 目录；当前目录是项目数据根。

## 初始化

将 `<cli>` 解释为：

```text
python <skill-directory>/scripts/doudou.py
```

每次任务开始时执行一次环境门禁：

1. 运行 `<cli> doctor`。该检查默认不启动浏览器；只有诊断 Camoufox 启动本身时才显式使用 `<cli> doctor --launch-probe`。
2. 若返回 `setup_required`，直接运行 `<cli> setup`，无需再次询问；完成后重跑 `<cli> doctor`。
3. 只有 doctor 返回 `status=ok` 才运行业务命令。
4. 运行 `<cli> profile list`，解析用户指定的 Profile；没有 Profile 时创建并登录。

setup 使用系统 Python 创建用户级共享 venv，按 `scripts/requirements.lock` 下载锁定依赖，并下载 `scripts/runtime-manifest.json` 指定的 Camoufox 浏览器。Skill 包不包含 venv、Python 包或浏览器二进制。不要把这些文件安装到 Skill 目录或用户项目。

## 登录与接管

创建或登录 Profile：

```text
<cli> profile create --profile <name>
<cli> profile login --profile <name>
```

扫码、快捷登录短信、普通短信、滑块或写验证出现时，使用 broker 显示的同一窗口让用户接管。不要另开浏览器。按以下错误分类处理：

- `LOGIN_REQUIRED`：登录 Cookie 缺失，执行登录。
- `VERIFICATION_REQUIRED`：登录仍在，但页面或 IM 初始化需要验证。
- `WRITE_VERIFICATION_REQUIRED`：写操作触发验证；先只读检查结果，不重发。

## 执行规则

- 读取走页面上下文 HTTP/Protobuf，不遍历聊天消息 DOM、React/Fiber 或任意对象图。
- 写入先生成 plan，向用户展示计划并取得明确确认；确认后仅以 plan ID 执行一次。
- 写控件按稳定语义、局部组件、唯一可见/启用状态和几何关系定位；非唯一时返回 `PAGE_ADAPTER_DRIFTED`，不猜测点击。
- 写入必须同时验证唯一网络回执和只读结果；`WRITE_RESULT_UNKNOWN` 不自动重试。
- 批量发送严格串行，相邻尝试至少间隔 5 秒；首个失败后停止。
- broker 复用同一 Profile，空闲 20 分钟退出。Windows 后台隐藏有头；Linux 使用虚拟有头。
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
