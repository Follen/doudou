# Outcome

首次安装在慢速网络、并发调用或调用方取消时仍能稳定完成或干净失败重试；所有落盘业务对象统一保存可获得的完整用户身份；CLI 提供零 setup 开销的版本别名和可控更新入口。

# Scope

- npm 启动器下载并校验 Doudou 专用 Python 3.12.13；不再探测或选择用户电脑上的 Python。
- 默认以 `~/.doudou` 作为唯一用户级 `DOUDOU_HOME`；专用 Python、venv、浏览器、下载、锁、状态、日志和 rollback 均位于该根目录，允许环境变量覆盖。
- 重写 setup 执行边界：用户级单实例锁、并发等待、分阶段进度、停滞/总超时、取消传播、进程树回收、临时 runtime 校验、原子切换和失败回滚。
- 在 analyze 隔离 HOME/cache 中完成 Camoufox 0.5.4 下载专项实验，并在 Windows/Linux CI 覆盖直连、代理、TLS、异常中断、磁盘/权限、并发和重试。
- 建立统一身份模型、Profile 级缓存、协议补全与迁移管线，覆盖群聊/私聊消息、参与者、群成员、进群事件、引用/@、视频作者、评论和回复用户。
- 新增 `doudou -v`、`doudou -version`，与 `doudou --version` 输出相同纯版本号，且不触发 Python、setup 或浏览器。
- 新增 `doudou update` 及更新检查、升级、无需更新和失败回滚流程。
- 更新 Skill/README、npm 分发、manifest、测试与 Windows/Linux CI。
- 回归全部既有读写能力；真实写继续先 plan 后确认、只执行一次、失败不自动重放。
- 修复 broker 登录态误判：账号/会话证据与 IM 抓包状态分离，登录成功事件异步触发状态重算并自动结束人工接管。
- 所有 DOM 操作和页面状态判断统一使用三级组件漏斗：版本化语义候选、已确认局部组件结构、唯一可见/启用/状态/几何校验；无法闭合时 fail closed。
- 修复 broker 异步登录状态机的丢唤醒、连续事件合并遗漏和页面导航后观察器失活，并以结构化账号/IM证据优先判断登录。

# Non-goals

- 不把 venv、浏览器、Python packages、Profile、message、Cookie、登录态或 analyze 打进 npm/Skill 或推送远端。
- 不通过 React/Fiber 深遍历补身份，也不拿 nickname 冒充抖音号。
- 不改变现有写操作的确认机制和失败重放边界。
- 不把共享 Camoufox 浏览器缓存迁入 Doudou runtime。
- 不复用系统或其他工具的 Camoufox 共享缓存。
- 不继续把用户系统 Python 作为生产 runtime 或 setup 前置条件。
- 不再提供发送群聊 @ 的计划、成员弹层选择或写入能力；读取既有消息中的 @ 身份仍属于身份补齐范围。

# Acceptance examples

- 两个首次 setup 同时启动时，只有一个安装进程；另一个显示正在等待，复用同一成功结果，不创建第二套临时 venv。
- setup 在 pip 或浏览器下载期间被取消/超时后，子进程树退出、current pointer 与旧 runtime 不变、临时目录可回收，下一次 setup 可直接成功。
- 慢速浏览器下载持续输出阶段、已传输量或可用的原生进度，不因长时间无最终 stdout 被误判为卡死。
- 没有安装 Python 的机器可由 npm 启动器完成专用 Python、venv、依赖和浏览器 setup。
- 专用 Python、venv 和依赖在临时 runtime 全部验证后一起切换；update 构建失败时旧 runtime 保持可用。
- 旧消息只有 sender uid/sec_uid；重新采集或运行迁移后补入 unique_id/nickname/display_name，旧空值不会覆盖缓存中的完整值。
- API 没有 unique_id 时，该字段保持 null，并带缺失/来源状态；不会写入 nickname 作为替代值。
- `doudou --version`、`doudou -version`、`doudou -v` 都只输出同一个版本号，即使 runtime 尚未安装也成功。
- 更新失败时仍可运行原 npm 启动器和原 Skill/runtime；项目 profile/message 内容与哈希不变。
- 浏览器下载被中断后保留可验证的 `.part`，下次在服务端支持 Range 时续传；不支持 Range 时安全清零重下，不把 partial 当成缓存命中。
- `doctor` 能区分依赖半安装、下载中/停滞、缓存损坏、版本/哈希不符和 launch 失败，并给出可执行修复建议。
- Windows 已存在有效 runtime 时，后续业务命令读取 `venv/Scripts/python.exe` 并直接复用，不因 Linux 路径误判重复 setup。
- 真实 Profile 的群聊、私聊、成员、进群事件、引用/@、视频作者及评论用户完成一次只读身份回归；迁移第二次运行改动为零。
- `/chat` 已加载会话列表且没有登录/验证码/滑块时，即使认证 Cookie 名称未知也判定账号已登录；缺少 IM 抓包单独返回 `IM_CAPTURE_MISSING` 或 `IM_INITIALIZATION_MISSING`。
- 登录成功后业务读取失败、IM 抓包缺失或等待超时，只要页面没有真实登录/验证组件，同一个 broker 窗口都自动恢复隐藏。
- 页面存在同名隐藏文本、错误容器、多候选、disabled 或错误几何时不执行动作，返回 `PAGE_ADAPTER_DRIFTED`；Page Doctor 指明受影响组件和能力。
- 从视频页进入聊天写操作时先确认 `/chat` 左侧会话列表；消息菜单、图片预览和视频评论动作只在所属局部组件内解析。
- 用户请求发送群聊 @ 时不会打开成员弹层或执行写操作，而是返回稳定、可理解的不支持结果。
- 连续网络/DOM事件、Event 清理竞态或主 frame 导航均不会丢失最新登录状态重算；验证消失后同一窗口自动隐藏。

# Constraints and invariants

- setup 锁和状态位于 Doudou 用户缓存，不依赖当前项目；锁必须识别存活所有者和可回收的陈旧所有者。
- 每个外部安装进程都必须加入可终止的进程组/Windows Job；父进程退出不能留下 pip、Python、Camoufox 或下载器孤儿。
- 只有专用 Python 的归档 SHA-256、依赖闭包、关键 import、浏览器资产 SHA-256/元数据、浏览器版本和真实启动全部通过后才能切换 current pointer。
- 安装错误按 `DEPENDENCY_INSTALL_FAILED`、`BROWSER_DOWNLOAD_FAILED`、`RUNTIME_VERIFY_FAILED`、`SETUP_CANCELLED`、`SETUP_TIMEOUT` 等稳定类别输出；报告不保存下载 URL 签名或凭据。
- 身份字段固定为 `uid`、`sec_uid`、`unique_id`、`nickname`、`remark_name`、`display_name`，并记录字段来源/补全状态；空值不能覆盖非空值。
- 身份补全优先使用页面上下文内的结构化协议与已有响应，不读取消息 DOM，不遍历任意对象图。
- 登录分类不依赖固定 Cookie 白名单，也不以本次 IM 请求/响应是否被监听到作为账号登录结论；会话页面证据只使用有界 DOM/网络状态，不遍历 React/Fiber。
- DOM 选择器、稳定文案和结构关系只存在于带版本号的 Component Adapter；业务层和 runtime 不允许全页模糊命中、任意 first/nth、固定坐标或猜测点击。MutationObserver 只唤醒状态机，不直接给出业务结论。
- 页面状态机使用单调 generation 或等价无丢唤醒机制；连续事件最终至少处理最新 generation，主 frame 导航后按当前 takeover 状态恢复观察器。
- npm 包和 Skill 本体不携带运行时；setup 在 `${DOUDOU_HOME:-~/.doudou}/runtimes/<runtime-id>/` 创建同级 `python/` 与 `venv/`。
- `${DOUDOU_HOME:-~/.doudou}/browsers/` 是 Doudou 唯一浏览器缓存；项目 `profile/`、`message/` 和 Agent Skill 目录不迁入其中。
- 浏览器下载必须区分“没有新字节的停滞超时”和“整个阶段的总超时”，并实时输出阶段心跳；partial 文件和缺失 version metadata 不能视为成功。
- 本地调研已验证：Python 3.12.9 和干净 3.13.12 均安装 Camoufox 0.5.4、browserforge 1.2.4，`pip check` 与关键 import 通过。截图中的 import 失败不足以证明 Python 版本或锁文件不兼容。
- Camoufox 0.5.4 源码确认：浏览器 zip 写入系统临时文件而不是最终缓存，所以缓存不增长不能证明卡死；下载请求没有显式超时；CLI 捕获 fetch 异常后可能正常返回；下载前创建最终版本目录但没有跨进程锁；现有 setup 的 `capture_output` 会隐藏 Rich 实时进度。
- 朋友截图必须分两段归因：第一次是多个或被中断的 setup 留下不完整临时 venv，导致 browserforge `Fingerprint` import 失败；清理后第二次已建成约 222 MiB runtime，才停留在约 492 MB 的 GitHub Release 浏览器下载。HTTP 200 只证明资源可达，不证明大文件下载完成。

# Decisions

- 用户要求把三组需求放进一个新的独立 Native change，不并入已归档的卸载 change。
- setup 的第二个调用等待当前安装，并复用最终结果；不并行构建另一套 runtime。
- 失败或取消在原子切换前不改变当前 runtime，临时环境清理后允许直接重试。
- 凡业务记录出现用户身份，都复用统一 enrichment/cache；缺失字段诚实保留 null 和来源状态。
- 既有落盘记录提供可重复迁移/merge，新的完整字段优先于旧空字段。
- 三个版本参数输出一致纯版本号，并由 npm 启动器直接处理。
- 用户选择更新方案 A：`doudou update --check` 只读检查；`doudou update` 只升级已全局安装的 npm 包。npx 场景只报告临时运行方式和版本状态，不自动创建或修改全局安装。
- 生产 runtime 固定使用 python-build-standalone CPython 3.12.13、发行批次 `20260728`，平台资产 URL 与 SHA-256 写入版本化 runtime manifest；升级 Python patch 或发行批次即产生新 runtime ID。
- Windows x86_64 使用 `install_only_stripped` 资产（21932566 bytes，SHA-256 `242b94b37682ac55f9bf9eb624348dc8d17c64f74f56028104545ea3ffe35e26`）；Linux x86_64 使用对应 stripped 资产（34175396 bytes，SHA-256 `7ceaf4360f4b4ab44f31d8217cfdc70df8e12b61d9561b4900ee7af87176e50d`）。
- npm/Node 负责无 Python 前提的 bootstrap、下载校验和启动专用 Python；后续 Python 只在受管 runtime 中运行。
- `doudou uninstall` 删除受管 Skill 与整个 `DOUDOU_HOME`；项目 profile/message 继续保留。
- 浏览器资产使用 Doudou 受控下载器写入隔离 `.part`，支持 Range 续传、连接/停滞/总时限、有限退避重试、精确 SHA-256 与版本校验，再交给 Camoufox 缓存原子激活；不直接以官方 CLI 的退出码作为成功证据。
- 下载连接超时为 15 秒、连续 90 秒无新字节判定停滞、浏览器下载阶段总时限为 45 分钟；最多尝试 3 次，退避 1/3/10 秒。状态文件每秒更新，终端至少每 5 秒输出一次阶段心跳。
- 用户已明确确认本 change 的完整范围，并要求按当前共享理解直接推进，无需为 controlled download 补充范围再次询问。
- 用户补充确认 `DOUDOU_HOME=~/.doudou` 的统一布局、Windows runtimeReady 回归和真实身份只读回归，要求直接并入当前 change。
- 用户确认把 broker 登录误判与接管不收尾问题直接并入本 change，并采用异步页面/网络事件重算，不再等待固定 Cookie 轮询。
- 用户确认所有 DOM 动作与页面业务判断统一执行三级组件漏斗，并要求全量审计读写入口、补齐 Page Doctor、事件状态机和真实业务回归，无需再次确认。
- 完整成员弹层实验触发 Camoufox 内容进程异常增长到约 7.4 GB，并以 `mention_member_exact_rebind_timeout` 结束；用户据此确认产品不再支持发送群聊 @，相关弹层、Page Doctor 与写回归全部移出范围。
- 用户选择兼容方案 A：CLI 继续识别旧 `--at` 参数，但在启动 broker 或浏览器前返回 `MENTION_UNSUPPORTED`；Skill 和公开使用文档不再出现该参数或群聊 @ 能力。

# Open questions

无。

# Verification expectations

- Node/Python 单测、Ruff、Skill Creator、manifest、pack 白名单与凭据扫描全部通过。
- 隔离 HOME/cache 做 Python 3.12/3.13 首次 setup、并发、超时、取消、孤儿回收、失败重试和原子切换回归。
- 真实全局安装、npx、禁用 lifecycle、版本别名和 update 回归覆盖 Windows/Linux cmd/PowerShell shim。
- 全量回归群聊/私聊发现、消息、图片、member_joined、身份迁移、视频/评论读取和全部既有写能力。
- Windows/Linux CI 全绿后才能发布。
- 真实 Profile 验证登录/验证码接管、IM 分类、窗口显隐、任务栏/Alt+Tab、群聊/私聊/图片/系统事件/身份/视频评论读取和全部既定写能力；每个写 plan 只执行一次，失败不自动重放。
