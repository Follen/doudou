# Outcome

Doudou 的正式 Skill 具备简洁、可执行且与当前 CLI 一致的长期规格；项目删除已结束迁移期的 legacy 兼容实现、命令、文档和测试，只保留 npm 托管运行时与当前 Profile/数据契约。清理后以自动化门禁和已登录 Profile 的真实浏览器协议回归证明当前主链路可用。

# Scope

- 把当前 `doudou/SKILL.md` 的入口选择、doctor/setup 门禁、Profile 选择、NDJSON 投影、读写约束和诊断路由写成完整目标规格。
- 固化 `messages collect` 单会话参数和群聊 command 301 live 诊断必须使用 `conversations refresh --force` 的行为。
- 删除旧缓存根自动迁移/卸载、`Profile/` 根迁移、Chromium Profile 自动备份迁移、`identity migrate` 命令及其专用实现和迁移测试；只保留返回 `LEGACY_UNSUPPORTED` 所需的最小只读识别与拒绝测试。
- 清理 canonical spec 中已经过时的系统 Python、旧 Node 生产入口保留、旧 Profile 自动迁移等要求。
- 重建 Skill manifest，并执行 Node、Python、Ruff、Skill、pack/install/release 等全部本地门禁。
- 使用现有已登录且空闲的 Profile 完成真实 doctor、Profile 选择、live 会话刷新、群聊/私聊读取、固定窗口重复采集与去重存储回归；不输出敏感业务内容。

# Non-goals

- 不删除或改写 `docs/comet/archive/` 中的历史归档和内容寻址证据。
- 不删除当前 npm 启动器内部必需的 Python bootstrap，除非调用图调整后有等价的当前入口。
- 不改变聊天、视频和评论写入的 plan/confirm 安全语义，不在线构造新的业务写入。
- 不发布新 npm 版本、不创建 tag。

# Acceptance examples

- 给定安装了 npm CLI，当 Agent 执行读取任务时，Skill 只选择 `doudou` 或 npx 入口，依次完成 doctor 与确定性 Profile 选择，并逐行解析 NDJSON 后只投影必要字段。
- 给定需要诊断群聊 command 301，当会话索引存在缓存时，标准诊断运行 `conversations refresh --force` 并取得 `source=live`，再使用单个 `--conversation` 固定窗口采集。
- 给定代码仓库完成清理，当检查旧缓存、顶层 `Profile/`、Chromium Profile 和 identity 历史记录时，正式源码、CLI help 与当前 references 不再包含迁移、删除或 deprecated 命令；活动测试只验证直接拒绝且数据不变。
- 给定当前格式的 Profile 和托管运行时，当运行全量自动化与真实只读回归时，全部门禁通过，live 群聊/私聊采集成功，重复采集产生稳定 duplicates 且 conflicts 为零。

# Constraints and invariants

- 正式业务入口保持 `doudou` 与 `npx --yes @follenfang/doudou`；npm 继续管理 Skill 安装和专用 Python/Camoufox runtime。
- 当前数据根固定为 `profile/` 和 `message/profiles/`，Profile schema 与运行时版本必须严格校验。
- stdout 继续为 NDJSON；验证记录不得包含 Cookie、token、`sec_uid`、完整会话 ID、成员列表或消息正文。
- 历史 Comet archive 保持原样；legacy 清理只作用于活动源码、当前文档、当前规格和当前测试。
- 真实写入不属于本次回归，既有写入自动化门禁仍必须通过。

# Decisions

- 使用两个完整目标规格：`doudou-skill-orchestration` 和 `legacy-free-runtime`。
- `scripts/doudou.py` 当前仍由 npm launcher 内部调用，不按文件名直接判定为 legacy；用户和 Agent 不直接执行它。
- legacy 自动迁移能力连同专用测试和文档一起删除，不保留隐藏开关或 deprecated alias。
- 旧顶层 `Profile/`、被明确选中的旧 Chromium Profile、被当前命令读取的旧 identity 数据或旧用户缓存统一返回 `LEGACY_UNSUPPORTED` 并停止；不迁移、不忽略、不创建同名新目录。
- 有效 v2 Profile 与历史 Chromium backup 目录可以共存；backup 目录保持原样且不参与当前 Profile 枚举，只在被明确选中时阻断。

# Open questions

- 已确认：旧数据和旧运行时直接不兼容，发现即返回 `LEGACY_UNSUPPORTED` 并停止。
- 已确认共享理解：完成 Skill spec 补齐，删除四类 legacy 迁移/兼容实现与当前文档/测试，保留 npm 内部 Python bootstrap，历史 Comet archive 不变；随后执行自动化门禁和已登录 Profile 的真实只读回归。
- 已确认：当前数据根同时存在有效 v2 Profile 与旧 Chromium backup 目录时，只在命令选中旧目录时返回 `LEGACY_UNSUPPORTED`。
- 已确认更新后的共享理解：legacy 兼容实现全部删除；旧顶层根、旧缓存和被明确访问的旧 Profile/identity 直接拒绝；历史 backup 可原样共存但不参与当前枚举；随后继续全部自动化与真实只读回归。

# Verification expectations

- 运行 `npm test`、manifest build/check、pack/install、release version check、全部 Python pytest、Ruff、Skill quick validation 和 `git diff --check`。
- 验证 `doudou --version`、`doudou doctor`、`doudou profile list` 的当前入口和 NDJSON 契约。
- 对已登录空闲 Profile 执行 `conversations refresh --kind all --force`，证明 `source=live`；分别对群聊和私聊执行固定时间窗消息采集，并重复至少一轮验证增量去重。
- 报告每项命令、退出状态、脱敏计数、HTTP/Protobuf 状态、added/duplicates/conflicts 和 NetworkError 计数；失败或未执行不得记为通过。
