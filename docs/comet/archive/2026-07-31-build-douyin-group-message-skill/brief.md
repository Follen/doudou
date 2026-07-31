# Outcome

交付一个名为 `douyin-group-message` 的可分发 Codex Skill。Skill 通过确定性的 Node.js CLI 驱动 Playwright，让每位使用者在自己的电脑上以有头 Playwright-managed Chromium 完成人工登录并保存独立 Profile，随后按指定群聊和时间范围批量导出群消息，并能对指定消息执行引用回复。

# Scope

- 在仓库根目录提供符合 Codex Skill 规范的 `SKILL.md` 和 `agents/openai.yaml`。
- 提供可直接调用的 Node.js CLI，账号、Profile、群聊、时间范围和输出路径均由运行时参数或本地配置决定。
- 分享包包含锁定版本的 `package.json`、`package-lock.json`、环境自检和 setup 命令；首次运行缺少 Playwright 或对应 Chromium 时，Skill 在自身目录完成本地依赖安装并展示进度，不依赖全局 Playwright。
- doctor 同时支持抖音页面契约漂移诊断：检测 bundle、接口字段和 DOM 稳定属性变化，生成字段级 diff 与脱敏诊断包，供适配器更新。
- Skill 提供完整对话式编排：确认 Profile、刷新群聊、明确显示“发现 N 个群聊”、展示编号群列表、允许选择单个/多个/全部群聊、收集时间范围、预览拉取计划、展示逐群进度并汇总结果与输出位置。
- 支持创建、列出和使用本地持久化浏览器 Profile；首次登录默认使用有头 Playwright-managed Chromium，并复用登录态。
- 除首次登录、重新登录和需要人工处理的验证流程外，环境/page doctor、列群、采集和回复均默认使用无头 Playwright-managed Chromium；真人可见路径约束页面操作顺序，不要求显示浏览器窗口。
- 浏览器登录态必须保存在项目内独立的 Profile 根目录；每个命名 Profile 包含浏览器数据目录和无敏感值的元数据。没有可用登录态时，Skill 引导创建或选择 Profile、打开有头浏览器人工登录、验证后继续原任务。
- 动态发现当前账号的群聊并返回稳定标识与显示名称，供用户选择一个或多个群聊。
- 对选定群聊滚动加载指定时间范围内的历史消息，记录群聊、消息、发送时间、发送人、消息类型、文本和图片链接。
- 页面适配必须遵循当前真人可见流程：从顶部消息面板进入混合会话列表，打开群聊后查看当前消息、向上滚动加载旧消息，再对单条消息执行引用回复。
- 采集结果、增量元数据和回复审计记录默认保存到运行项目内的本地数据目录，并按 Profile 与群组拆分；每个群组一个文件夹。
- 支持定位一条已采集的目标消息，并通过抖音页面的引用回复交互发送文本回复。
- 回复发送必须避免并发写入，具备频率限制、确认、失败熔断和可审计结果。
- 提供自动化测试，并用当前测试 Profile 完成不泄露凭据的端到端验证。

# Non-goals

- 不自动识别、求解或代用户提交验证码、滑块和其他需要人工完成的安全验证；无头浏览器与同一 Profile 的有头浏览器保持稳定指纹一致性属于本 change 范围。
- 不提供集中托管多账号的服务器；每位使用者在本机运行并管理自己的 Profile。
- 不把当前 `Follen-douyin` Profile、群名称、账号标识、绝对路径或登录凭据写入发布内容。
- 不发送营销群发、无目标消息或未经用户明确授权的回复。
- 不承诺抖音页面结构变化后无需更新适配器。

# Acceptance examples

- 新用户安装 Skill 后创建任意命名的 Profile，CLI 打开有头 Playwright-managed Chromium；用户登录并关闭后，同一 Profile 可再次打开正常首页。
- Profile 目录存在但登录失效时，Skill 标记状态并重新打开同一 Profile 供人工登录；验证成功后恢复用户原先的列群或拉取任务，不要求重新输入已确认参数。
- 用户列出群聊时，CLI 返回当前账号可见群聊的名称和内部稳定标识，不依赖预置群名。
- 用户只说“拉取群消息”且尚未指定群聊时，Skill 先刷新群列表，告诉用户群聊总数并显示编号列表；用户可用编号、多个编号、范围或“全部”选择，Skill 再收集缺失的时间范围。
- 用户指定多个群聊以及 `from`/`to` 时间后，CLI 只导出范围内消息；每条记录包含可用于后续回复的消息标识、发送时间、发送人和内容，图片消息包含可获取的图片链接。
- 拉取多个群聊时，Skill 按 `[当前/总数]` 展示逐群状态；完成后汇总每群新增、重复、冲突、失败数量以及实际输出目录。只读拉取中单群失败必须继续其余群并在最终汇总中明确标记。
- 用户选择一条导出记录并提供回复文本时，CLI 在目标群中建立引用回复、展示目标与待发送内容、按确认策略发送，并验证发送结果。
- 同一个 Profile 已被进程占用时，后启动的任务展示经校验的占用进程信息并询问用户是否终止；只有用户明确确认后才终止占用进程、等待锁释放并自动继续原任务，用户拒绝时返回 `PROFILE_IN_USE`。
- 页面出现验证码或安全验证时，CLI 停止自动发送并要求人工处理，不自动识别、求解或代用户提交验证挑战。
- 同一 Profile 分别以有头和无头模式运行指纹探针时，无头模式的 UA 不包含 `HeadlessChrome` 且与有头模式一致，`navigator.webdriver` 呈现普通有头浏览器值，固定 Canvas 探针结果以及 WebGL vendor/renderer 与有头基线一致；连续运行不得随机漂移。
- 抖音 bundle、结构化字段或稳定 DOM 属性变化时，page doctor 生成 bundle 内容指纹、字段新增/删除/改型/可空性 diff、DOM finding 和脱敏样本；诊断不发送消息，也不自动修改正式字段映射或基线。

# Constraints and invariants

- 使用 Node.js CLI 和项目本地 Playwright 依赖，不要求使用者预先全局安装 Playwright。
- Playwright 必须锁定精确版本；Chromium 必须与该版本要求的 revision 匹配。Node、npm、网络、写权限或浏览器安装失败必须返回机器可读诊断与可重试步骤。
- 页面漂移诊断只读运行，不触发回复发送；诊断包不得包含 Cookie、token、完整消息正文、用户头像原图或带认证查询参数的 URL。
- 首次登录、登录失效和人工验证恢复使用有头浏览器；其余实际运行默认无头，不得因列群、采集或回复自动弹出浏览器窗口。
- 有头与无头模式必须复用同一 Playwright-managed Chromium、持久 Profile、locale、timezone、viewport 和 device scale。每个 Profile 必须从实际有头运行建立稳定指纹基线；无头运行必须完整对齐网络与 JavaScript 可见 UA/UA-CH、`navigator.webdriver`、Canvas、WebGL、platform、languages、hardwareConcurrency、deviceMemory、screen、permissions、plugins/MIME、media capabilities 和 WebRTC 可观察表面。对齐值不得按页面、任务或启动次数随机变化，也不得使用与该 Profile 有头基线无关的通用随机设备身份。
- 实测当前抖音会对 Playwright Chromium 的默认 headless、`channel: chromium` 和 `--headless=new` 都显示验证码中间页；有头验证后原无头任务只自动重试一次，再次命中时停止恢复循环并返回 `HEADLESS_VERIFICATION_PERSISTENT`。
- 浏览器 Profile、导出数据和截图默认保存在 Git 忽略的本地数据目录，不进入 Skill 发布包。
- 项目内数据目录必须默认被 Git 忽略；群组文件夹必须包含稳定会话标识以处理重名、重命名和非法路径字符。
- Profile 同一时间只能由一个浏览器上下文持有；所有会产生外部消息的操作必须串行化。
- 回复前必须能唯一确认目标群聊和目标消息；模糊匹配不得发送。
- 发送失败、页面结构不匹配、登录失效或风控页面均应失败关闭，不继续尝试后续发送。
- 图片 URL 可能是短期签名地址；导出必须如实标记来源，不能声称永久有效。
- Skill 指令和 CLI 日志不得输出 Cookie 值、令牌或其他登录凭据。

# Decisions

- Skill、项目和 CLI 名称统一为 `douyin-group-message`。
- 产品形态是可分发的本地 Codex Skill，不是中心化多用户服务。
- Node CLI 承担确定性浏览器操作，`SKILL.md` 负责触发、参数收集、安全确认和命令编排。
- 登录采用有头浏览器人工完成；登录态保存在每位用户自己的持久化 Profile 中。
- 消息采集支持指定多个群聊和指定时间范围。
- 导出至少覆盖发送时间、发送人、文本、图片链接、消息标识和群聊标识。
- 回复目标必须是指定消息，并需要控制并发与频率。
- 回复同时支持单条和批量任务；默认是单条，批量必须显式启用，内部严格串行处理。
- 批量回复先展示完整发送计划；用户整批确认一次后，系统按计划顺序串行发送，任一风控、验证或发送失败都会中断剩余任务。
- 本地运行目录需要保存可复核数据：每个 Profile 下按群组建立独立文件夹，群组文件夹内保存已拉取历史消息、采集元数据和回复记录。
- 默认数据布局固定为 `.douyin-group-message/profiles/<profile>/groups/<群名>__<会话ID>/`；可读群名用于浏览，稳定会话 ID 用于隔离身份。
- 同一群组多次拉取采用增量合并：按稳定消息 ID 去重，维护一份持续更新的群历史；同 ID 的非空字段冲突必须显式记录，不得静默覆盖。
- 群组目录使用 JSONL：`messages.jsonl` 保存规范化消息历史，`replies.jsonl` 追加保存回复计划与执行结果；更新必须避免留下半写文件。
- 结构化存储固定包含 Profile 元数据、群聊索引、群聊元数据、消息记录、冲突记录、回复审计和每次拉取 Run 汇总；每种 JSON/JSONL 都有固定 schema 标识、RFC 3339 时间和稳定 ID。
- 批量任务即使整批确认，也必须复用单条真人操作路径：逐个重新定位目标、核对引用对象、发送并确认结果后再处理下一项。
- 用户已确认批量发送采用逐条真人路径：每条目标先查看上下文、执行引用回复、确认发送结果，再处理下一条。
- Skill 负责友好的参数收集和结果呈现，CLI 负责确定性的 JSON 输入输出；CLI 返回群聊总数、稳定 ID、拉取进度事件和最终统计，Skill 不从自由文本日志猜测状态。
- Skill 每次执行实际浏览器任务前先运行环境 doctor；doctor 通过后继续，缺少本地 npm 依赖或 Chromium 时进入 setup，安装成功后自动恢复用户原任务。
- 正常任务遇到未知字段、字段类型变化、关键选择器缺失或页面 bundle 指纹变化时，CLI 返回稳定漂移代码并提示运行 page doctor；未知契约不得静默猜测。
- 项目内登录态根目录固定为 `Profile/`；命名 Profile 的浏览器状态保存在 `Profile/<profile>/browser-data/`，非敏感元数据保存在 `Profile/<profile>/profile.json`。
- Profile 锁冲突必须询问用户是否终止占用进程；确认后终止并恢复原任务，拒绝后返回 `PROFILE_IN_USE`，不得静默结束其他进程。
- 用户已确认当前完整 Shape 契约并同意进入 Build。
- 用户在 Build 中补充确认：仅登录获取、重新登录和人工验证使用有头浏览器，其余任务默认无头运行。
- 用户已重新确认包含无头运行约束的完整 Build 契约并要求继续。
- 用户要求降低无头模式触发验证码的概率，并明确要求无头模式使用有头 UA、普通有头浏览器的 `navigator.webdriver` 表现，以及与有头环境一致的 Canvas/WebGL 指纹。
- 指纹数据以同一 Profile、同一锁定 Chromium 的实际有头运行结果为基线；不得每次随机生成或跨 Profile 复用不相干的通用身份。
- 先前“保持 headless 原生 UA、禁止 webdriver/Canvas/WebGL 对齐”的约束由本轮决定替换；持续验证码仍沿用停止自动发送并进入有头人工验证的失败关闭行为，不自动改成有头完成整个原任务。
- 用户选择完整指纹对齐范围：除 UA、`navigator.webdriver`、Canvas 和 WebGL 外，还覆盖 UA-CH、platform、languages、hardwareConcurrency、deviceMemory、screen、permissions、plugins/MIME、media capabilities 和 WebRTC。
- 用户要求同时提供两种基线恢复路径：显式有头校准是主路径；自动打开有头校准窗口、采集后关闭并恢复原无头任务是 fallback。两条路径必须复用同一 Profile 锁和原任务参数，不能并发启动两个浏览器上下文。
- 自动校准采用混合触发：默认在 `FINGERPRINT_BASELINE_REQUIRED` 后请求一次确认；当前任务显式启用 `--auto-calibrate-fallback` 时视为仅对该任务的预授权，可直接打开一次有头校准窗口并恢复原任务。预授权不得持久化到 Profile 或影响后续任务。
- 用户已确认包含完整指纹对齐、双校准路径和混合 fallback 触发策略的更新后完整契约，并同意继续 Build。
- 用户补充要求指纹身份必须持久化，避免后续任务因重新生成或丢失对齐值而反复触发验证。每个 Profile 必须保存一份可实际重放的稳定指纹配置，进程退出、电脑重启和后续任务继续复用；不能只保存校验摘要后在下次启动重新生成。
- 用户授权按最优持久化策略继续：普通跨日和重启不触发失效；启动时使用版本兼容检查与本地契约探针验证已保存身份，仅在文件缺失/损坏、schema 或 Chromium 指纹契约不兼容、探针不一致或显式刷新时重新校准。用户确认按更新后的完整契约继续 Build。

# Open questions

- 无。

# Verification expectations

- 对参数解析、时间边界、记录规范化、Profile 锁、回复限速和失败熔断运行自动化测试。
- 对群聊总数、编号选择、单选/多选/范围/全部、空列表、无效选择、逐群进度和最终汇总的 JSON 契约运行自动化测试。
- 对无 Profile、未完成登录、登录失效、人工重新登录、恢复原任务，以及全部 JSON/JSONL schema、原子写入、损坏文件和版本不兼容运行自动化测试。
- 对全新分享目录、缺 Playwright、缺 Chromium、浏览器 revision 错配、离线下载失败和 setup 后恢复原任务运行自动化测试。
- 对 bundle 指纹变化、新增/删除/改型字段、关键 DOM 属性缺失、脱敏诊断包和契约 diff 运行 fixture 测试。
- 对同一 Profile 的有头基线与无头运行执行指纹契约测试，逐项比较 UA/UA-CH、`navigator.webdriver`、Canvas、WebGL、platform、languages、hardwareConcurrency、deviceMemory、screen、permissions、plugins/MIME、media capabilities 和归一化 WebRTC 表面，并验证主页面/frame/worker 一致、重复启动稳定且 Profile 间不误共享基线。
- 验证完成校准后关闭进程并重新启动 CLI，后续无头任务读取同一 `fingerprint.json` 且公开指纹与首次校准一致，不重新生成身份、不要求校准；同时覆盖文件损坏、schema 不兼容、Chromium 指纹契约不兼容和显式刷新导致的受控失效。
- 使用模拟页面或 fixture 验证群聊识别、历史滚动、文本/图片消息解析和引用回复定位。
- 使用当前本地测试 Profile 做只读群聊采集验证，不在自动化测试中发送真实消息。
- 真实回复验证必须由用户明确指定测试目标和文本，并在发送前确认；未获授权时记录为跳过。
- 运行 Skill 结构验证、Node 测试、CLI 帮助命令和敏感文件泄漏检查。
