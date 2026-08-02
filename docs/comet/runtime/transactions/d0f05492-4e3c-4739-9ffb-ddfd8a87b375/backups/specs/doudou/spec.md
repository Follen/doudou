# doudou

## 交付与运行时边界

系统必须以 `doudou` 名称交付标准 Codex Skill 和确定性 Python CLI。用户必须通过 `python <skill>/bin/doudou.py ...` 直接运行；不得保留 Node.js 转调入口。分发版生产浏览器运行时必须直接使用 Python `camoufox==0.5.4` 和官方浏览器 `152.0.4-beta.28`，不得把 Node.js Playwright Chromium 或 `camoufox-reverse-mcp` 作为生产浏览器入口或 fallback。

CLI 必须从调用者当前项目运行。默认 data root 必须是当前工作目录；只有显式 `--data-root` 或 `DOUDOU_HOME` 才能覆盖。用户登录态必须写入 `profile/<profile>/`，生产记录必须写入 `message/profiles/<profile>/`，不得写入已安装 Skill 目录或用户级依赖缓存。

仅存在旧顶层 `Profile/` 时必须一次性迁移为 `profile/`；新旧根目录同时存在且不是同一目录时必须停止并报告冲突。新版只接受 `doudou.*` schema、`doudou` 锁 owner、Python doudou CLI 和 `DOUDOU_HOME`。其他产品 schema 或 owner 必须返回包含文件路径和旧格式说明的稳定错误，不得覆盖、删除或静默接受。

`setup` 必须在操作系统用户级 doudou 缓存中建立共享的锁定 Python 运行环境并安装官方 Camoufox 浏览器，供该用户的多个调用项目复用。Skill 目录与业务项目不得包含虚拟环境、第三方依赖副本或浏览器二进制。共享运行环境只能包含可重新安装的程序依赖和浏览器，不得包含 Profile、Cookie、Storage、消息、媒体、会话索引、发送回执、诊断或业务项目标识。

运行环境升级必须先在同一用户缓存下的临时新目录完成依赖安装、浏览器校验和真实 doctor 启动验证，再原子切换当前环境指针。只有新环境验证通过、切换成功且旧环境没有有效进程占用时，才能删除旧环境。安装、验证或切换失败时必须保留并继续使用旧环境，不得留下指向不完整新环境的当前指针。

setup 必须为环境升级保存脱敏 rollback 记录，包含旧/新运行时版本、阶段、切换状态、验证结论和恢复结果。旧 Chromium Profile 自动备份必须保存项目内迁移记录，包含原路径、备份路径、时间、原子改名结论和新 Profile 创建状态。rollback 不属于消息发送语义；发送链路继续只保存业务 request、response、receipt 和 verification。

Skill 安装目录只包含发布清单声明的代码、指令、参考文档、测试和锁文件，不得包含用户 Profile、消息、发送回执、诊断、`analyze/`、虚拟环境、依赖缓存或浏览器二进制。`camoufox-reverse-mcp`、Babel、mitmproxy、WABT 和研发 Skill 只属于维护者工具链，不是普通用户依赖。

分发包必须包含 Python 源码、`pyproject.toml`、带哈希的锁定依赖、Camoufox/官方浏览器版本清单以及 setup/doctor bootstrap，不得包含开发者机器生成的 venv。CLI bootstrap 必须从系统 Python 启动，定位或创建用户级共享 venv，然后用该 venv 的解释器重新执行真实命令。

`docs/comet/archive/`、`analyze/`、旧发送回执、旧诊断和其他历史证据必须保持创建时的内容、名称和哈希，不参与 Python 迁移的机械改写。运行时残留检查必须把不可变历史路径列为明确排除项。

### 场景：从 Skill 目录外调用

给定 Skill 安装在 `~/.agents/skills/doudou/`，用户在任意空项目运行 doctor，当 CLI 初始化数据根时，只在该项目创建 `profile/` 与 `message/`；Skill 安装目录不出现运行数据。

### 场景：多个项目共用运行环境

给定同一用户在两个项目运行 doudou，两个项目可以复用同一锁定 Python 环境和 Camoufox 浏览器；各自的 `profile/`、`message/`、锁、会话索引、消息、媒体、回执和诊断必须保持项目隔离，用户级缓存中不出现这些业务数据。

### 场景：共享环境升级

给定旧环境可用且需要安装新锁定版本，当 setup 升级时，必须先在新目录完成安装和真实启动验证，再切换当前环境；验证成功且旧环境无人占用后删除旧环境。任一步失败时旧环境和当前指针保持可用，新环境残留必须作为可清理的失败安装报告。

### 场景：打包交付

给定维护者生成分发包，当检查内容与凭据时，包内只有运行代码、Skill 文档、参考资料、测试和锁定依赖定义；不存在 Profile、聊天记录、诊断、研发工具、虚拟环境、浏览器二进制、Cookie、token 或动态签名。

## 环境检测与安装

doctor 必须以机器可读 NDJSON 检测 Python `>=3.10,<4.0`、包管理器、精确 `camoufox==0.5.4`、官方浏览器 `152.0.4-beta.28`、浏览器文件完整性、Windows 隐藏有头或 Linux Xvfb 虚拟有头启动能力、交互登录条件以及用户项目目录写权限。结果必须报告实际解释器、实际包版本、浏览器版本和安装来源。

setup 必须使用 Skill 的锁定 Python 依赖定义在用户级 doudou 缓存建立或更新隔离运行环境，并通过 Camoufox 官方安装机制取得精确浏览器版本；不得依赖用户全局恰好存在的同名包或任意 Firefox/Chromium。安装完成后必须重新 doctor，成功后才能恢复原浏览器任务。

锁定依赖必须可重复安装并校验包哈希。首次 setup 必须按“创建临时 venv → 安装锁定依赖 → 下载并校验官方浏览器 → 验证平台默认后台有头模式 → 切换当前环境”的顺序执行；任一步失败时不得把不完整环境标记为当前。

Node.js、npm、Playwright 和 Playwright-managed Chromium 不得继续作为生产 CLI 的必需环境或兼容入口。若仅维护者调研命令需要额外工具，doctor 必须将其放在单独的研发检查中，不得让普通业务任务失败。

### 场景：首次使用缺少环境

给定用户只有已安装 Skill，没有 Python 依赖或 Camoufox 浏览器，当 doctor 运行时，结果为 `setup_required` 并列出稳定问题码与修复命令；setup 完成后再次 doctor，依赖版本、浏览器版本和真实启动探测全部通过。

### 场景：版本漂移

给定系统发现其他 Camoufox 包或浏览器版本，当业务命令运行时，必须在访问业务页面前报告版本不匹配和修复命令；不得静默使用其他版本、下载最新版或回退到 Chromium。

### 场景：安装失败

给定 Python 缺失、依赖安装失败、浏览器下载或校验失败、目录不可写或浏览器无法启动，当 setup 或 doctor 运行时，结果必须指出失败步骤、稳定错误码和可重试命令，原登录、拉取或发送任务保持未执行。

## Profile、登录、指纹与锁

每个命名 Profile 必须位于项目的 `profile/<profile>/`，包含非敏感元数据、`browser-data/`、持久化指纹配置、broker descriptor 和运行期锁。有头登录、重新登录、人工验证、协议 capture、broker 会话初始化、会话发现、消息拉取、页面发送、回复和 page doctor，必须复用同一个 `browser-data/`、同一 Camoufox 版本和同一指纹身份。

Profile 元数据必须记录运行时类型、锁定 Camoufox 包版本、官方浏览器版本和指纹配置版本。普通跨日、进程退出或机器重启不得生成新 Profile 身份。运行时版本或指纹契约不兼容时必须在业务导航前失败，并提供显式修复流程。

持久指纹配置必须覆盖 Camoufox 对 UA/UA-CH、`navigator.webdriver`、Canvas、WebGL、platform、languages、hardwareConcurrency、deviceMemory、screen、permissions、plugins/MIME、media capabilities 和可归一化 WebRTC 表面的配置。产品不得再用 `about:blank` 采集 Playwright Chromium baseline 或依赖 CDP 临时修补 HeadlessChrome；交互可见与后台有头必须从同一 Camoufox 配置生成一致身份。

登录命令必须打开有头 Camoufox，并仅在账户 Cookie 存在且 IM 初始化成功后报告登录完成。验证码或人工验证必须在同一有头 Profile 中完成。登录状态分类必须区分：账户 Cookie 缺失为 `LOGIN_REQUIRED`；Cookie 存在但 IM 初始化失败或出现 JSON verification gateway 为 `VERIFICATION_REQUIRED`；二者均正常才是 valid。

同一 Profile 同时只允许一个有效 owner。锁必须校验 PID、进程启动身份、data root、Profile 和命令；只有用户明确确认后才能终止完全匹配的 owner，PID 被复用或身份不匹配时不得终止。

现有 Chromium `browser-data/` 不能由 Camoufox 直接启动、原地转换或合并。第一次遇到同名旧 Profile 时，CLI 必须在启动浏览器前把整个旧 Profile 原子改名为 `profile/<name>__chromium-backup-<UTC时间>/`，再以原 Profile 名创建全新 Camoufox Profile 并要求有头重新登录。既有 `message/profiles/<name>/` 必须继续沿用。备份目标已存在、原子改名失败或旧运行时无法明确识别时必须停止并报告，不得覆盖、删除、合并或留下部分迁移结果。

### 场景：同 Profile 交互与后台复用

给定用户用 Camoufox 有头登录成功，当后续后台 broker 启动时，必须使用完全相同的 `profile/<name>/browser-data/`、浏览器版本和指纹配置，并完成 IM 初始化；不得复制 Cookie 到另一个临时 Profile。

### 场景：人工验证恢复

给定后台业务会话返回 `VERIFICATION_REQUIRED`，Skill 让同一 Profile 进入人工可见验证流程；验证通过后只按原参数重试一次，不重新询问已选 Profile、会话、时间范围或正文。

### 场景：旧 Chromium Profile

给定 `profile/<name>/` 标记为旧 Chromium 运行时，当 Camoufox 命令运行时，必须在启动浏览器前把整个目录原子备份为带 UTC 时间的同级目录，以原名称创建 Camoufox Profile，并返回需要重新登录的结果；不得把旧 Cookie/Storage 当作 Camoufox 已登录状态。后续消息读取和发送记录仍使用 `message/profiles/<name>/`。

### 场景：旧 Profile 备份失败

给定备份名称冲突、目录被占用、权限不足或无法证明旧运行时类型，当迁移开始时，CLI 必须在创建新 Profile 前停止并报告原路径、拟备份路径和稳定错误码；原 Profile 与消息目录保持原样。

## Profile Broker 生命周期

每个 broker 必须由规范化后的 data root 与 Profile 名共同唯一标识。同一键最多存在一个 broker、一个 Camoufox persistent context 和一个 Profile 锁 owner；不同 data root 中的同名 Profile 不得共享 broker、浏览器、锁、descriptor、队列或运行数据。

CLI 必须先读取 `profile/<name>/runtime/broker.json`，通过本机 IPC 连接现有 broker；descriptor 不存在、过期或对应进程身份不匹配时，CLI 才能启动新 broker。descriptor 必须原子写入，至少包含 schema、PID、进程启动身份、规范化 data root、Profile、IPC endpoint、随机认证 token 的受限引用、运行模式、浏览器版本、创建时间和最后活动时间。broker 正常退出后必须删除自身 descriptor；CLI 遇到残留 descriptor 时必须验证 PID 与进程启动身份后清理，不得终止无关进程。

IPC 必须只监听本机，不得绑定外部网络接口。每个 broker 启动时生成高熵随机 token，每次请求都必须认证；Windows descriptor 与 token 文件必须限制为当前用户可读，Linux 必须使用用户私有 runtime 目录和 `0600` 权限。请求必须携带唯一 request ID、命令类别、只读或写入语义和参数；响应使用稳定 NDJSON envelope，不得通过 descriptor、日志或错误输出泄露 Cookie、Storage、token、动态签名或上传凭证。

broker 必须严格串行执行同一 Profile 的命令。命令在排队或执行期间不得另启浏览器，不得绕过 broker 直接取得 Profile 锁。空闲计时只在队列为空且最后一条命令完成后开始；默认空闲期限为 20 分钟。排队、运行、等待页面网络回执或等待受控页面动作期间均不算空闲，不得被空闲计时器关闭。满 20 分钟后 broker 必须依次关闭页面和 browser context、释放 Profile 锁、删除 descriptor 并退出。

Windows 业务 broker 默认使用隐藏有头 Camoufox：浏览器保持正常有头渲染与页面生命周期，但窗口不出现在用户桌面。Linux 业务 broker 默认使用 Camoufox `headless='virtual'`，由 Xvfb 提供虚拟显示。扫码登录、页面“快捷登录”后触发的手机验证码、普通短信验证码、滑块或其他人工验证必须把同一 broker 的现有浏览器切换或重新呈现为人工可见模式，并继续使用同一个 `browser-data/`、browser context 与指纹配置；完成后恢复平台默认后台有头模式，不创建临时 Profile。

Windows CLI bootstrap、broker 进程及其辅助进程必须以无控制台窗口方式启动，不得弹出黑色命令行窗口。Windows 浏览器后台监视必须在创建 Camoufox context 之前启动并覆盖 broker 全生命周期，只控制 `MozillaWindowClass` 主窗口并在后台使用 alpha=0 的 layered window；启动、导航、正常读取、写入、健康检查和空闲等待期间不得闪现浏览器。`MozillaBatteryClass`、`MozillaHiddenWindowClass`、IME 等辅助窗口不得被显示或作为人工接管窗口。只有验证状态机确认需要人工接管，或用户显式运行 `profile login`，才恢复主窗口 alpha 并呈现。

持久 Profile 必须禁用崩溃会话恢复。context 启动后只保留一个受控业务页面，关闭其余恢复页；Cookie、Storage 和 IndexedDB 继续保留。用户手动关闭最后一个原生浏览器窗口后，broker 不得对失效 context 调用 `new_page`，下一条命令必须使用同一 Profile 重建 context。

验证检测必须返回稳定的 `verification_kind`，至少区分 `qr_login`、`quick_login_sms`、`sms_code`、`slider` 与 `generic_verification`。账户 Cookie 不存在时返回 `LOGIN_REQUIRED`；Cookie 已存在但验证界面仍可见、IM init 缺失或读取到 JSON 验证网关时返回 `VERIFICATION_REQUIRED`。`decision=KICK` 只表示服务端要求写验证，不得据此伪造页面验证码已出现。不得只因扫码后出现 Cookie、首页头像或登录弹窗消失就报告成功并关闭窗口。

broker 只有在账户确实未登录，或同一页面检测到真实可见验证控件时才呈现现有窗口，并把包含 Profile、验证类型、窗口状态和 `profile login --profile <name>` 恢复命令的结构化错误返回 CLI。`profile login` 必须保持同一 broker 命令处于活动状态并轮询；只有真实验证界面先被观察到、随后消失，且认证 Cookie 有效、`/chat` Protobuf IM 初始化成功后才隐藏窗口并返回 `logged_in`。超时不得关闭仍需用户处理的窗口或把 Profile 标记为有效。

只读请求遇到 JSON `decision=KICK` 且没有可见验证控件时，broker 必须返回脱敏的 `VERIFICATION_REQUIRED`，其中 `takeover.active=false`，并保持浏览器隐藏。该状态不得进入人工等待、不得自动重试原请求、不得构造或提交评论/消息探针，也不得把 KICK 正文保存到内存恢复对象、错误详情、日志、检查点或诊断文件。

同一页面真实显示 Verify Center 短信输入后，broker 必须自动点击一次可见的“获取验证码”或“发送验证码”控件，然后等待用户输入。验证码控件消失后必须自动重新初始化 `/chat`；Cookie 与 IM 同时有效时立即隐藏浏览器，并从已落盘检查点重试原只读命令一次。第二次仍 KICK、超时或恢复结果不明确时返回稳定错误，不得循环触发。写命令触发 KICK 时仍返回 `WRITE_VERIFICATION_REQUIRED`，不得自动重放原写命令。

页面写请求若返回 JSON `decision=KICK`，或页面操作后出现短信、滑块或通用验证，必须返回 `WRITE_VERIFICATION_REQUIRED`，保存已开始写入、禁止自动重放和可用脱敏回执状态。只有真实验证控件可见时才立即呈现同一窗口；KICK 后仍是普通页面时保持隐藏且 `takeover.active=false`。该状态不得归类为 `LOGIN_REQUIRED`，也不得在用户验证后自动再次点击；用户必须先只读复查原写入结果，再创建新的确认计划。

broker 意外断开时，只读命令可在验证原 request 未产生业务写入后重新连接并自动重试一次。写命令一旦进入 `started` 状态，客户端断线、超时或 broker 崩溃且缺少完整服务端回执时，必须返回 `WRITE_RESULT_UNKNOWN`，保存 request ID 和可用证据，并禁止自动重放；用户必须先通过只读协议复查目标状态，再决定是否创建新的写计划。

### 场景：连续命令复用

给定同一 data root 与 Profile 连续运行 collect 和已确认写命令，当第二、第三条命令连接时，三条命令必须由同一 broker PID、同一 persistent context 和同一 Profile 锁串行执行，不出现额外浏览器进程或第二次实时会话初始化。

### 场景：空闲自动退出

给定队列为空且最后一条命令刚完成，当经过 19 分钟时 broker 仍可接受并复用命令；持续无新命令满 20 分钟后，浏览器已关闭、锁已释放、descriptor 已删除且 broker 进程退出。若第 20 分钟前已有命令排队或运行，必须从该命令完成且队列再次为空后重新计时。

### 场景：写入期间连接中断

给定一个已确认写命令已经开始页面操作，当客户端连接中断且最终回执不完整时，重连结果必须为 `WRITE_RESULT_UNKNOWN` 并指向脱敏证据；broker 和 CLI 均不得自动再次点击、再次提交或把该命令当作只读任务重试。

## Broker 会话初始化与会话发现

用户不得把独立 `warmup` 命令作为拉取、发送或回复的前置步骤，正式 CLI 不提供面向用户的 `warmup` 业务命令。broker 收到当前浏览器会话的第一条业务命令时必须执行一次内部 `ensure_ready`：检查环境、Profile、锁、指纹、Cookie 和实时 IM 初始化，解析 init 及相关 IM Protobuf并建立群聊和私聊统一索引。`conversation_type=1` 为私聊，`conversation_type=2` 为群聊。结构化类型缺失时只可使用有界明确页面属性补充，不得依据标题文字分类，不得遍历 React Fiber、DOM 内部对象或任意对象图。

同一 broker/browser context 的后续命令必须先做轻量健康检查；Cookie、页面和 IM 上下文仍有效时直接复用，不得重复 `ensure_ready`。只有 broker 新启动、当前会话尚未初始化，或健康检查确认上下文失效时才重新初始化。会话与成员索引过期只触发同一浏览器会话内的协议刷新，不得重启浏览器。

成功会话索引快照可在同一项目、同一 Profile 下缓存 2 小时，年龄从成功刷新时间计算。缓存只负责会话解析、消息目标解析和确认计划；实际拉取或写入仍必须在其唯一业务浏览器会话中验证 Cookie、IM 初始化和当前协议上下文。CLI 可以提供显式 `session refresh` 与诊断命令，用于强制刷新索引或查看状态，但普通业务命令不依赖用户先调用它们。

索引刷新失败时可以使用不超过 2 小时的最近成功快照，并明确输出 `session_index_source: stale_cache`、完成时间和年龄；超过 2 小时或快照契约不兼容时必须失败，不得消费旧快照。

会话目标必须支持稳定 ID、同一刷新快照编号和用户可读名称。名称匹配使用 Unicode 大小写折叠、合并连续空白并支持包含匹配；多个匹配必须全部返回稳定候选，不得静默优先同名项或猜测。

编号选择只允许引用同一 `refreshed_at` 快照，支持单个编号、逗号列表、范围和“全部”。越界、重复、空选择或快照时间不匹配必须返回明确结果，不得静默改选。收件人解析只覆盖 broker 会话索引中的现有会话，不得使用全站搜索或创建新私聊。

### 场景：2 小时缓存边界

给定一份 119 分钟前成功的兼容会话索引，目标解析可以复用它；给定 121 分钟前快照，broker 必须在当前浏览器会话内刷新索引，失败时不得仅凭该快照形成可执行发送，也不得为刷新索引另启浏览器。

### 场景：连续命令无需预热

给定 broker 已通过 `ensure_ready` 完成 IM 初始化，当连续执行拉取、发送计划和已确认写入时，每条命令只做轻量健康检查并复用同一页面、IM 上下文和会话索引；不得再次执行完整初始化。broker 空闲退出后的下一条命令允许在新 broker 中初始化一次。

### 场景：混合会话

给定 init 同时包含 type 1 与 type 2，会话刷新必须返回私聊和群聊并保持稳定 ID、显示名与 `refreshed_at`；名称是否包含“群”不改变类型。

## 消息拉取、未知字段与图片

系统必须按稳定 conversation ID 和 RFC3339 时间范围，通过 `/chat` 页面上下文的协议请求拉取群聊或私聊消息。用户未给范围时默认使用项目时区当天 `00:00` 到命令执行时刻。消息拉取不得打开会话 DOM、滚动消息列表、读取可见消息文本或遍历 React 对象。

每条规范化消息必须保存稳定消息 ID、conversation ID/kind、发送时间、发送者、消息类型、正文、图片列表和可审计协议元数据。未知 Protobuf 字段必须进入 field inventory 与脱敏样本；已知字段缺失、改型或冲突不得由 DOM 文本静默替代。

系统消息不得只保存为一段显示文字。规范化消息必须可选携带 `system_event`，至少包含稳定事件 kind、协议 message type、awe type、参与用户角色、来源、能力标志、本地化资源、模板动作和原始结构摘要。已验证的 `message_type=1001`、`aweType=100140` 加群事件必须映射为 `member_joined`：`active_users` 为主页或邀请来源用户，`passive_users` 为新成员，模板语义映射加入方式和 `history_visible`，同时保留 UID、sec UID、昵称和原始 locale template。无法识别的系统事件使用 `kind=unknown` 并完整保留脱敏协议结构，不得丢弃。

消息按 `message/profiles/<profile>/conversations/<kind>__<display-name>__<conversation-id>/` 隔离。重复稳定消息 ID 必须增量合并，补齐缺失字段并记录不同非空值冲突；原子写入不得留下半行 JSONL。一次只读运行中单个会话失败不得阻断其他会话，Run 必须记录逐会话新增、重复、冲突、跳过和失败计数。

读取现有 JSONL 时，若最后一行不完整、schema 未知或必填字段缺失，必须报告文件与行号并停止覆盖原文件。不同非空字段冲突必须写入 conflicts，不得静默采用后读值。

图片消息必须从 Protobuf content 取得资源与密钥，下载密文，按协议使用 AES-256-GCM 解密，并校验声明长度、图片格式、MD5 和 SHA-256 后原子写入 `media/YYYY-MM-DD/<message-id>/`。单张图片失败不得留下部分明文、覆盖有效文件或阻断其他消息；后续 collect 必须可重试失败图片。

### 场景：群聊与私聊采集

给定一个群聊和一个私聊及同一时间范围，当 collect 完成时，两者分别写入独立生产目录，未知字段有 inventory，图片已下载解密，Run 报告每个会话的完整统计。

### 场景：成员通过个人主页加入群聊

给定 `message_type=1001`、`aweType=100140` 且包含 active/passive users 与本地化模板的事件，当消息规范化时，结果必须为 `system_event.kind=member_joined`，可直接读取新成员、主页来源用户、加入方式和历史消息可见性；原模板、用户稳定 ID 和未知字段仍可审计。

### 场景：图片校验失败

给定下载失败、GCM tag 无效、长度、格式或 MD5 不匹配，消息记录仍保存且图片项记录稳定失败码和可重试状态；既有有效文件与其他消息保持不变。

## 用户视频、详情与评论读取

系统必须把用户可见的“抖音号”建模为 `unique_id`，把 `/user/<value>` 主页路径中的不透明值建模为 `sec_uid`，并与数字 `uid`、昵称分别保存；四者不得混为一个字段。视频采集主要入口为 `--douyin-id <unique_id>`，同时接受主页 URL、数字 `uid` 或 `sec_uid`。CLI 必须通过页面上下文的用户搜索/用户信息读协议取得平台返回的 `unique_id -> sec_uid -> uid -> nickname` 映射；昵称只用于显示和候选提示，不得作为唯一账号键。抖音号未被平台响应精确确认或出现多个候选时必须返回候选及稳定 ID，不得猜测或静默选择。解析成功后必须使用 `sec_uid` 通过页面上下文读协议获取该用户的视频列表、分页游标和稳定视频 ID。

视频详情允许读取视频页 HTTP HTML 中的 `#RENDER_DATA`，执行 URL 解码和 JSON 解析后读取 `app.videoDetail`；不得依赖页面可见标题、卡片 DOM 或滚动列表文字。规范化详情至少保存视频 ID、作者、标题、描述、hashtag、创建时间、时长、尺寸、统计、封面和播放资源摘要。封面必须下载、校验格式与哈希后原子落盘。

一级评论必须使用 `/aweme/v1/web/comment/list/` 完整分页，回复必须使用 `/aweme/v1/web/comment/list/reply/` 按根评论完整分页。`reply_id` 保存根评论 ID，`reply_to_reply_id` 保存直接父回复 ID；父回复被删除、过滤或未返回时必须保留 ID 并标记 `parent_resolved=false`。展示计数保存为 `reported_reply_count`，实际返回数保存为 `reply_count`，二者不得互相覆盖。

所有页面上下文读协议都必须先读取 HTTP 状态、Content-Type、Retry-After 和响应文本，再由 Python 统一解析，不得在页面中直接调用 `response.json()`。评论错误必须稳定区分：Cookie 缺失为 `LOGIN_REQUIRED`；Cookie 存在且命中验证界面、验证网关或 JSON `decision=KICK` 为 `VERIFICATION_REQUIRED`；仅在真实验证控件可见时显示现有 broker 浏览器；HTTP 429 为 `RATE_LIMITED`；5xx 为 `UPSTREAM_UNAVAILABLE`；空正文为 `COMMENT_RESPONSE_EMPTY`；非 JSON 为 `COMMENT_RESPONSE_INVALID`；有效 JSON 但业务状态非零为 `COMMENT_COLLECT_FAILED`。其他领域使用同一分类顺序和对应领域错误码。证据只保存状态、Content-Type、Retry-After、长度和 SHA-256，不保存响应正文。只有瞬时 5xx 允许原请求有限重试一次；验证、KICK、429、空响应和格式异常不得循环重试。

Web 协议端点、游标参数、响应字段别名和必需结构必须集中在带版本号的 Protocol Adapter 中。业务层只能读取 adapter 输出的领域模型，不得直接依赖 bundle 变量名、React key、DOM index 或未声明的 JSON 字段。服务端成功响应缺少必需字段或字段类型变化时必须返回 `PROTOCOL_SCHEMA_DRIFTED`，只记录 contract 版本、操作、字段别名、响应顶层 key 和实际类型，不记录响应正文。已知 snake_case/camelCase 别名可以在 adapter 中兼容；未知字段继续进入 inventory，禁止运行时猜测或自动改映射。

评论运行必须在每个成功根评论页和每个完成回复线程后原子保存检查点，至少包含根评论下一游标、已完成根页、已完成回复根 ID、下一回复根 ID、去重评论 ID 和运行状态。恢复时必须从检查点继续并合并去重，不能重新请求已完成页；完成后把检查点封存为 completed。页面已内嵌的回复直接规范化，只有 `reported_reply_count` 大于已内嵌数量的根评论才请求更多回复；多个回复线程允许小规模受控并发，但单线程内部游标必须保持顺序。

视频数据必须写入当前 Profile 的 `message/profiles/<profile>/content/users/<user-id>/` 与 `content/videos/<aweme-id>/`。用户目录保存身份、解析候选、视频索引和分页运行；视频目录保存 metadata、封面、评论 JSONL、线程索引、原始分页摘要、冲突和媒体。相同公开内容由不同 Profile 读取时仍分别落入各自 Profile 目录，不建立跨 Profile 共享业务缓存。

### 场景：按用户名发现视频

给定 `--douyin-id Rurutia.wow`，当 collect videos 运行时，系统先形成包含 `unique_id=Rurutia.wow`、对应 `sec_uid`、数字 `uid` 和昵称的身份记录，再通过 `sec_uid` 分页保存视频列表；昵称相同或抖音号未精确确认时只返回候选，不创建可执行采集计划。

### 场景：视频详情与评论树

给定稳定视频 ID，当 collect video 完成时，SSR 详情、封面、根评论和回复分别完成结构化与校验；多级回复保持直接父 ID，缺失父节点显式标记，服务端 `has_more=false` 后停止且不补虚构记录。

## 页面写操作、协议校验与批量

所有页面写操作必须使用独立 Page Component Object。Component Adapter 必须按三层顺序定位：先使用 role、aria-label、data-e2e 或稳定文案形成语义候选；再限定在已确认的输入框、消息项、图片预览、评论输入框或评论项内部做局部结构定位；最后校验候选唯一可见、已启用、位于组件预期区域且操作前状态正确。三层仍不能唯一确定时必须返回 `PAGE_ADAPTER_DRIFTED`，不得使用固定坐标、DOM 下标、React/Fiber、全局文本或自动猜测选择器继续写入。

文字发送、引用回复、群聊 @、图片发送、聊天消息点赞、视频发表评论、回复视频评论和视频评论点赞必须分别通过 `ChatComposerComponent`、`MessageItemComponent`、`ImagePreviewComponent`、`VideoCommentComposerComponent` 或 `CommentItemComponent` 组合完成。业务 feature 与 write service 只能传入协议稳定 ID 和确认计划，不得直接持有页面选择器。

普通文本、引用回复、群聊 @ 和图片发送必须先从结构化协议数据确定唯一会话、成员或目标消息并形成发送计划。最终发送必须由当前 `/chat` 页面的一次正常操作完成，使同一 Camoufox Profile 的 action report、identity security、SDK sequence、设备状态和动态请求参数生效。CLI 必须监听 SDK 请求和服务端回执用于校验，不得替换、重放、中止后重发或另行 `fetch` 发送请求。

聊天消息点赞必须先从协议消息记录解析唯一 conversation ID 和 message ID，再在 broker 的后台有头 `/chat` 页面定位同一目标并执行一次正常点赞操作。操作前已是点赞状态时必须返回 `already_liked` 且不点击；未点赞时只点击一次。成功必须以目标 ID 对应的 reaction 请求/推送和操作后协议状态为准，不以图标或提示文案为准。

文字必须填写真实正文并点击发送；群聊 @ 必须在编辑器中输入 @、选择协议成员目录解析出的唯一成员、填写真实正文并点击发送；图片必须选择真实文件并点击图片预览发送；引用回复必须用协议消息 ID 定位对应页面消息后执行引用并填写真实正文。不得使用 `protocol-bootstrap-*`、随机占位符或其他临时消息，不得遍历 React/Fiber 或任意页面对象图。页面可见文本不得用于判断发送成功。

第一次写命令必须只生成包含 Profile、目标会话、模式、正文摘要、引用消息 ID或图片摘要和 plan ID 的 `confirmation_required`。只有第二次调用携带匹配、未过期且未漂移的确认参数才可发送。确认后的业务会话必须重新验证 Cookie、IM 初始化、目标会话和当前协议上下文。

`protocol send` 必须接受互斥 payload：`--text` 或 `--image`。两者同时存在或均不存在必须在形成预览前失败。`--at` 可重复或使用逗号列表，但只允许群聊文本 payload。

群聊 @ 必须从协议成员目录解析稳定 UID；重名返回候选，不得猜测。页面选择结果和监听到的发送 Protobuf 必须与预期 UID、正文和 UTF-16 mention 区间一致。私聊不得接受 @。

图片发送必须把用户确认的真实本地图片交给页面图片选择控件，并点击图片预览中的发送按钮。CLI 必须监听并校验页面完成的上传配置、Apply、TOS 上传、Commit、`batch_build_image` 和最终 `message_type=27`、`aweType=2702` 发送；任一阶段失败必须报告对应稳定阶段，且不得伪造最终 sent。

batch 发送必须在一次确认后复用 broker 持有的一个 Profile 锁和一个后台有头业务会话，按计划顺序严格串行。每个成功发送与下一次尝试之间必须至少等待 5 秒；每条消息独立形成页面操作并验证动态请求与回执。任何失败或不可验证结果必须停止剩余任务，并分别报告 sent、failed 和 skipped。

每次实际发送必须在 `message/profiles/<profile>/sends/YYYY-MM-DD/<timestamp>__<client-id>/` 保存编号 CLI 请求、服务端响应、receipt 与 verification；图片发送另存脱敏上传摘要。验证至少覆盖 HTTP 状态、Protobuf 状态、服务端 result、client message ID 回显和目标会话一致性，不能根据 DOM 可见文本声称成功。落盘记录不得包含 Cookie、token、动态签名、上传临时凭证或完整签名 URL。

视频发表评论、回复评论和评论点赞必须使用 broker 的后台有头 Camoufox 打开目标视频的正常页面界面并执行一次受控操作。发表评论保存真实正文；回复评论必须由协议 comment ID 解析目标后进入对应回复入口；评论点赞必须保证最终为已点赞，若操作前已点赞则不点击。CLI 必须监听并校验写请求的 aweme ID、comment ID 或 parent comment ID、正文摘要和业务结果，并在只读协议复查中确认新评论或点赞状态。

所有非发送写操作必须保存到 `message/profiles/<profile>/interactions/<domain>/YYYY-MM-DD/<timestamp>__<operation-id>/`，其中 domain 为 `chat` 或 `video`。记录必须包含确认计划、目标稳定 ID、操作前状态、受控页面操作计数、脱敏请求/响应、receipt、verification 和复查结果。不同 Profile 不得读取或覆盖彼此的写计划与回执。

### 场景：文字与回复

给定唯一现有会话和未漂移确认计划，当发送普通文本或引用回复时，系统使用真实正文和真实引用目标执行一次页面发送；服务端回执通过后报告 sent，且不存在临时消息、请求替换或第二个发送请求。

### 场景：群聊 @ 与图片

给定唯一群聊成员或有效图片，当用户确认后，系统分别在页面选择真实成员或真实图片并点击一次发送，同时验证 SDK 请求和服务端回执；不得遍历 React/Fiber，失败后不得重复点击。

### 场景：聊天消息点赞

给定协议记录中的唯一消息 ID，当用户确认点赞时，未点赞状态下页面只执行一次正常点赞并由 reaction 协议复查；已经点赞时返回 `already_liked`，不得因为再次点击而取消。

### 场景：视频评论互动

给定唯一视频或评论目标及未漂移确认计划，当发表评论、回复评论或评论点赞时，broker 的后台有头页面只执行一次对应正常操作；网络回执和后续只读协议均与预期目标一致后才报告成功，失败或不可验证时不得重复提交。

### 场景：批量节流

给定三个已确认目标，当三个发送均成功时按原顺序完成且两个相邻发送间隔均不少于 5 秒；第二个失败时第三个标记 skipped 且不产生发送请求。

## 页面诊断、协议抓包与研发边界

page doctor 必须以只读方式检查页面 bundle 内容指纹、结构化字段、稳定页面属性和脱敏样本，报告字段新增、删除、改型和可空性变化。它不得发送消息、自动修改正式映射或保存无法证明脱敏的原始样本。

所有前端选择器必须集中在带版本号和适用页面的 UI 适配器中；业务层不得散落 `data-e2e`、CSS class、aria 或可见文案。协议字段、SSR/CSR 结构化响应和网络回执是业务主契约，选择器只负责最终页面动作。page doctor 必须比较当前 bundle 指纹、页面路径和适配器要求的最小控件基线；基线缺失或歧义时，相关写计划返回 `PAGE_ADAPTER_DRIFTED` 并停止，不能猜测、盲点、自动改映射或遍历 React/Fiber。只读协议未依赖该控件时应继续工作并单独记录漂移告警。

Page Doctor 必须按每个 Component Adapter 分别报告页面版本和路径、是否 exercised、候选数量、唯一性、可见/启用状态、采用的语义或局部结构策略，以及漂移影响的具体写能力。聊天输入框和发送控件只有在唯一会话已经选中后才算 exercised；未选择会话时报告 `not_exercised`。单个组件漂移不得阻止不依赖它的协议读取或其他写组件。

## Windows 后台窗口与接管

Windows broker 创建 `MozillaWindowClass` 主窗口时必须在首次显示前保存原始 extended style，设置 `WS_EX_LAYERED | WS_EX_TOOLWINDOW`、清除 `WS_EX_APPWINDOW`、alpha 0，并在 style 变化后刷新非客户区；若任务栏已经登记窗口，还必须通过任务栏 API 移除。后台验收必须同时证明窗口像素不可见、任务栏无入口、Alt+Tab 不枚举该窗口、未移动到屏幕外且页面仍维持正常 headed 生命周期。`MozillaBatteryClass`、`MozillaHiddenWindowClass` 与输入法辅助窗口不作为可接管主窗口。

人工扫码、短信、滑块或通用验证需要接管时，系统必须对同一主窗口恢复保存的 extended style、alpha 255、正常位置和任务栏登记；验证完成后自动重新应用后台 style 并移除任务栏登记。每次双向切换都必须记录窗口 class、visibility、alpha、extended style、taskbar/Alt+Tab 判定与主窗口数量，不能只检查 alpha。

领域目标必须使用服务端稳定 ID：聊天使用 conversation ID、message ID 与成员 uid/sec_uid，视频使用 aweme ID，评论使用 comment ID 和 parent comment ID。UI adapter 可以在当前页面用有界文本或控件定位完成一次动作，但必须用监听到的网络请求重新校验稳定 ID；前端控件、class、文案或 bundle 字段不得写入计划作为跨命令目标。一个 adapter 的基线漂移只阻止依赖该 adapter 的写能力，不得把协议读取或其他页面能力统一标记为登录失败。

protocol capture 必须使用同一 Camoufox Profile 打开有头浏览器，允许用户完成一次正常普通发送或引用回复，并把脱敏 manifest、二进制请求/响应、字段 inventory 和完整性状态保存到调用项目 diagnostics。manifest 必须移除敏感 query 和 header 值；未捕获完整响应时必须记录 incomplete。

`camoufox-reverse-mcp`、Babel、mitmproxy、WABT 与研发 Skill 可以用于 `analyze/` 下的维护者抓包、Hook、JS/WASM 和协议验证，但不得由普通业务 CLI 自动安装、启动或调用，不得进入生产依赖锁或分发包。

### 场景：普通用户运行 page doctor

给定用户只安装生产依赖，当 page doctor 运行时，它可以使用 Camoufox 完成只读诊断，不要求 MCP、Babel、mitmproxy 或 WABT 存在。

## Skill 编排与恢复

Skill 必须从用户项目运行 CLI，把 stdout 视为 NDJSON，并在任务开始时处理 doctor、setup、Profile 选择和登录状态。恢复流程必须保留已确定的 Profile、conversation ID、时间范围、正文、图片和原命令参数。

`PROFILE_IN_USE` 必须先获得终止确认；`LOGIN_REQUIRED` 必须打开同一 Profile 有头登录；`VERIFICATION_REQUIRED` 必须打开同一 Profile 完成人工验证。恢复成功后只重试原任务一次。受控页面最终发送是正式链路而非 fallback；任何失败不得重复点击或重复发送。

自动化测试与交付验证没有用户明确指定真实目标和正文时，只能验证 fixtures、预览、builder、节流和回执判断，不得线上发送。

所有命令必须显式选择一个 Profile，或在项目只有一个有效 Profile 时返回可确认的默认候选；不得把某个开发者 Profile 写死为默认。`profile list` 必须同时报告每个 Profile 的运行时版本、登录状态、锁状态、最近会话索引刷新、聊天数据摘要、内容数据摘要和最近写操作时间，但不得读取其他 Profile 的敏感内容形成聚合缓存。

Python 包必须保持以下依赖方向：`models` 与 `storage` 不依赖浏览器；`protocol` 只负责结构化读协议与解码；`reads` 编排协议和存储；`ui` 只包含有界页面操作与选择器；`writes` 使用协议目标和确认计划调用 `ui`，并监听回执；`runtime` 管理 venv、Camoufox、Profile、锁、登录、`ensure_ready` 和会话索引；feature adapters 组合聊天与视频能力。CLI 命令不得直接包含 Protobuf 解码、页面选择器或文件布局逻辑。

正式源码必须按职责形成包目录，而不是把 broker、浏览器、Profile、协议和业务功能平铺在 `doudou` 包根目录。`common/` 只保存无业务依赖的错误、序列化和文本工具；`runtime/environment.py` 管理共享 venv 与 doctor/setup；`runtime/broker/` 管理 IPC 与生命周期；`runtime/browser/` 管理 Camoufox context；`runtime/platform/` 管理 Windows 隐藏窗口和 Linux 虚拟显示；`runtime/profile.py`、`runtime/lock.py` 与 `runtime/paths.py` 管理项目本地身份和所有权。业务层必须分别位于 `models/`、`storage/`、`protocol/`、`reads/`、`ui/`、`writes/` 与 `features/chat/`、`features/video/`。包根 `cli.py` 只允许参数解析、调用编排和 NDJSON 输出，不得实现业务算法。

依赖方向必须单向：`common -> models/storage -> protocol -> reads`；页面写路径为 `models/protocol -> ui -> writes`；`features` 可以组合 `reads` 与 `writes`；`runtime` 为这些层提供浏览器和 Profile 能力，但不得反向导入 feature adapter。`storage` 不得导入浏览器，`protocol` 不得包含页面点击，`ui` 不得决定生产目录，`cli` 不得直接依赖页面选择器或 Protobuf 字段号。违反边界的导入必须由自动化架构测试失败。

### 场景：新增业务能力

给定后续新增一种聊天系统事件、视频互动或页面选择器，当实现进入正式代码时，数据结构、协议解析、读取编排、页面动作、写回执和聊天/视频组合分别位于对应包；`cli.py` 只增加命令映射，broker 只增加可调用 handler，不把业务算法复制到入口或 runtime。

正式运行时迁移必须采用研究先行门槛：先在 Git 忽略的 `analyze/` 使用隔离 Camoufox 环境与实验 Profile，验证同一 browser-data 的有头登录/人工验证、后台有头 broker IM 初始化和持久复用，记录脱敏指纹、Cookie/Storage 哈希变化、协议拉取，以及经用户确认的真实页面发送结果和受控 DOM 操作计数。该门槛未通过时不得删除现有 Node.js 生产 CLI；研究失败必须形成明确结论和下一实验。

研究门槛必须新增独立实验与结论：聊天系统事件结构化、消息图片读取、用户名解析与视频列表、SSR 视频详情、封面下载、根评论与回复分页、聊天点赞、视频发表评论、回复评论、评论点赞，以及两个 Profile 的锁和目录隔离。`analyze/video-comments/` 的历史 Node 研究文件与输出不得覆盖；Python/Camoufox 验证写入新的隔离子目录并引用既有 3,402 条记录作为对照。

进入 Archive 前必须对正式 Python CLI 与正式 Camoufox Profile 执行全量真实业务回归并全部通过。回归至少覆盖登录/扫码/短信或滑块接管、后台隐藏恢复、同一 broker 连续命令、群聊与私聊、今日全部消息、进群系统事件、未知字段 inventory、消息图片下载解密、抖音号身份解析、视频列表、视频详情、评论与回复分页，以及在用户授权测试目标上的文字发送、引用回复、@、图片、聊天点赞、视频评论、回复评论和评论点赞。每项必须有脱敏的目标 ID、命令、退出状态、业务回执和只读复查证据；失败、跳过、人工未完成或结果不明确的项目均不得记为通过或进入 Archive。

### 场景：登录失效后恢复原任务

给定用户已选会话和时间范围，业务会话发现 Cookie 缺失，当用户在同一 Camoufox Profile 有头登录完成后，Skill 用原参数恢复一次 broker 后台任务，不要求重新选择目标。
