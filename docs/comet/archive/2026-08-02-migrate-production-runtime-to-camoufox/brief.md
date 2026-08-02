# Outcome

把 `doudou` 的生产浏览器运行时从 Node.js Playwright Chromium 迁移到官方 Camoufox/Python，并由项目 Profile 级常驻 broker 复用同一个后台有头浏览器；在保留现有聊天能力的同时，补齐结构化系统消息、按用户发现视频、视频详情与评论树采集，以及聊天和视频互动写操作。

# Scope

- 分发版生产 CLI 改为 Python 运行时，锁定 `camoufox==0.5.4` 与官方浏览器 `152.0.4-beta.28`。
- `doctor`/`setup` 检测并安装受支持的 Python、锁定依赖和对应 Camoufox 官方浏览器，并做真实启动探测。
- 有头登录、人工验证、协议 capture、会话初始化、会话发现、消息拉取、发送、回复和 page doctor，共用 `profile/<name>/browser-data/`、Profile 锁、浏览器版本、指纹配置和一个 Profile 级 Python broker。
- CLI 命令通过本机 IPC 连接 broker，不为每条命令重新启动浏览器。broker 串行执行同一 Profile 的命令；没有活动或排队命令满 20 分钟后关闭浏览器、释放锁并退出。
- 业务 broker 在 Windows 默认运行隐藏有头 Camoufox，在 Linux 默认运行 Xvfb 虚拟有头 Camoufox；不再把真实 headless 作为写操作默认模式。
- Windows 上启动 CLI、broker 和 Camoufox 时不得弹出额外黑色命令行窗口；broker 全生命周期只把 `MozillaWindowClass` 主窗口置为透明后台窗口，浏览器启动、页面导航和空闲等待均不得闪现界面。`MozillaBatteryClass`、`MozillaHiddenWindowClass` 和输入法等辅助窗口不得被显示。只有确认需要扫码、短信、滑块或其他人工验证时才恢复主窗口显示。
- Windows 后台状态还必须从任务栏和 Alt+Tab 隐藏：保存主窗口原始 extended style，设置 `WS_EX_TOOLWINDOW`、清除 `WS_EX_APPWINDOW` 并移除任务栏登记；人工接管时恢复原始 style、alpha 255 和任务栏登记，验证完成后再次隐藏。不得用移到屏幕外代替。
- broker 必须识别扫码登录、页面“快捷登录”后触发的手机验证码、普通短信验证码、滑块和通用安全验证。命中后立即把同一 Profile、同一 browser context 的现有窗口呈现给用户接管，不新开临时浏览器，不因已出现登录 Cookie 就提前关闭；只有页面验证消失、账户 Cookie 有效且 `/chat` IM 初始化成功后才恢复后台模式。
- 只读协议返回 JSON `decision=KICK` 但页面没有真实可见的短信、滑块、扫码或通用验证控件时，只返回脱敏的 `VERIFICATION_REQUIRED`，保持浏览器隐藏，不等待用户、不重试原命令，也不构造评论或消息探针。只有同一 broker 页面实际出现验证控件时才显示窗口；短信面板出现后自动点击一次获取验证码，用户完成后自动确认面板消失、Cookie 和 IM 初始化有效、隐藏窗口，并从检查点重试原只读命令一次。
- 保持从任意用户项目运行、默认 data root 为当前目录、Profile 与生产记录只写入调用项目。
- broker 首次业务命令按需执行一次 `ensure_ready`，后续命令只做轻量健康检查并复用当前页面、IM 上下文和会话索引；不再要求用户运行独立 `warmup` 前置命令。会话与成员索引缓存 2 小时，过期刷新索引时不得重启浏览器。
- 保持群聊/私聊 Protobuf 发现、未知字段 inventory、图片下载解密、协议目标解析和发送计划、确认计划、batch 串行 5 秒间隔、回执与验证记录、page doctor 和 protocol capture。
- 消息读取必须结构化保存群成员加入等系统事件。已验证的 `message_type=1001`、`aweType=100140` 事件必须区分主页/来源用户、新成员、加入方式、本地化模板、历史消息权限和相关链接；未知系统事件保留原始结构与 field inventory。
- 新增内容读取入口：以用户可见的抖音号 `unique_id`（例如 `Rurutia.wow`）为主要输入，也接受主页 URL、数字 `uid` 或 URL 中的 `sec_uid`；先解析并保存四者与昵称的身份映射，再通过 `sec_uid` 协议获取视频列表。按视频 ID 拉取 SSR 详情、封面、一级评论、回复和多级父引用关系。
- 评论分页必须按 HTTP、Content-Type、响应长度/哈希、登录状态、验证界面和 JSON 业务状态分层分类；逐页保存根评论游标、已完成回复根 ID 与下一回复目标，验证、限流或网关中断后从检查点恢复，不重新拉取已完成页。
- 上述响应边界适用于全部页面上下文读协议，而不只评论：页面只返回状态、Content-Type、Retry-After 与原始文本，Python 统一分类并按领域给出稳定错误码；协议端点、查询字段、响应字段别名和必需结构集中在版本化 Protocol Adapter，结构漂移返回 `PROTOCOL_SCHEMA_DRIFTED`，不得退化成 broker 内部错误。
- 新增聊天点赞，以及视频发表评论、回复评论、评论点赞。所有写操作均由后台有头 Camoufox 执行页面正常操作；协议只负责目标解析、计划、请求监听和服务端结果校验，不直接提交写请求。
- 八类页面写操作统一采用 Page Component Object：聊天输入框、消息项、图片预览、视频评论输入框和评论项分别由独立 Component Adapter 管理语义定位、组件内局部结构定位及唯一可见/启用/相对位置/操作状态校验。业务模块不得直接保存 CSS、DOM 下标、固定坐标、React/Fiber 或全局页面文本；三层定位仍不唯一时必须 `PAGE_ADAPTER_DRIFTED` 并在点击前停止。
- 所有聊天、视频、评论、媒体、计划、写回执、诊断和运行记录必须位于 `message/profiles/<profile>/`，支持多个 Profile 并严格隔离；共享 venv 和浏览器缓存不得包含任何业务数据。
- Python 正式实现按 runtime、protocol、models、reads、writes、ui、storage 和 feature adapters 分层，新增功能不得继续堆入单一命令或浏览器模块。
- 最终发送回归页面可信发送边界：broker 的后台有头浏览器在目标会话中使用真实正文、真实 @ 成员、真实引用目标或真实图片，执行一次正常页面发送；CLI 监听并校验 SDK 请求和服务端回执，不替换、重放或二次提交发送请求。
- 重写 Skill、Python CLI、README、依赖锁、环境说明、测试和分发清单；删除正式 Node.js 源码、npm 元数据和 Playwright Chromium 生产入口，保留 `analyze/` 历史研究证据。

# Non-goals

- `camoufox-reverse-mcp`、Babel、mitmproxy、WABT 和 `hello_js_reverse_skill` 不进入普通用户运行依赖或交付包；它们只用于维护者调研。
- 不在迁移中改变现有 Protobuf 业务字段语义、会话分类规则、消息目录结构或发送确认规则。
- 不使用全站 DOM 抓取代替协议读取；视频详情允许从 HTTP HTML 的 `#RENDER_DATA -> app.videoDetail` 提取结构化 SSR 数据。
- 不使用 `protocol-bootstrap-*` 或其他临时消息作为发送内容；不遍历 React/Fiber、消息 DOM 或任意页面对象图，不把 DOM 可见文本当作发送成功证据。
- 不转换或合并 Chromium 与 Camoufox 的浏览器内部数据库。
- 不把开发者 Profile、聊天记录或 `analyze/` 打包进 Skill。

# Acceptance examples

- 空项目运行 `doctor` 时可识别 Python、`camoufox==0.5.4`、官方浏览器 `152.0.4-beta.28`、Windows 隐藏有头或 Linux Xvfb 虚拟有头启动能力和项目写权限；缺失时 `setup` 给出可重试步骤，完成后 doctor 通过。
- 新 Camoufox Profile 有头扫码登录后，后台 broker 使用同一 `browser-data/` 和指纹身份成功完成 IM 初始化；进程退出、跨日或机器重启不会生成新身份。
- 同一 Profile 连续执行拉取和写入时只启动一个 broker/浏览器并只完成一次会话初始化；后续命令不重复初始化。最后一条命令完成后 19 分钟仍保持可复用，满 20 分钟无活动或排队命令时自动退出；下次命令重新启动 broker 并初始化一次。
- 扫码、快捷登录短信、普通短信或滑块出现时，CLI 返回具体验证类型与 `profile login --profile <name>` 恢复命令，同时当前 broker 窗口已经可见。用户完成验证后，只有 Cookie 与 IM 初始化同时通过才报告成功并隐藏窗口；未完成时窗口在登录命令的等待期限内保持可接管。
- 群聊与私聊仍按 Protobuf `conversation_type` 发现；消息拉取保留未知字段 inventory，并把图片解密、校验后写入原生产目录。
- 进群事件落盘为可查询的 `system_event.kind=member_joined`，可直接读取新成员、来源用户、加入方式和历史消息权限；模板和未知字段同时保留。
- 给定唯一抖音号或主页，系统可协议发现用户视频列表，并按视频 ID 保存元信息、hashtag、封面、统计、根评论、回复和多级父引用；昵称不作为唯一键，抖音号解析存在冲突或未获平台精确确认时只返回候选，不猜测。
- 文本、引用回复、群聊 @ 和图片由协议数据确定目标和发送计划，再由 `/chat` 页面执行一次真实发送；监听到的目标会话、消息类型、正文、引用、@ 或图片必须与确认计划一致，服务端回执通过后才报告成功。
- Windows broker 后台运行时屏幕、任务栏和 Alt+Tab 均无 Camoufox；显示人工验证时同一主窗口恢复正常可见和任务栏入口，验证结束后自动恢复三项隐藏状态。
- 聊天消息点赞、视频发表评论、回复评论和评论点赞均在 broker 的后台有头页面执行一次正常操作，并通过对应网络请求、业务状态和目标 ID 验证；已处于目标状态时不得反向取消。
- 两个 Profile 拉取同一会话或视频时，各自只写入自己的 `message/profiles/<profile>/`；任一 Profile 的锁、缓存、索引、媒体、计划和回执不得被另一个 Profile 复用。
- batch 确认后复用一个业务会话，严格按计划串行发送，成功项之间至少间隔 5 秒；失败后停止剩余项并分别记录 sent、failed、skipped。
- 从 `~/.agents/skills/doudou/` 调用时，Profile、消息、会话索引、回执和诊断只出现在调用项目；Skill 包不包含运行数据、浏览器二进制或研发工具链。

# Constraints and invariants

- 生产 CLI 直接调用 Python Camoufox，不通过 MCP 启动浏览器。
- 交互可见与后台有头模式必须使用完全相同的 Camoufox 版本、Profile 目录和持久指纹配置。
- Profile 锁和浏览器会话由 broker 持有；CLI 客户端不得另取浏览器锁、额外启动初始化浏览器或把 `warmup` 作为业务命令前置步骤。
- `ensure_ready` 只在 broker 当前浏览器会话尚未初始化或健康检查确认上下文失效时运行；会话与成员索引的 2 小时缓存只用于目标解析和确认计划。实际拉取或写入仍要在当前业务会话验证 Cookie、IM 初始化和协议上下文。
- 写操作必须先预览再使用未漂移 plan ID 确认；自动化验证没有明确真实目标与正文时不得线上发送。
- 所有网络与运行证据必须脱敏；动态签名、Cookie、上传临时凭证和完整签名 URL 不落盘。
- 前端 `data-e2e`、aria、文本和 CSS 选择器只属于版本化 UI 适配器，不是业务数据契约。协议字段、SSR/CSR 结构化响应和网络回执是主契约；page doctor 检出选择器基线漂移时必须阻止相关写操作并给出稳定诊断，禁止盲点或深遍历 React/Fiber。
- 领域层只保存 conversation ID、message ID、uid/sec_uid、aweme ID 和 comment ID 等服务端稳定 ID；不得保存 React key、DOM index、bundle 内变量名或 CSS class 作为跨命令目标。单个 UI adapter 漂移只能阻止依赖它的写能力，不得连带中断不依赖该控件的消息、视频或评论协议读取。
- 现有 `analyze/` 研究证据保持原样，Node 版历史证据不做机械改写。

# Decisions

- Camoufox/Python 是分发版唯一生产浏览器运行时，不保留 Playwright Chromium 生产 fallback。
- 锁定 `camoufox==0.5.4` 与官方浏览器 `152.0.4-beta.28`，版本不匹配时先修复环境，不静默换浏览器。
- `camoufox-reverse-mcp` 仅用于研发抓包、Hook、JS/WASM 和协议验证，不是运行时依赖。
- 页面最终发送是正式边界，不是失败 fallback；消息读取、目标解析、成员解析、确认计划、图片下载和回执判断继续使用结构化协议数据。
- 统一读写边界为：聊天消息、系统事件、图片、用户视频、视频详情、评论和回复优先走页面上下文协议；聊天发送/回复/@/图片/点赞及视频评论/回复/评论点赞走 broker 后台有头页面真实操作。
- broker 按规范化 data root 与 Profile 唯一；命令串行执行，空闲计时从最后一条命令完成且队列为空时开始，活动命令不受 20 分钟空闲关闭影响。
- 持久 Profile 启动时禁用崩溃会话恢复，并只保留一个受控业务页面；历史 `Battery Watcher`、旧视频页或其他恢复页不得成为额外业务窗口。用户关闭最后一个浏览器窗口后，下条命令重建同一 Profile 的 browser context。
- Windows 隐藏实现使用窗口创建事件前置处理：主窗口保留 `WS_EX_LAYERED` 与 alpha 0，同时设置 `WS_EX_TOOLWINDOW`、清除 `WS_EX_APPWINDOW` 并在需要时调用任务栏 API删除登记；接管和重新隐藏必须双向恢复且留下真实窗口状态证据。
- 用户已确认本次扩大后的完整 Change 范围，无需为任务栏隐藏与 Component Adapter 变更再次询问确认。
- broker 意外断开时只读命令可重新连接并重试一次；已经开始的写命令若回执不明确必须返回不可验证结果，不得自动重放。
- `KICK` 是服务端写验证受限信号，不等于页面已经出现验证码。禁止把 KICK 响应回放给无关页面请求、禁止用本地探针伪造 Verify Center 状态，也不得为触发验证制造公开评论或消息。
- 读链路遇到登录/验证网关时返回 `LOGIN_REQUIRED` 或 `VERIFICATION_REQUIRED`；只有页面真实验证控件可见时才显示现有窗口。页面写请求返回 JSON `KICK` 或发送后出现验证界面时返回 `WRITE_VERIFICATION_REQUIRED`，保留 plan/request 证据并禁止自动重发。
- 发送前必须先选中并校验目标会话，再执行一次真实 UI 操作；文字填写真实正文，@ 选择协议解析出的唯一成员，图片选择真实文件并点击预览发送，回复定位协议消息 ID 对应的页面消息后执行引用发送。监听只用于验证，不修改或重放请求。
- 现有业务命令、NDJSON 输出、项目本地 `profile/` 与 `message/` 边界继续保留。
- Camoufox 与现有 Chromium browser-data 不兼容，禁止直接拿旧目录启动或原地转换。
- 第一次遇到同名旧 Chromium Profile 时，CLI 自动把整个旧 Profile 原子改名为 `profile/<name>__chromium-backup-<UTC时间>/`，再以原 Profile 名创建全新 Camoufox Profile 并要求有头重新登录；既有 `message/profiles/<name>/` 继续沿用。
- 备份目标已存在、原子改名失败或旧 Profile 无法明确识别时必须停止并报告，不得覆盖、删除或部分迁移旧数据。
- 用户直接通过 `python <skill>/bin/doudou.py ...` 运行生产 CLI；不保留 Node.js 转调入口，Node.js、npm 和 Playwright 不再是普通用户依赖。
- `setup` 在用户级 doudou 缓存中创建一套锁定的 Python 运行环境并安装官方 Camoufox 浏览器，供该用户的所有业务项目共用；不得把依赖环境或浏览器写入 Skill 或业务项目。
- `profile/`、`message/`、Profile 锁、会话索引、消息、媒体、发送回执和诊断仍按 data root 隔离在当前业务项目中，不因共享运行环境而共享用户数据。
- 运行环境升级先在临时新目录完成安装和真实 doctor 验证，再原子切换为当前版本；确认新版本可用且旧环境无人占用后删除旧环境。安装、验证或切换失败时继续保留并使用旧环境。
- 分发包只交付 Python 源码、`pyproject.toml`、带哈希的锁定依赖、Camoufox/浏览器版本清单和 setup/doctor 脚本，不分发不可移植的 venv 本体。
- 用户首次 setup 在操作系统用户级缓存创建共享 venv；Python CLI bootstrap 自动转入该 venv，依赖安装和浏览器下载完成并验证后才恢复业务命令。
- rollback 记录只覆盖程序环境升级和旧 Chromium Profile 目录迁移，证明失败时可恢复原状态；消息发送继续只保存请求、响应、receipt 和 verification。
- Build 必须先在 Git 忽略的 `analyze/` 使用隔离 Camoufox 环境和实验 Profile 完成真实验证；在同 Profile 人工可见/后台有头复用、broker IM 初始化和真实页面发送链路闭合前，不删除正式 Node.js CLI。
- Analyze 门槛还必须覆盖：系统消息结构化样本、用户名到视频列表、视频详情/封面、完整评论与回复分页、聊天点赞、视频发表评论、回复评论、评论点赞，以及至少两个隔离 Profile 的目录和锁验证。每项写实验只执行一次并保存脱敏网络回执。

# Open questions

- None.

# Verification expectations

- 运行 Python 单元、fixture、CLI 契约和打包测试，覆盖所有现有业务命令与稳定错误。
- 在干净临时项目完成 doctor → setup → profile create 的真实官方 Camoufox 启动验证。
- 正式代码修改前，先在 `analyze/` 保存 Camoufox 包/浏览器版本、启动参数、同一实验 browser-data 的人工可见与后台有头稳定指纹字段、Cookie/Storage 哈希变化、IM 初始化结果、协议拉取结果，以及协议构建加页面最终发送的请求/回执和受控 DOM 操作计数。
- 复用 `analyze/video-comments/` 已验证的 2,154 条根评论、1,248 条回复和多级父引用结论，迁移前补做 Python/Camoufox 重放；不得为了换语言重新制造或覆盖历史 Node 证据。
- 为每一种页面写操作保存目标 ID、操作前状态、受控交互计数、命中的请求类别、HTTP/业务结果、操作后状态和是否产生重复写入；页面文案不作为成功依据。
- 使用隔离测试 Profile 验证同一 browser-data 的有头登录与后台有头 broker IM 初始化、连续命令不重复初始化、20 分钟空闲退出、2 小时会话索引缓存和 Profile 锁。
- 用 fixture 与真实页面分别验证扫码、快捷登录短信、普通短信、滑块、通用验证和写入 `KICK` 的分类；确认 Windows 显示的是 broker 原窗口，验证未完成时不会因 Cookie 已存在而误报成功或提前隐藏，验证完成后同一会话恢复后台模式。
- 通过 fixtures 回归群聊/私聊发现、未知字段、消息合并、图片解密、文字/@/图片/引用页面操作计划、确认计划、5 秒 batch 和回执判定。
- 经用户明确指定测试目标后，执行一组真实发送回归，记录 HTTP/Protobuf 结果、页面真实操作和无额外消息证据。
- 对分发包做 dry run、凭据扫描，并验证 source/installed Skill 的发布文件哈希一致。
- Archive 前必须完成全量真实业务回归并全部通过：同 Profile 登录/接管/后台恢复、连续 broker 复用、群聊与私聊发现、今日消息与进群系统事件、未知字段、图片下载解密、用户身份/视频/详情/评论读取，以及用户授权测试群中的文字、引用、@、图片、聊天点赞和视频评论/回复/点赞。任何真实项失败、跳过或回执不完整都留在 Build/Verify，不得 Archive。
