# Runtime contracts

## Setup

`doctor` 检查专用 Python、版本化 venv、Camoufox 包、指定浏览器版本、setup owner/下载 partial、平台浏览器模式和项目目录写权限。默认不启动浏览器；只有排查浏览器启动时才使用 `doctor --launch-probe`。Windows probe 使用真无头，Linux probe 使用 Xvfb。`setup` 会执行真实启动验证。首次环境不存在时返回 `setup_required`。

`setup` 执行以下步骤：

1. Node 下载并校验固定的 python-build-standalone CPython 3.12.13。
2. 在隔离临时 runtime 创建 `python/` 和 `venv/`，使用 `pip install --require-hashes` 安装依赖并验证关键 imports。
3. 受控下载器以 `.part` 下载固定 Camoufox 资产，实时报告字节/速度，支持 Range、15 秒连接超时、90 秒停滞超时和 45 分钟总时限。
4. 校验大小、SHA-256、浏览器元数据并真实启动；只复用 `DOUDOU_HOME/browsers/` 内的有效专用缓存。
5. 最终目录再次启动专用 Python 后原子切换 `current.json`；失败时保留旧 runtime。

同一用户同时只有一个 setup owner。第二个调用显示 owner 阶段并等待同一结果；取消或失败会回收子进程树，未验证临时 venv 永不复用。下载错误区分 DNS、代理、TLS、403/404、连接/停滞/总超时、磁盘、权限和校验失败。

Windows 就绪检测固定检查 runtime 内的 `venv/Scripts/python.exe`，Linux 检查 `venv/bin/python`。已生成且验证通过的 `current.json` 会被后续命令直接复用，不会因平台路径误判而重复 setup。

Windows 与 Linux 默认都使用 `~/.doudou/`，可由 `DOUDOU_HOME` 覆盖。`runtimes/` 保存 Python 和 venv，`browsers/` 保存 Doudou 专用 Camoufox，`downloads/` 与根目录状态文件保存 setup 进度。Skill 和调用项目均不保存依赖或浏览器二进制。

## Broker

每个项目和 Profile 仅启动一个 broker。首条业务命令执行一次 `ensure_ready`，后续命令只做健康检查。不要设计独立 warmup 命令。

Windows 日常 broker 使用真无头；Linux 日常 broker 使用 Xvfb 虚拟有头。登录或验证需要人工操作时，当前 broker 先安全关闭后台 context，再用同一 `browser-data` 启动可见有头 context；完成后关闭可见 context 并恢复平台后台模式。不要创建新 broker、临时 Profile 或并行占用同一 Profile。

登录状态、验证状态和 IM 初始化状态是三个独立信号：有效 IM 会话或 `/chat` 已加载的会话列表可以证明账号已登录；二维码、短信、滑块、Passkey 等可见组件决定是否需要人工接管；init request/response 只决定协议读取是否就绪。Cookie 名称和数量仅用于诊断，不能作为固定登录白名单。

broker 在相关网络响应到达时异步触发状态重算。登录或验证完成后，一旦账号证据有效且验证组件消失，就记录 Profile 为 `valid`、关闭可见 context 并恢复后台 context，不等待业务命令轮询。即使后续 IM 捕获或业务读取失败，也要结束已经完成的人工接管，并保留原始错误分类。

## Reads

- 会话和消息使用 IM Protobuf；图片下载后验证 AES-GCM、MD5、大小和格式。
- `member_joined` 等系统事件必须结构化；未知 Protobuf 字段写入 inventory。
- 视频列表、详情、评论和回复使用结构化响应；根评论和回复以协议 `cursor/has_more` 为完成条件，不以 DOM 行数或滚动位置判断全量完成，也不复用过期签名 URL。
- HTTP 读取先记录状态、Content-Type、Retry-After、字节数和 SHA-256，再由 Python 解析。错误记录不保存响应正文。
- 评论分页逐页保存本轮 checkpoint，并按 CID 去重；中断后从失败游标继续。同一 Profile 重采同一视频成功时，历史正式评论与本轮结果按 CID 增量合并：有效新值更新旧值，缺失、null 或空文本保留旧值，`0` 和 `false` 仍是有效新值。
- 评论页面定位优先使用当前区域唯一可滚动的 `.comment-mainContent`；滚动或展开回复动作使用有界重试，失败时不得把已渲染部分误报为完整评论集合。

## Writes

协议负责解析稳定 ID、生成 plan 和验证回执；页面 Component Adapter 负责最终一次正常动作。禁止直接重放写请求。

文字、引用、图片、聊天点赞、视频评论、回复评论和评论点赞都必须满足：

1. plan 与确认参数完全一致。
2. 页面目标和当前状态与 plan 一致。
3. 写控件唯一、可见、启用且满足局部几何契约。
4. 只执行一次页面动作。
5. 捕获唯一写请求和服务端响应。
6. 使用只读协议补充复查消息 ID、引用、CID、父 CID、正文或点赞状态。评论创建/回复的页面成功回执先落盘，复查不得同步全量分页或覆盖已确认成功。

## Stable errors

- `RUNTIME_SETUP_REQUIRED`：运行 setup。
- `DEPENDENCY_INSTALL_FAILED`：锁定依赖安装失败；临时 runtime 不会激活。
- `DOWNLOAD_STALLED`：下载 90 秒没有新字节；再次 setup 会续传有效 partial。
- `DOWNLOAD_CHECKSUM_MISMATCH`：资产哈希不符；损坏文件被隔离且不会激活。
- `RUNTIME_RELOCATION_FAILED`：最终 runtime 中的专用 Python 启动失败，current pointer 保持原值。
- `LOGIN_REQUIRED`：页面仍显示登录入口或二维码。
- `VERIFICATION_REQUIRED`：页面仍显示短信、滑块或其他验证组件。
- `IM_CAPTURE_MISSING`：账号已登录，但当前 broker 未捕获 IM 初始化请求；不要重新登录。
- `IM_INITIALIZATION_MISSING`：账号已登录，但 IM 初始化没有可用 Protobuf 响应；不要重新登录。
- `WRITE_VERIFICATION_REQUIRED`：写操作触发验证；不重放。
- `PAGE_ADAPTER_DRIFTED`：页面组件契约变化；不猜测点击。
- `PROTOCOL_SCHEMA_DRIFTED`：接口结构或字段别名变化。
- `RATE_LIMITED`：遵守 Retry-After，停止当前分页。
- `UPSTREAM_UNAVAILABLE`：仅允许有限重试一次。
- `WRITE_RESULT_UNKNOWN`：可能已经写入；先只读核对，不重发。
