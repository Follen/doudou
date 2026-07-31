# douyin-group-message

## Skill packaging

系统必须以名称为 `douyin-group-message` 的 Codex Skill 发布，并包含符合 Skill 规范的元数据和简洁工作流说明。实际浏览器操作必须由仓库内的 Node.js CLI 实现，Skill 指令不得要求 Codex 临时生成 Playwright 脚本。

CLI 必须从项目本地依赖加载 Playwright。所有账号相关路径和选择必须在运行时解析，不得包含开发者用户名、开发机绝对路径、固定 Profile 名称或固定群聊。

## Skill interaction and orchestration

`SKILL.md` 必须把常见任务组织成清晰的对话式流程，而不是要求用户自行拼装长命令。用户提出“拉取群消息”等目标但缺少参数时，Skill 必须按以下顺序只收集当前缺失的信息：

1. 检查可用 Profile；没有 Profile 时引导创建并人工登录，只有一个时展示并使用，有多个时让用户选择。
2. 调用确定性的群聊刷新命令，明确展示“发现 N 个群聊”，并以编号列表显示群聊名称和稳定会话标识；可获得时同时显示最近消息时间，但不得把显示名当成唯一身份。
3. 接受单个编号、逗号分隔的多个编号、编号范围和“全部”；解析后回显选中的群聊数量与名称。空选择、越界编号、重复编号或已经失效的会话必须给出明确结果并重新要求选择，不得静默改选。
4. 收集缺失的时间范围。用户可用自然语言日期，由 Skill 规范化为带时区的 CLI 参数；开始晚于结束、时间含义不明确或范围缺失时必须先澄清。
5. 展示只读拉取计划，包括 Profile、群聊数量与名称、时间范围和数据根目录，然后开始执行。
6. 逐群呈现 `[当前/总数]`、群聊名称、已扫描数量、当前最早时间和阶段状态；进度来源必须是 CLI 的机器可读事件，不得解析不稳定的自由文本。
7. 完成后展示总群数、成功/失败群数，以及每群新增、重复、冲突、跳过记录数和输出目录。只读拉取中某个群失败时继续处理其余群，并在最终汇总中保留错误；回复发送仍遵循失败即停止。

CLI 必须支持非交互的结构化参数和 JSON/JSONL 输出，使 Skill 可以可靠编排；CLI 本身不承担开放式自然语言理解。已明确提供的 Profile、群聊 ID 或时间范围不得重复询问。

## Environment bootstrap

分享包必须包含精确锁定版本的 `package.json`、`package-lock.json`、`SKILL.md`、环境自检命令和 setup 命令，不包含 `node_modules` 或浏览器二进制。Playwright 依赖必须使用精确版本而不是范围版本，避免 npm 解析结果与 Chromium revision 不匹配。

Skill 在任何登录、列群、采集或回复任务前必须先运行 doctor，并从机器可读结果判断：

- Node.js 与 npm 是否存在且版本满足项目声明；
- Skill 本地 `node_modules` 是否包含锁文件对应的 Playwright；
- `chromium.executablePath()` 指向的 Playwright-managed Chromium 是否真实存在并能完成一次启动/关闭探测；
- 项目目录、登录态根目录和数据目录是否可写；
- 当前平台是否需要额外系统依赖。

doctor 的浏览器启动/关闭探测必须使用无头模式。doctor 结果固定使用 `douyin-group-message.doctor.v1`，至少包含 `status`、`node`、`npm`、`playwright`、`chromium`、`filesystem` 和 `issues[]`。每个 issue 必须包含稳定 `code`、`message`、`repair_command` 或 `null`。

当 Playwright 缺失或版本与锁文件不一致时，setup 必须在 Skill 自身目录执行等价于 `npm ci` 的本地安装；不得全局安装 npm 包。当对应 Chromium 缺失时，setup 必须使用刚安装的本地 Playwright 执行等价于 `npx playwright install chromium`。安装成功后重新运行 doctor，并自动恢复用户原先的登录、列群、采集或回复任务。

Node.js/npm 缺失、下载网络失败、磁盘空间不足、目录无写权限或 Linux 系统依赖缺失时，setup 必须停止并返回明确 issue；不得把部分安装报告为成功。可重试问题必须给出同一 Skill 目录下的确定性命令。Linux 系统依赖需要管理员权限时，只显示所需命令并等待用户自行执行，不静默提权。

系统 Chrome 只能作为用户明确选择的兼容回退；默认和自动化测试必须使用 Playwright-managed Chromium。浏览器 revision 由当前锁定的 Playwright 决定，不得硬编码开发机上的旧 revision 路径。

## Page contract doctor

环境 doctor 通过后，用户必须能够显式运行只读的 page doctor，对当前抖音页面实际加载的 bundle、结构化消息数据和稳定 DOM 契约进行重新盘点。正常登录、列群、采集或回复任务遇到 bundle 指纹变化、未知字段、字段类型变化或关键 DOM 契约缺失时，必须返回稳定漂移代码并提示运行 page doctor；不得用猜测字段继续采集或发送。

page doctor 必须使用所选 `Profile/<profile>/browser-data/` 以无头模式访问正常消息页面，但不得点击发送、提交回复或修改群消息。每次运行将产物写入 `.douyin-group-message/diagnostics/<doctor-run-id>/`：

```text
.douyin-group-message/diagnostics/<doctor-run-id>/
  doctor.json
  bundle-manifest.json
  field-inventory.json
  dom-contract.json
  redacted-samples.jsonl
```

`doctor.json` 使用 `douyin-group-message.page-doctor.v1`，至少记录 `doctor_run_id`、`profile_id`、`started_at`、`completed_at`、`status`、基线契约版本、当前 bundle 指纹、各诊断文件的项目相对路径和 `findings[]`。每个 finding 必须包含稳定 `code`、`severity`、`contract_area`、`path`、`change_type` 与脱敏摘要。状态只能是 `compatible | drift_detected | incomplete | failed`。

`bundle-manifest.json` 必须记录实际加载的 JavaScript 资源。URL 在记录和哈希前必须删除 query string、fragment、认证参数和用户相关路径值；每项至少包含清理后的资源路径、内容 SHA-256、字节数和发现时间。当前 bundle 指纹由按清理后资源路径排序的内容哈希确定性生成，不能使用带签名的完整 URL。无法读取某个跨域 bundle 内容时，必须记录 `content_unavailable`，不得把 URL 哈希伪装成内容哈希。

`field-inventory.json` 必须从与群聊发现、消息解析、历史分页和引用定位相关的结构化数据中生成字段清单。每个字段以 JSON Pointer 风格路径记录观察到的类型集合、是否出现 `null`、出现次数、样本数和所属契约区域；对象键顺序和数组长度不得造成伪变化。与上次被批准的基线比较时，diff 必须明确列出 `added`、`removed`、`type_changed` 和 `nullability_changed`，并保留基线值与当前值。新增字段只能进入候选清单，不能自动提升为生产适配映射；删除、改型或关键字段缺失必须使状态至少为 `drift_detected`。

`dom-contract.json` 必须盘点真人路径依赖的稳定页面契约，包括消息入口、会话列表、群聊身份/标题、消息列表、历史加载边界、消息稳定标识、发送人、时间、单条“回复”动作、引用对象、编辑器、发送动作和发送结果。契约项至少记录逻辑名称、允许的稳定属性或可访问语义、匹配数量、可见性与状态；不得把动态 CSS 类名或易变层级位置单独视为稳定契约。缺失、匹配多项或身份校验不一致必须分别返回稳定 finding，发送相关契约存在未知状态时不得发送。

`redacted-samples.jsonl` 只保存复现字段形状所需的最小样本。Cookie、token、localStorage、sessionStorage、认证 header、完整消息正文、用户昵称/标识原值、头像原图、完整 HTML，以及 URL 的 query string 和 fragment 必须在落盘前删除或替换为带类型的摘要；字段名、容器形状、原始类型、长度区间和不可逆摘要可以保留。脱敏失败时不得写入原始样本，page doctor 必须以 `incomplete` 或 `failed` 结束。

诊断包必须可与仓库中的已批准基线进行确定性比较。page doctor 只负责发现和报告候选变化，不得自动修改适配器、覆盖基线或把新字段提升为正式映射；适配器更新必须经过代码修改、fixture 测试和明确的基线更新。若没有可用基线，page doctor 生成初始 inventory 并标记 `baseline_missing`，不得报告“兼容”。

## Local profiles and login

用户必须能够创建或选择一个本地 Profile 标识。浏览器登录态必须保存在项目根目录下的 `Profile/<profile>/browser-data/`。系统必须为不同 Profile 使用隔离的持久化浏览器目录，并阻止两个进程同时持有同一 Profile。

Profile 锁必须记录足以防止 PID 重用误判的占用信息，至少包括 Profile 标识、PID、进程创建时间、Skill 命令和锁创建时间。后启动任务发现有效锁时，必须向用户展示不含敏感参数的进程摘要，并询问是否终止占用进程；不得自动终止。

用户拒绝或取消时，当前任务必须返回稳定代码 `PROFILE_IN_USE`，保留占用进程和原 Profile 数据不变。用户明确确认后，CLI 只能在 PID、进程创建时间和 Skill/Profile 身份均与锁记录一致时终止该占用进程；随后等待锁释放、重新验证 Profile 完整性并自动恢复当前任务的原参数。身份校验失败、终止失败、锁未释放或 Profile 状态异常时必须分别返回稳定错误，不得尝试终止一个仅 PID 相同的进程。已确认不存在对应活动进程的陈旧锁可以在记录诊断后安全清理。

首次登录必须启动有头 Playwright-managed Chromium 或用户明确选择的兼容回退浏览器，打开抖音页面并等待用户人工完成登录。系统必须保存该 Profile 的登录态供后续采集使用。遇到验证码、安全验证或登录失效时，系统必须提示人工处理，不得自动识别、求解或代用户提交验证挑战。

除首次登录、重新登录、验证码、安全验证或其他必须由用户人工处理的恢复流程外，环境/page doctor、登录状态验证、群聊发现、历史采集和单条/批量回复必须默认使用无头 Playwright-managed Chromium，不得自动弹出浏览器窗口。需要人工处理时，当前无头任务必须暂停或失败关闭，随后由明确的登录/验证恢复命令以有头模式打开同一 Profile；验证后恢复原任务参数。页面适配仍必须遵循真人可见交互路径，无头模式不得改用未经验证的隐藏跳转。

同一 Profile 的有头登录与无头任务必须使用同一锁定版本的 Playwright-managed Chromium、同一持久化 `browser-data/`、同一 locale、timezone、viewport 和 device scale。系统必须从该 Profile 的实际有头运行建立并版本化一份不含 Cookie、token 或浏览器存储内容的稳定指纹基线；基线失效或锁定 Chromium 版本变化时必须重新采集，不能静默沿用旧版本结果。

无头任务必须在任何站点脚本执行前完整对齐该有头基线，覆盖以下可观察表面：

- HTTP `User-Agent`、`Sec-CH-UA*` 请求头以及 `navigator.userAgent`、`navigator.appVersion`、`navigator.userAgentData`，其中任何位置均不得包含 `HeadlessChrome`，同一导航内的低熵与高熵 UA-CH 值必须自洽；
- `navigator.webdriver` 及其属性描述行为，结果必须与普通有头浏览器基线一致；
- `navigator.platform`、`navigator.languages`、首选语言请求头、`navigator.hardwareConcurrency`、`navigator.deviceMemory`；
- `screen` 的尺寸、可用区域、色深、像素深度、方向，以及 viewport 与 device scale 之间可推导关系；
- 对固定字体、文本、图形和像素读取用例执行 Canvas 2D/OffscreenCanvas 探针的结果摘要；
- WebGL/WebGL2 vendor、renderer、unmasked vendor/renderer、支持扩展集合和关键参数；
- 常见权限查询的状态与异常形态、`navigator.plugins`/`navigator.mimeTypes` 的长度、顺序、字段和交叉引用关系；
- `MediaSource.isTypeSupported`、`navigator.mediaCapabilities` 和媒体设备能力的归一化结果；
- WebRTC API 可用性、SDP 能力、编解码器与 ICE candidate 类型集合的归一化结果；端口、会话 ID、候选随机串和实时网络地址等每次运行本就变化的值不要求逐字相同，但字段形态、能力集合和地址暴露策略必须与有头基线一致。

这些覆盖必须同时作用于主页面、同源与跨源 frame、dedicated/shared worker 以及可控制的 service worker 执行环境；因浏览器隔离机制不能直接注入的上下文，必须通过浏览器启动配置或协议级设置获得一致结果。不得只修改单一 JavaScript getter 而让请求头、关联 API、属性描述、原型链或不同执行上下文相互矛盾。

同一 Profile 的对齐值在页面、任务、浏览器进程退出、CLI 重启和电脑重启之间必须稳定，不得每次随机生成，也不得跨 Profile 复用与实际有头环境无关的通用设备身份。完整指纹身份必须原子持久化到 `Profile/<profile>/fingerprint.json`，schema 固定为 `douyin-group-message.fingerprint-profile.v1`。文件必须记录 Profile 标识、安全化后的 Chromium 产品版本与 revision、采集时间、最近验证时间、校准状态、各表面的可重放实际配置、动态表面的稳定策略以及整份配置的完整性摘要。

`fingerprint.json` 中的校验摘要只能用于完整性验证，不能替代下次无头运行所需的实际配置。UA/UA-CH、platform、languages、硬件与屏幕参数、permissions、plugins/MIME、媒体能力、Canvas/WebGL 对齐材料及 WebRTC 能力/暴露策略必须以能够确定性重建已校准公开行为的形式持久化；实时内网地址、完整 ICE candidate、端口或会话随机值不得持久化。文件不得包含网页正文、Cookie、token、浏览器存储或其他认证材料。

后续任务必须先读取并验证该文件，通过后直接复用已有身份，不得因普通进程重启、任务类型变化或时间流逝重新生成值或打开校准窗口。只有文件不存在、JSON/schema/必填字段或完整性校验失败、锁定 Chromium 的产品版本/revision 与已校准指纹契约不兼容，或用户显式执行指纹刷新命令时，才允许把基线标记为失效并进入校准流程。兼容性判断必须基于明确版本与探针结果，不得仅因日期变化让基线过期。

系统应复用正常页面会话、保持单 Profile 串行、避免不必要的首页重载和高频重复点击，以减少由环境突变或异常导航造成的额外验证；不得声称消除抖音验证。

系统必须同时提供显式校准主路径和自动校准 fallback。已有 Profile 缺少完整基线、基线 schema 不受支持或锁定 Chromium 版本变化时，无头任务必须先返回 `FINGERPRINT_BASELINE_REQUIRED`，机器可读结果至少包含 Profile 标识、安全化后的当前/基线 Chromium 版本、失效原因、显式有头校准命令和可恢复的原任务摘要；不得使用缺失或过期基线继续访问目标页面。

显式校准命令必须取得同一 Profile 锁，打开一次有头 Playwright-managed Chromium，采集并验证完整基线，原子写入 Profile 元数据后关闭窗口。自动校准 fallback 必须复用同一校准实现，在取得 Profile 锁后打开一次有头窗口；校准成功后关闭窗口并以原 Profile、原命令和全部原参数恢复被中断的无头任务，不要求用户重新输入群聊、时间范围、回复目标或其他已确认参数。两条路径不得并行持有同一 Profile，也不得在一次恢复中重复打开多个校准窗口。

校准窗口被关闭、采集不完整、基线自检不通过、Profile 锁冲突或恢复后的无头契约探针仍不一致时，fallback 必须失败关闭并返回稳定的阶段与差异信息；不得把部分基线标记为有效，不得循环弹窗，也不得在指纹契约未通过时继续原任务。

自动校准 fallback 必须采用混合触发策略。默认情况下，CLI 返回 `FINGERPRINT_BASELINE_REQUIRED` 后不自行打开窗口；Skill 必须展示 Profile、失效原因、将打开一次有头窗口以及原任务将在成功后恢复，并取得一次用户确认。用户在当前任务显式传入 `--auto-calibrate-fallback` 时，该参数构成对本次 fallback 的预授权，Skill 和 CLI 可以在收到基线缺失/过期状态后直接执行一次校准与恢复，不再二次询问。

`--auto-calibrate-fallback` 只对当前顶层任务及其一次恢复链有效，不得写入 Profile 元数据、本地默认配置或环境级配置，也不得被后续任务继承。该参数不能跳过 Profile 锁、发送确认、验证码人工处理或指纹契约验证；恢复原任务时必须保留参数，但恢复链已经消费过一次 fallback 后不得再次触发窗口。

当前任务的 fallback 预授权与持久化指纹身份是两个独立状态：预授权在恢复链结束后清除，但成功校准产生的 `fingerprint.json` 必须保留并供后续任务复用。清除预授权不得删除、重置或随机化持久化指纹。

页面标题、可见契约或响应表明验证码/安全验证时，CLI 必须返回 `VERIFICATION_REQUIRED` 而不是把它误报为普通登录失效，并给出同一 Profile 的有头恢复命令。验证完成后继续使用相同稳定环境执行无头任务。

同一原任务在有头验证成功后最多自动重试无头一次。若重试仍返回 `VERIFICATION_REQUIRED`，Skill 必须停止登录恢复循环并对原任务返回 `HEADLESS_VERIFICATION_PERSISTENT`，附带两次验证时间与原命令摘要；不得持续弹出有头窗口。当前实测基线必须记录：Playwright Chromium 默认 headless、`channel: chromium` 和 Chromium `--headless=new` 均自报 `HeadlessChrome` 并进入抖音验证码中间页，而同一 Profile 的有头模式进入正常首页。

Skill 必须在列群、拉取或回复前验证所选 Profile 的登录状态：

- 没有 Profile 时，显示当前为零个 Profile，收集一个安全的 Profile 名称，创建目录和元数据，再启动有头浏览器人工登录。
- Profile 存在但状态为 `login_required`、`expired` 或 `verification_required` 时，打开同一 Profile 供用户处理，不创建替代 Profile。
- 人工完成后，CLI 必须验证正常抖音页面和消息入口可访问，再把状态更新为 `valid` 并恢复原任务。
- 用户关闭或取消登录流程时，保留 Profile 目录并标记 `login_required`，使后续任务可以继续登录；不得把未验证状态报告为成功。
- Cookie、token、localStorage 和浏览器内部认证文件只存在于 `browser-data/`，不得复制到自定义 JSON、日志或导出目录。

每个 Profile 的可审计元数据保存在 `Profile/<profile>/profile.json`，字段固定为：

```json
{
  "schema": "douyin-group-message.profile.v1",
  "profile_id": "string",
  "display_name": "string",
  "browser_data_dir": "browser-data",
  "created_at": "RFC3339 timestamp",
  "updated_at": "RFC3339 timestamp",
  "last_login_verified_at": "RFC3339 timestamp or null",
  "login_status": "login_required | valid | expired | verification_required",
  "last_error_code": "string or null"
}
```

`profile_id` 必须经过确定性路径安全编码；`browser_data_dir` 必须是 Profile 目录内的相对路径。`profile.json` 必须使用同目录临时文件和原子替换。

## Conversation discovery

系统必须从当前登录账号动态读取可见会话，并区分群聊与普通私信。群聊选择必须优先使用页面或消息数据中的稳定会话标识；显示名称只能用于展示和显式消歧，不能在名称重复时静默选择。

列群结果必须至少包含 Profile 标识、群聊显示名称和可供后续命令使用的会话标识。

## Human-visible conversation workflow

当前页面的真人路径是：用户从顶部“消息”入口打开会话面板；会话列表同时包含私信和群聊；用户进入一个群聊后，主区域显示最近消息，向上滚动加载更早消息，底部固定输入区；对某条消息操作时，页面提供该消息对应的“回复”入口，引用对象随后显示在发送区域。

采集和回复适配器必须优先复用这些可见交互和页面提供的稳定属性。系统不得把“打开群聊”实现成直接跳过会话列表的隐藏跳转而宣称等同于真人路径；仍可用稳定会话标识加速定位，但必须验证最终群聊标题和会话身份。

## Historical message collection

用户必须能够指定一个或多个群聊以及开始、结束时间。系统必须逐个打开群聊并按需向上加载历史消息，直到覆盖开始时间、确认没有更早消息，或遇到可报告的失败条件。

导出的每条规范化记录必须至少包含：

- Profile 标识；
- 群聊名称与会话标识；
- 消息标识；
- 发送时间；
- 发送人昵称及可获得的发送人标识；
- 消息类型；
- 文本内容；
- 零个或多个图片 URL；
- 采集时间和来源说明。

时间范围之外的消息不得进入结果。系统必须保留可用于定位引用回复目标的稳定字段，并明确报告无法解析精确时间、发送人或消息标识的记录，而不是伪造值。

图片 URL 必须优先取自结构化消息数据；只有结构化数据不可用时才回退到 DOM 的实际资源 URL。系统必须标记可能过期的签名或缩略图 URL。

## Project data layout

系统默认必须在运行项目内维护本地数据工作区，而不是把历史消息和回复记录散落到全局目录。不同 Profile 的数据必须隔离；每个群聊必须拥有独立文件夹，文件夹内保存该群聊的历史消息导出、采集元数据、回复计划与回复结果记录。

群组文件夹必须使用稳定会话标识参与命名，群聊显示名只能作为可读部分。系统必须处理同名群聊、群聊改名、非法路径字符和超长文件名；不得因为显示名重复而把两个群聊的数据写入同一目录。

默认数据布局必须是 `.douyin-group-message/profiles/<profile>/groups/<群名>__<会话ID>/`。其中 `<profile>` 必须由 Profile 标识安全编码；`<群名>` 是经过路径安全化的当前显示名；`<会话ID>` 是经过路径安全化的稳定会话标识。群聊改名后，系统必须依据稳定会话 ID 找到既有目录并安全更新其可读名称，不得创建第二份逻辑群组数据。

同一群组多次拉取时，系统必须按稳定消息 ID 把新结果增量合并到该群组的一份持续历史中。已存在的消息 ID 不得生成重复记录；后续采集补全原先缺失字段时可以合并补全，但同一字段出现两个不同的非空值时必须记录冲突并保留可审计信息，不得静默覆盖。

每个群组目录必须使用 JSONL 保存主要数据：`messages.jsonl` 每行是一条规范化消息，表示去重合并后的当前群历史；`replies.jsonl` 每行是一条追加式审计事件，记录回复计划、用户确认、发送尝试和最终结果。群聊身份、显示名、采集游标和格式版本等元数据必须保存于同目录的 `group.json`。

`messages.jsonl` 的合并更新必须通过同目录临时文件和原子替换完成，失败时保留上一份完整文件；`replies.jsonl` 必须只追加完整 JSON 行。读取时遇到损坏或不完整行必须失败并报告位置，不得静默丢弃。所有 JSONL 记录必须包含可迁移的 schema version。

项目内数据工作区、Profile 数据、导出、截图、回复审计记录、临时锁和本地配置必须默认被版本控制忽略，且不得包含在 Skill 发布包中。

Profile 数据根固定包含以下结构：

```text
.douyin-group-message/profiles/<profile>/
  groups.json
  runs/
    <run-id>.json
  groups/
    <群名>__<会话ID>/
      group.json
      messages.jsonl
      replies.jsonl
      conflicts.jsonl
```

`groups.json` 是最近一次成功刷新得到的群聊索引：

```json
{
  "schema": "douyin-group-message.groups-index.v1",
  "profile_id": "string",
  "refreshed_at": "RFC3339 timestamp",
  "count": 2,
  "groups": [
    {
      "conversation_id": "string",
      "display_name": "string",
      "folder_name": "string",
      "last_message_at": "RFC3339 timestamp or null",
      "discovery_status": "active | unavailable"
    }
  ]
}
```

`count` 必须等于 `groups` 数组长度。群组按页面发现顺序保存，Skill 展示的编号只绑定本次 `refreshed_at` 对应的索引；刷新后旧编号不得静默复用。

每个 `group.json` 固定为：

```json
{
  "schema": "douyin-group-message.group.v1",
  "profile_id": "string",
  "conversation_id": "string",
  "display_name": "string",
  "folder_name": "string",
  "discovered_at": "RFC3339 timestamp",
  "updated_at": "RFC3339 timestamp",
  "last_successful_run_id": "string or null",
  "oldest_collected_at": "RFC3339 timestamp or null",
  "newest_collected_at": "RFC3339 timestamp or null",
  "message_count": 0
}
```

`messages.jsonl` 每行固定包含：

```json
{
  "schema": "douyin-group-message.message.v1",
  "profile_id": "string",
  "conversation_id": "string",
  "conversation_name": "string",
  "message_id": "string",
  "sent_at": "RFC3339 timestamp or null",
  "sender": {
    "id": "string or null",
    "name": "string or null"
  },
  "type": "text | image | mixed | system | unsupported",
  "text": "string or null",
  "images": [
    {
      "url": "string",
      "source": "structured | dom",
      "may_expire": true
    }
  ],
  "reply_to_message_id": "string or null",
  "collected_at": "RFC3339 timestamp",
  "run_id": "string",
  "source": "douyin-web",
  "parse_warnings": ["string"]
}
```

`message_id` 是主去重键。无法取得稳定消息 ID 的记录不得写入 `messages.jsonl`，必须以错误或冲突事件写入 `conflicts.jsonl`。消息按 `sent_at` 升序保存；`sent_at` 为空时按 `collected_at` 排序，并在 `parse_warnings` 标记。

`replies.jsonl` 每行固定包含一个不可变审计事件：

```json
{
  "schema": "douyin-group-message.reply-event.v1",
  "event_id": "string",
  "plan_id": "string",
  "task_id": "string",
  "occurred_at": "RFC3339 timestamp",
  "profile_id": "string",
  "conversation_id": "string",
  "target_message_id": "string",
  "event_type": "planned | confirmed | send_started | sent | failed | skipped",
  "reply_text": "string",
  "attempt": 1,
  "result": {
    "status": "pending | sent | failed | skipped",
    "sent_message_id": "string or null",
    "reason_code": "string or null",
    "message": "string or null"
  }
}
```

同一 `event_id` 不得重复追加；一次任务的事件按实际发生顺序追加。日志不得包含 Cookie、token、浏览器存储或完整页面 HTML。

每次群聊刷新或历史拉取写入 `runs/<run-id>.json`：

```json
{
  "schema": "douyin-group-message.collection-run.v1",
  "run_id": "string",
  "profile_id": "string",
  "requested_at": "RFC3339 timestamp",
  "started_at": "RFC3339 timestamp",
  "completed_at": "RFC3339 timestamp or null",
  "from": "RFC3339 timestamp",
  "to": "RFC3339 timestamp",
  "selected_groups": ["conversation-id"],
  "status": "running | completed | partial | failed | cancelled",
  "totals": {
    "groups_selected": 0,
    "groups_succeeded": 0,
    "groups_failed": 0,
    "messages_new": 0,
    "messages_duplicate": 0,
    "messages_conflict": 0,
    "messages_skipped": 0
  },
  "groups": [
    {
      "conversation_id": "string",
      "status": "pending | running | completed | failed | skipped",
      "messages_new": 0,
      "messages_duplicate": 0,
      "messages_conflict": 0,
      "messages_skipped": 0,
      "error_code": "string or null",
      "output_dir": "project-relative path or null"
    }
  ]
}
```

Run 文件在执行开始时原子创建并在每个群完成后原子更新；进程中断后保留 `running` 状态，下一次启动必须把它报告为可恢复或可重新执行，不得伪造成 `completed`。

`conflicts.jsonl` 使用 `douyin-group-message.conflict.v1`，至少包含 `conflict_id`、`occurred_at`、`run_id`、`conversation_id`、可获得的 `message_id`、`field`、`existing_value`、`incoming_value` 和 `reason_code`。其中任何字段可能包含认证值时必须改为安全摘要而不是原值。

## Reply to a specific message

用户必须能够以已采集记录中的会话标识和消息标识指定回复目标，并提供文本回复。系统必须重新打开目标群聊、唯一定位目标消息、触发页面提供的引用回复交互并验证引用对象与待发送文本。

发送前必须展示目标群聊、目标发送人、目标时间、目标消息摘要和回复文本，并遵循用户确认的发送策略。目标不唯一、目标已不可加载、引用对象不一致或页面结构未知时不得发送。

回复必须同时支持单条和批量任务。单条回复是默认模式；批量回复必须由用户显式启用，并提供由目标记录和回复文本组成的有限任务清单。批量任务必须严格按照计划顺序逐条执行，同一 Profile 不允许两个并发发送动作。

批量任务必须先展示完整发送计划，包括每个目标群聊、目标消息摘要、回复文本和发送顺序。用户整批确认一次后，系统按计划顺序连续串行发送；任一风控、验证码、引用不一致、发送失败或结果无法确认都会中断剩余任务，并把已发送、未发送和失败原因写入回复记录。

用户已确认：批量发送不采用预设的“每小时 N 条”脚本节奏；每条任务都必须复用真人路径，先进入或验证对应群聊、查看目标上下文、触发该消息的引用回复、发送一次并确认结果，再处理下一条。

## Sending safety

系统必须为每个 Profile 建立跨进程互斥和持久化发送节流状态。具体节奏必须遵循已确认的真人交互策略，不得让多个调用绕过互斥或在未完成结果确认时继续发送。

出现验证码、安全验证、发送失败、限流提示、登录失效或无法确认发送结果时，系统必须停止当前发送流程并进入冷却或人工处理状态，不得自动高速重试。

系统不得在列群、采集或测试命令中发送消息。真实回复测试必须由用户明确指定目标和内容，并完成发送前确认。

## Output and privacy

采集结果必须写入用户指定或本地默认的项目内群组目录，并提供机器可读格式。日志必须输出可操作的状态和错误，但不得输出 Cookie 值、认证令牌或完整浏览器存储。

Profile 数据、导出、截图、回复审计记录、临时锁和本地配置必须默认被版本控制忽略，且不得包含在 Skill 发布包中。

## Acceptance scenarios

### New user login

给定一台已安装受支持 Node.js 的电脑且项目内没有 `Profile/`，当用户请求列群时，Skill 显示当前为零个 Profile，收集任意安全名称，在 `Profile/<profile>/` 创建元数据与 `browser-data/`，用 Playwright-managed Chromium 打开有头浏览器；人工登录并通过消息入口验证后，自动恢复原列群任务。

给定 `Profile/<profile>/` 已存在但登录状态过期，当用户请求拉取已选群聊和时间范围时，Skill 打开同一 Profile 供人工重新登录；验证成功后继续原群聊和时间范围，不要求重新输入，且不创建替代 Profile。

### Environment setup and recovery

给定分享包只有锁文件而没有 `node_modules`，当用户首次请求列群时，环境 doctor 返回缺少精确 Playwright 依赖，setup 在 Skill 目录执行本地 `npm ci`；若对应 Chromium 缺失，再安装锁定 Playwright 所需的 Chromium，重新 doctor 通过后自动恢复列群任务。

给定 Node.js/npm 缺失、Chromium 下载网络失败、目录无写权限或系统依赖缺失，当 setup 无法完成时，结果包含稳定 issue、失败步骤和可重试命令，原浏览器任务保持未执行；部分安装不得被报告为成功。

给定登录态有效，当用户运行 doctor、列群、采集或回复时，Playwright-managed Chromium 以无头模式运行且不显示浏览器窗口；只有创建 Profile、登录失效或人工验证恢复时才打开有头浏览器，完成后原任务回到无头模式继续。

给定同一 Profile 和同一锁定 Chromium 版本，当系统先在有头模式采集指纹基线、再以无头模式运行契约探针时，请求头和 JavaScript UA 均与有头基线一致且不含 `HeadlessChrome`，`navigator.webdriver` 呈现普通有头浏览器值，固定 Canvas 探针以及 WebGL vendor/renderer 与有头基线逐项一致；在主页面与 frame 中重复探测结果相同。

给定完整指纹基线，当系统比较有头与无头契约探针时，UA-CH、platform、languages、hardwareConcurrency、deviceMemory、screen、permissions、plugins/MIME、media capabilities 和 WebRTC 归一化结果也必须逐项通过；差异报告必须指出表面、路径、基线摘要和实际摘要，不得把缺失探针视为通过。

给定页面在主文档、frame 或 worker 中读取同一个受控指纹表面，当该值按浏览器契约应相同时，各执行环境的值和属性形态必须一致；测试必须覆盖至少一个跨源 frame，并记录浏览器隔离导致只能通过启动配置对齐的项目。

给定同一 Profile 连续启动两次无头任务，当没有重新采集有头基线或升级 Chromium 时，两次公开指纹结果必须相同；给定两个 Profile 分别具有不同的有头基线时，无头任务不得把一个 Profile 的基线套用到另一个 Profile。

给定已有 Profile 缺少基线或 Chromium 升级使基线过期，当无头任务启动时，CLI 返回 `FINGERPRINT_BASELINE_REQUIRED`、失效原因、显式校准命令和原任务摘要，且尚未访问目标页面；用户运行显式命令后，系统以有头模式生成完整基线，随后原任务可使用该基线继续。

给定自动校准 fallback 已按确认策略启用，当无头任务收到 `FINGERPRINT_BASELINE_REQUIRED` 时，Skill 只打开一次有头校准窗口；成功后关闭窗口并使用完全相同的原任务参数恢复无头执行。若用户关闭窗口或任一指纹表面校验失败，原任务保持停止并返回校准失败结果，不再次弹窗。

给定任务未传入 `--auto-calibrate-fallback`，当收到 `FINGERPRINT_BASELINE_REQUIRED` 时，Skill 展示校准与恢复计划并等待一次确认，确认前不打开窗口；给定同一任务已显式传入该参数时，Skill 不再二次询问并直接执行一次 fallback。无论成功或失败，后续新任务都不得自动继承这次预授权。

给定某 Profile 已成功校准并写入 `fingerprint.json`，当浏览器进程、CLI 或电脑重启后执行新的无头任务时，系统读取并复用同一份可重放配置，契约探针结果与校准值一致，且不返回 `FINGERPRINT_BASELINE_REQUIRED`、不打开有头窗口、不生成新的设备身份。

给定 `fingerprint.json` 被截断、schema 未知、必填表面缺失、完整性摘要不匹配或 Chromium 指纹契约不兼容，当无头任务启动时，系统在访问目标页面前返回具体失效原因并进入已确认的显式校准/fallback 流程；原文件保留供诊断，不得静默覆盖。给定只是跨日或普通重启且版本与探针兼容时，文件继续有效。

### Page contract drift

给定已批准的页面契约基线，当抖音更新 bundle 并新增字段、删除关键字段、改变字段类型或可空性、或移除真人路径依赖的稳定 DOM 属性时，page doctor 生成新的 bundle 内容指纹、字段级 diff、DOM finding 和脱敏样本，并以 `drift_detected` 结束；诊断运行不发送消息，也不自动修改正式字段映射或基线。

给定 bundle URL 只有认证 query 参数变化而内容不变，当再次运行 page doctor 时，清理后的资源路径和内容哈希保持一致，不得仅因签名 URL 变化报告 bundle 漂移。

给定脱敏器无法证明样本中不存在认证值或完整消息正文，当 page doctor 写诊断包时，原始样本不得落盘，运行状态为 `incomplete` 或 `failed` 并返回稳定错误。

### Select and export groups

给定当前账号存在多个群聊，当用户列群并选择其中两个会话标识，再提供时间范围时，系统只读取这两个群聊，并导出范围内的规范化消息记录。

### Guided group selection

给定用户只要求“拉取群消息”，当当前 Profile 存在十二个群聊时，Skill 显示“发现 12 个群聊”和编号列表；用户输入 `1,3-5` 后，Skill 回显已选择四个群聊，再收集缺失的时间范围并展示拉取计划。

给定当前 Profile 未发现群聊时，Skill 显示“发现 0 个群聊”和可操作的检查结果，并停在群聊发现步骤；不得要求用户输入不存在的编号。

### Collection progress and summary

给定用户选择三个群聊，当第二个群聊拉取失败时，Skill 继续第三个群聊；最终汇总显示两个成功、一个失败，并为每个成功群列出新增、重复、冲突记录数和输出目录，为失败群列出稳定会话标识与错误摘要。

### Export image messages

给定范围内存在图片消息，当系统导出该消息时，记录包含消息类型和一个或多个可获取的图片 URL，并注明 URL 来源或有效性限制。

### Group data folders

给定用户为两个群聊拉取历史消息，当两个群聊名称相同或后来改名时，系统仍为每个稳定会话标识维护独立群组文件夹；每个文件夹内包含该群聊的历史消息文件和回复记录文件。

给定默认配置和 Profile `work`，当群聊“产品讨论”的稳定会话标识为 `conversation-123` 时，其数据写入 `.douyin-group-message/profiles/work/groups/产品讨论__conversation-123/`；路径组成部分包含操作系统不允许的字符时，系统必须进行确定性的安全编码。

给定同一群聊的一条消息先后被采集两次，当两次记录具有相同稳定消息 ID 时，群历史中只保留一条逻辑消息；第二次采集补全的缺失字段被合并，而不同的非空字段值被记录为冲突。

给定拉取过程中程序异常退出，当用户再次读取群组目录时，`messages.jsonl` 仍是更新前或更新后的完整 JSONL，不得是部分覆盖文件；已完成的回复审计事件仍以完整行存在于 `replies.jsonl`。

给定上一次 `runs/<run-id>.json` 仍为 `running`，当 CLI 再次启动时，系统报告该 Run 可恢复或可重新执行，并保持已完成群组的统计；不得把中断 Run 静默改为 `completed`。

给定任一 JSON 或 JSONL 文件包含未知 schema、缺失必填字段或不完整 JSON 行，当 CLI 读取它时，系统返回文件路径、行号（适用时）和稳定 schema 错误，不得忽略损坏记录或继续覆盖原文件。

### Reply safely

给定一条已导出的消息记录和回复文本，当系统能唯一加载目标且用户完成所需确认时，系统建立正确引用、按节流策略发送一次并验证结果；若目标不唯一或出现风控页面，则不发送并返回失败。

### Human-like batch navigation

给定已确认的批量计划，系统必须对每个目标依次进入对应群聊或已验证的群聊视图，确认目标消息和引用对象，发送一次并确认结果后才进入下一项；任一目标无法唯一定位或结果不确定时，剩余计划立即停止。

### Concurrent use

给定两个进程使用同一 Profile，当后启动任务检测到有效锁时，Skill 展示经 PID、创建时间和 Skill/Profile 身份校验的占用进程摘要并询问是否终止；用户拒绝时返回 `PROFILE_IN_USE` 且占用进程继续运行，用户确认时只终止该已校验进程，锁释放和 Profile 完整性复核通过后自动恢复后启动任务的原参数。

给定锁中的 PID 已被操作系统复用于其他进程，或进程身份与锁记录不一致，当用户确认终止时，CLI 不终止该进程并返回身份校验错误；不得仅凭 PID 杀进程。

### Explicit batch mode

给定包含多条目标消息的回复计划，当用户未显式启用批量模式时，系统不得接受该计划；显式启用后，系统必须按原计划顺序逐条处理且始终保持最多一个发送动作。
