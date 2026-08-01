# Outcome

把当前项目、交付 Skill、Node.js CLI、内部产品标识和 GitHub 远端仓库从 `douyin-message` / `douyin-group-message` 完整重命名为 `doudou`。最终用户只看到 `doudou` 品牌；抖音域名、接口、协议和业务术语继续按真实目标保留。

# Scope

- 项目根目录保持 `D:/Code/doudou`，交付目录使用 `doudou/`。
- Skill frontmatter 名称、标题、Agent 元数据、安装路径和调用示例改为 `doudou`。
- npm package、bin、入口文件、脚本名、帮助文本、修复命令和环境变量改为 `doudou` / `DOUDOU_HOME`。
- 运行时 schema、锁 owner、迁移临时目录、测试 fixture、函数/常量命名和产品注释改用新品牌。
- README、根 `.gitignore`、当前 canonical Comet specs 和交付文档改用新路径与名称。
- 本地 `.agents/skills/doudou` 安装副本与交付目录同步；旧的项目内 Skill 安装目录在验证后移除。
- GitHub private repository 从 `Follen/douyin-message` 重命名为 `Follen/doudou`，更新 origin 并验证 fetch/push URL。

# Non-goals

- 不修改 `douyin.com`、`imapi.douyin.com` 等平台域名、API endpoint 或网络协议字段。
- 不重写历史 Comet archive、历史诊断和既有发送回执；它们是不可变研究/验证记录。
- 不重命名用户自定义 Profile ID、会话名、消息内容或媒体文件。
- 不改变消息拉取、发送、回复、@、图片、登录和指纹行为。

# Acceptance examples

- 在项目根运行 `node .agents/skills/doudou/bin/doudou.js help`，帮助和修复命令只使用 `doudou`。
- `npm pack --dry-run` 的包名和 bin 均为 `doudou`，包内不存在 `douyin-message` 命名的入口文件或安装路径。
- 新创建的 Profile、会话索引、消息、Run、回执和锁记录使用 `doudou.*` schema 或 `doudou` owner 标识。
- 活跃源码、测试、README、Skill 和 canonical specs 中，除真实平台名/域名及明确迁移映射外，不存在旧项目品牌。
- GitHub 仓库地址为 `https://github.com/Follen/doudou`，本地 `origin` 与其一致，仓库仍为 private。

# Constraints and invariants

- 保留当前 `profile/` 与 `message/` 文件，不把运行数据纳入 Git 或发布包；新版不迁移、不兼容其中的旧产品 schema/owner，用户需要用新版重新生成可用元数据。
- 目录重命名必须被 Git 识别为同一项目内容的 rename，不丢失实现文件。
- GitHub 重命名前必须确认目标仓库名可用；当前 `gh` 登录失效时先完成本地工作，远端步骤保持明确阻塞，不伪造成功。
- 所有自动化测试、Skill 校验、打包检查和安装副本哈希比较必须通过。

# Decisions

- 用户指定最终名称为 lowercase `doudou`，并要求项目、Skill、远端仓库、函数和注释统一使用新名称。
- 实际抖音平台名、URL 与协议字段不是项目品牌，不做替换。
- 历史 archive、diagnostics、发送回执和用户内容保持不可变。
- 用户选择彻底切换：只提供 `$doudou`、`doudou` CLI、`bin/doudou.js` 和 `DOUDOU_HOME`，不保留 `douyin-message` Skill、CLI、bin 或环境变量兼容入口。
- 2026-08-01 已验证 GitHub CLI 登录有效，原仓库 `Follen/douyin-message` 为 private，目标仓库名 `Follen/doudou` 尚不存在。
- 用户选择不迁移既有本地运行记录：新版只接受 `doudou.*` schema 与 `doudou` owner 标识，旧 `profile/`、`message/` 文件保持原样但不作为兼容输入。
- 用户选择保持历史证据原样：`docs/comet/archive/`、`analyze/`、旧发送回执和诊断中的旧项目名称不重写；旧品牌残留检查明确排除这些不可变历史目录。
- 用户于 2026-08-01 确认本 brief 与三份目标规格，批准进入 Build 并执行本地及 GitHub 重命名。

# Open questions

无。

# Verification expectations

- 全量运行 Node test suite、Skill quick validation、`npm pack --dry-run --json` 和 CLI help/doctor fixture。
- 对活跃交付源码、测试、README、Skill 和 canonical specs 做旧品牌残留扫描，并人工复核每个允许保留项。
- 比较 `doudou/` 与 `.agents/skills/doudou/` 的发布文件 SHA-256。
- 使用 GitHub API/CLI 验证仓库新名称、private 状态、默认分支和 origin URL。
