# doudou

## 交付边界

系统必须以 `doudou` 名称交付标准 Codex Skill 和确定性 Node.js CLI。Skill 安装目录只包含发布清单声明的代码、指令、参考文档、测试和锁文件，不得包含用户 Profile、消息、发送回执、诊断、`analyze/`、`node_modules/` 或浏览器二进制。

CLI 必须从调用者当前项目运行。默认 data root 必须是 `process.cwd()`；只有显式的 `--data-root` 或 `DOUDOU_HOME` 才能覆盖。用户登录态必须写入 `profile/<profile>/`，生产记录必须写入 `message/profiles/<profile>/`，不得写入已安装 Skill 目录。仅存在旧 `Profile/` 时必须一次性迁移；新旧目录同时存在且不是同一目录时必须停止并报告冲突。

产品重命名采用不兼容切换。新版只接受 `doudou.*` schema、`doudou` 锁 owner、`doudou` CLI 和 `DOUDOU_HOME`；不得保留旧产品名的 CLI、Skill、bin、环境变量或 schema 兼容入口。项目中既有的旧格式 `profile/` 与 `message/` 文件不得被自动改写或删除，新版必须把它们视为不支持的旧数据并返回稳定错误，用户通过新版命令重新生成元数据。

### 场景：旧运行记录

给定 `profile/` 或 `message/` 中存在旧产品 schema/owner，当新版读取该记录时，必须返回包含文件路径和旧格式说明的稳定错误；不得迁移、覆盖、静默接受或删除原文件。

## 历史证据边界

`docs/comet/archive/`、`analyze/`、旧发送回执、旧诊断和其他已有运行证据必须保持创建时的内容、名称和哈希，不参与产品品牌机械替换。旧品牌残留验证必须把这些不可变历史路径列为明确排除项，而不是把它们报告为当前产品残留。

### 场景：从代理目录调用 Skill

给定 Skill 安装在 `~/.agents/skills/doudou/`，且用户在另一个空项目运行 doctor，当 CLI 初始化数据根时，只在该用户项目创建 `profile/` 与 `message/`；Skill 安装目录不出现运行数据目录。

### 场景：打包交付

给定交付目录已准备完成，当检查顶层内容时，只包含 `agents/`、`bin/`、`references/`、`scripts/`、`src/`、`test/`、`.gitignore`、`npm-shrinkwrap.json`、`package.json` 和 `SKILL.md`，且包内不存在凭据或开发者 Profile。

## 环境检测与安装

doctor 必须以机器可读 NDJSON 检测 Node.js >= 20、npm、Skill 本地精确 Playwright `1.62.0`、匹配的 Playwright-managed Chromium、浏览器实际启动能力以及用户项目目录写权限。缺失项必须返回稳定 issue 和修复命令。

setup 必须在 Skill 安装目录执行锁文件安装，并安装该 Playwright 版本对应的 Chromium；不得依赖全局 Playwright。安装后必须重新 doctor，成功后才能恢复原浏览器任务。

### 场景：首次使用没有依赖

给定分发包没有 `node_modules`，当用户首次运行 doctor 时，结果为 `setup_required` 并指向 setup；setup 完成后再次 doctor，Playwright 版本、Chromium 路径和实际启动探测均通过。

### 场景：安装失败

给定 npm 缺失、网络下载失败、目录不可写或 Chromium 无法启动，当 setup 或 doctor 运行时，结果明确失败步骤与可重试命令，且原登录、拉取或发送任务保持未执行。

## Profile、登录与指纹

每个命名 Profile 必须位于 `profile/<profile>/`，包含非敏感元数据、`browser-data/`、持久化 `fingerprint.json` 和运行期锁。登录、重新登录、指纹校准、人工验证和协议 capture 使用有头 Playwright-managed Chromium；doctor、会话发现、消息拉取、协议发送和 page doctor 默认无头。

无头运行必须重放同一 Profile 从实际有头 Chromium 取得的稳定指纹，包括 UA/UA-CH、`navigator.webdriver`、Canvas、WebGL、platform、languages、hardwareConcurrency、deviceMemory、screen、permissions、plugins/MIME、media capabilities 和可归一化 WebRTC 表面。普通跨日、进程退出或电脑重启不得使兼容基线失效或生成新身份。

### 场景：首次登录并复用

给定不存在的命名 Profile，当用户创建并登录时，CLI 打开有头 Chromium，保存登录态和指纹基线；后续无头任务使用同一 browser-data 与 fingerprint，UA 不含 `HeadlessChrome`，受控指纹探针与基线一致。

### 场景：基线失效

给定 fingerprint 缺失、损坏、schema 不兼容、Chromium 契约不兼容或探针不一致，当无头任务启动时，CLI 在访问目标页面前返回 `FINGERPRINT_BASELINE_REQUIRED` 和显式校准命令。只有当前任务预授权自动 fallback 时，系统才打开一次有头校准并恢复原参数；该授权不得持久化到后续任务。

### 场景：Profile 并发占用

给定另一个有效进程持有同一 Profile，当新任务启动时，CLI 返回经校验且脱敏的 owner 信息。只有用户明确确认后才能终止匹配 owner 并恢复原任务；PID 身份不匹配时不得终止进程。

## 群聊和私聊发现

会话刷新必须支持 `all`、`group` 和 `direct`。分类必须优先使用 Protobuf `conversation_type`：`1` 为私聊，`2` 为群聊；只有结构化类型不可用时，才使用有界的明确 DOM 属性或头像结构。不得从标题文本分类，不得深遍历 React Fiber、DOM 内部对象或任意对象图。

刷新结果必须保存稳定 conversation ID、kind、显示名和 `refreshed_at`。编号选择只允许引用同一刷新快照，支持单个、逗号列表、范围和“全部”；越界、重复、空选择或过期快照必须返回明确结果，不得静默改选。

### 场景：混合会话列表

给定列表中同时存在 `conversation_type=1` 与 `conversation_type=2` 的会话，当刷新 `--kind all` 时，两类均返回；标题是否包含“群”字不改变协议类型。

### 场景：快照编号选择

给定一次刷新返回五个会话，当用户以该 `refreshed_at` 选择 `1,3-5` 时，CLI 按原顺序返回四个稳定 ID；使用其他刷新时间的编号时失败关闭。

## 消息拉取与字段处理

系统必须按稳定 conversation ID 和 RFC3339 `from`/`to` 拉取群聊或私聊消息。每条规范化记录必须保存 schema、稳定消息 ID、conversation ID/kind、发送时间、发送者、消息类型、文本、图片列表以及可审计的协议元数据。

未知字段必须进入 field inventory 与脱敏样本。已知消息类型应从 Protobuf 字段映射；关键字段缺失、改型或结构冲突不得由 DOM 文本静默替代。消息拉取不得打开会话 DOM、滚动消息列表、读取可见文本或遍历 React 对象。

消息存储必须按 `message/profiles/<profile>/conversations/<kind>__<display-name>__<conversation-id>/` 隔离。重复稳定消息 ID 进行增量合并：补齐缺失字段，记录非空冲突，不留下部分 JSONL。每次拉取必须在 `runs/` 保存状态和逐会话统计；只读任务中单个会话失败不阻断其余会话。

图片消息必须从 Protobuf `content.resource_url` 取得资源和 `skey`，下载密文并按 `12-byte IV + ciphertext + 16-byte authentication tag` 使用 AES-256-GCM 解密。明文必须校验声明长度、图片格式和顶层 `content.md5`，再原子写入 `media/YYYY-MM-DD/<message-id>/`。消息图片项必须记录项目相对 `local_path`、下载状态、格式、字节数、MD5 和 SHA-256。单张图片失败不得留下部分明文文件，也不得阻断同一会话其他消息或其他会话；后续 collect 必须能够重试失败图片。

### 场景：拉取群聊和私聊

给定一个群聊和一个私聊的稳定 ID及同一时间范围，当用户执行 collect 时，系统依次拉取两者并写入独立 conversation 目录，最终 Run 报告每个会话的新增、重复、冲突、跳过和失败数量。

### 场景：重复与字段升级

给定同一消息先以通用类型保存、后续从 Protobuf 取得具体类型和图片列表，当再次合并时，记录升级为更具体的非空字段；两个不同非空值同时存在时写入 conflicts，不得静默覆盖。

### 场景：图片随消息拉取

给定时间范围内存在 `message_type=27` 图片消息，当 collect 完成时，消息 JSONL 与解密后的本地图片同时存在，图片记录中的相对路径、格式、字节数和 hash 与文件一致；重复 collect 不重复下载已通过完整性校验的相同资源。

### 场景：图片完整性失败

给定下载失败、GCM tag 无效、声明长度不匹配或顶层 MD5 不匹配，当 collect 处理该图片时，消息仍保存，图片项记录稳定失败码和可重试状态，不覆盖既有有效文件，也不停止其余消息。

### 场景：损坏数据

给定 JSONL 最后一行不完整、schema 未知或必填字段缺失，当 CLI 读取时，结果包含文件与行号并停止覆盖原文件。

## 协议文本、引用回复、@ 与图片发送

普通发送、引用回复、群聊 @ 和图片发送必须复用当前登录 Profile，并直接构造发送端 Protobuf。业务链路不得填写输入框、点击发送按钮、触发键盘事件、使用文件 input、打开会话 DOM 或拦截一条由 UI 生成的 bootstrap 发送请求。最终请求必须由 `/chat` 页面上下文 `fetch` 发出，使页面网络层提供 Cookie、设备参数及 `a_bogus`、`msToken`、`fp`、`verifyFp` 等动态参数。读取接口的 Protobuf 响应不得直接反向当作发送接口。

`protocol send` 必须接受互斥 payload：`--text` 或 `--image`。两者同时存在或均不存在必须在预览前失败。`--at` 可重复或使用逗号列表，但仅可与群聊文本 payload 一起使用。成员必须从协议成员目录解析为稳定 UID；名称歧义返回编号候选，不得猜测。正文必须按 `@显示名 ` 前缀组织，mention 和字体 metadata 使用相同 UTF-16 location/length，发送 body 的 mention UID 字段必须同步写入。

图片发送必须通过页面上下文取得 `/aweme/v1/web/im/upload/config/v2` 的 `public_image_config`，按页面实际 SDK 的 AWS4-HMAC-SHA256 canonical 规则执行 `ApplyUploadInner` 和 `CommitUploadInner`，使用服务端 Auth 执行 TOS 二进制上传，再通过页面上下文调用 `batch_build_image`。最终发送内容必须为 `message_type=27`、`aweType=2702`，包含服务端返回的 URI、SecretKey、尺寸、明文 MD5、原始字节数和构建后的 URL 列表。

第一次发送调用必须只生成包含目标会话、模式、正文摘要、引用消息 ID（适用时）和 plan ID 的 `confirmation_required` 结果。只有第二次调用携带匹配且未漂移的 `--confirm --plan-id` 才能发送。发送必须严格串行，并验证 HTTP 状态、服务端 result、新 client message ID 回显和当前选中会话。

每次实际发送必须在 `message/profiles/<profile>/sends/YYYY-MM-DD/<timestamp>__<client-id>/` 保存编号的 CLI 请求、服务端响应、receipt 和 verification；图片发送还必须保存不含临时凭证或完整签名 URL 的上传阶段摘要。不能仅凭 DOM 可见文本报告成功。

### 场景：普通协议发送预览

给定唯一 conversation ID 和文本，当用户第一次调用 protocol send 时，不产生网络发送，只返回 plan。使用匹配 plan ID 明确确认后，系统发送一次并生成通过或失败的 verification。

### 场景：协议引用回复

给定已采集且唯一的目标消息，当用户调用 protocol reply 时，plan 包含 conversation ID、message ID 和正文；确认后编码引用字段并验证服务端回执。目标不存在、歧义、plan 漂移或风控页面出现时不发送。

### 场景：协议群聊 @

给定唯一群聊、可唯一解析的成员和正文，当用户确认发送时，CLI 直接构造带 UID 和 UTF-16 区间的 Protobuf 并发送；服务端回执通过后报告 sent。私聊、成员不存在或成员重名时不发送。

### 场景：协议图片发送

给定可读的 PNG、JPEG 或 WebP 文件和唯一会话，当用户确认发送时，CLI 依次验证 Apply、TOS upload、Commit、image build 和 message send 的真实结果；任一阶段失败时报告对应稳定阶段码且不得伪造最终 sent。

## 协议抓包与页面诊断

protocol capture 必须使用同一 Profile 打开有头浏览器，允许用户完成一次正常普通发送或引用回复，并捕获相应请求/响应。manifest 必须移除敏感 query 和 header 值；二进制 body、字段清单和证据仅保存到用户项目的 diagnostics 目录。

page doctor 必须只读检查 bundle 内容指纹、结构化字段、稳定 DOM 属性和脱敏样本，报告字段新增、删除、改型和可空性变化。它不得发送消息，不得自动修改正式字段映射，不得在无法证明脱敏时保存原始样本。

### 场景：发送协议漂移

给定页面发送字段已变化，当用户完成一次有头 capture 时，系统保存脱敏 manifest 与二进制请求/响应供更新 builder；若没有捕获到完整响应，记录 incomplete 而非伪造成功。

### 场景：页面契约漂移

给定 bundle 内容或关键字段发生变化，当 page doctor 运行时，输出字段级 findings 和脱敏证据目录，整个运行不产生发送行为。

## Skill 编排与恢复

Skill 必须从用户项目运行 CLI，把 stdout 视为 NDJSON，并在每个任务开始时依次处理 doctor、setup、Profile 选择和登录状态。已确定的 Profile、conversation ID、时间范围和原任务参数必须在一次恢复中保留，不得要求用户重复输入。

遇到 `PROFILE_IN_USE` 时必须先确认终止；遇到 `FINGERPRINT_BASELINE_REQUIRED` 时按校准契约处理；遇到 `LOGIN_REQUIRED` 或 `VERIFICATION_REQUIRED` 时打开有头登录/人工验证，完成后只重试原无头任务一次。

### 场景：登录失效后恢复

给定用户已选定会话和时间范围，但 Profile 登录失效，当 collect 启动时，Skill 打开同一 Profile 供人工登录；验证完成后以原参数恢复一次无头 collect，不重新询问已确认参数。

### 场景：交付验证不发送真实消息

给定维护者执行自动化交付验证，当测试普通发送或引用回复时，只验证预览、请求构造 fixture、确认计划和回执判定；没有用户明确指定真实目标和文本时，不执行线上发送，并在报告中记录该限制。
