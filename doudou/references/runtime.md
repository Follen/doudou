# Runtime contracts

## Setup

`doctor` 检查系统 Python、用户级 venv、Camoufox 包、指定浏览器版本、后台窗口模式和项目目录写权限。默认不启动浏览器，避免每次任务额外创建一个短命 Camoufox；只有排查浏览器启动时才使用 `doctor --launch-probe`。`setup` 仍会执行一次真实启动验证。首次环境不存在时返回 `setup_required`。

`setup` 执行以下步骤：

1. 在用户缓存创建临时 venv。
2. 使用 `pip install --require-hashes -r scripts/requirements.lock` 安装依赖。
3. 选择 `scripts/runtime-manifest.json` 中的官方 Camoufox 版本并执行下载。
4. 真实启动一次浏览器探针。
5. 全部通过后原子切换 `current.json`；失败时保留旧运行时并写 rollback 记录。

Windows 运行时位于 `%LOCALAPPDATA%/doudou/runtimes/`；Linux 位于 `${XDG_CACHE_HOME:-~/.cache}/doudou/runtimes/`。Camoufox 浏览器使用其用户级缓存。Skill 和调用项目均不保存依赖或浏览器二进制。

## Broker

每个项目和 Profile 仅启动一个 broker。首条业务命令执行一次 `ensure_ready`，后续命令只做健康检查。不要设计独立 warmup 命令。

登录或验证需要人工操作时显示当前 broker 的同一窗口；完成后自动恢复后台状态。不要关闭当前窗口后再创建新 broker。

## Reads

- 会话和消息使用 IM Protobuf；图片下载后验证 AES-GCM、MD5、大小和格式。
- `member_joined` 等系统事件必须结构化；未知 Protobuf 字段写入 inventory。
- 视频列表、详情、评论和回复使用结构化响应；只用受控滚动触发下一页，不复用过期签名 URL。
- HTTP 读取先记录状态、Content-Type、Retry-After、字节数和 SHA-256，再由 Python 解析。错误记录不保存响应正文。
- 评论分页逐页保存 checkpoint，并按 CID 去重；中断后从失败游标继续。

## Writes

协议负责解析稳定 ID、生成 plan 和验证回执；页面 Component Adapter 负责最终一次正常动作。禁止直接重放写请求。

文字、引用、@、图片、聊天点赞、视频评论、回复评论和评论点赞都必须满足：

1. plan 与确认参数完全一致。
2. 页面目标和当前状态与 plan 一致。
3. 写控件唯一、可见、启用且满足局部几何契约。
4. 只执行一次页面动作。
5. 捕获唯一写请求和服务端响应。
6. 使用只读协议复查消息 ID、引用、@ UID、CID、父 CID、正文或点赞状态。

## Stable errors

- `RUNTIME_SETUP_REQUIRED`：运行 setup。
- `LOGIN_REQUIRED`：登录 Cookie 缺失。
- `VERIFICATION_REQUIRED`：登录仍在，但读取初始化需要验证。
- `WRITE_VERIFICATION_REQUIRED`：写操作触发验证；不重放。
- `PAGE_ADAPTER_DRIFTED`：页面组件契约变化；不猜测点击。
- `PROTOCOL_SCHEMA_DRIFTED`：接口结构或字段别名变化。
- `RATE_LIMITED`：遵守 Retry-After，停止当前分页。
- `UPSTREAM_UNAVAILABLE`：仅允许有限重试一次。
- `WRITE_RESULT_UNKNOWN`：可能已经写入；先只读核对，不重发。
