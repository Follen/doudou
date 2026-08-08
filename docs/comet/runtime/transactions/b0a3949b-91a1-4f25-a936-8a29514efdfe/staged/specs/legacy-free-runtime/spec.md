# legacy-free-runtime

## 单一当前运行时契约

Doudou 只支持 npm 管理的 Skill、`${DOUDOU_HOME:-~/.doudou}` 下的专用 Python/Camoufox runtime、项目 `profile/` 与 `message/profiles/` 数据根，以及当前 Profile schema。正式源码不得保留旧产品入口、旧目录、旧浏览器 Profile 或历史 identity 记录的自动迁移、删除或补写逻辑；可以保留返回 `LEGACY_UNSUPPORTED` 所需的最小只读格式识别。

内部 `scripts/doudou.py` 只作为 npm launcher 启动当前 Python 包的实现细节，不是独立安装方式、公开兼容入口或文档命令。删除兼容层时不得破坏 npm 对当前托管 runtime 的调用。

## 删除的兼容能力

活动源码、CLI help、当前 references、README 和当前测试必须移除以下能力及专用实现：

- Windows `%LOCALAPPDATA%/doudou` 与 Linux `${XDG_CACHE_HOME:-~/.cache}/doudou` 旧缓存根的迁移、删除和迁移回执；只读识别仅用于拒绝继续。
- 项目顶层 `Profile/` 自动改名为 `profile/`，以及新旧目录冲突兼容分支。
- Chromium Profile schema 的自动备份、重建、迁移回执和相关恢复逻辑。
- `identity migrate` CLI、identity migration 模块和针对历史记录的批量补写。
- 仅为上述能力存在的文档、manifest 项、测试 fixture、错误文案和导出符号。

不得保留 deprecated alias、隐藏环境变量、兼容开关或“运行一次后删除”的迁移代码。普通当前 schema 名称中使用 `.v1` 不等同于 legacy，除非存在替代的当前 schema 契约；不得按字符串批量删除有效 schema。

## 历史证据边界

`docs/comet/archive/` 和既有只读验证记录保持创建时内容，不参与 legacy 清理。项目本地 Profile、消息和抓包数据不进入 Git 或 npm 包。清理后的 manifest 与 package inventory 必须只包含当前正式 Skill 交付树。

## 旧数据遇到时的结果

旧顶层 `Profile/`、被命令明确选择的旧 Chromium Profile、被当前命令读取的旧 identity 数据或带 Doudou 标记的旧用户缓存必须返回稳定错误 `LEGACY_UNSUPPORTED`，包含脱敏路径槽位、旧格式类别和当前支持路径，并在任何浏览器、broker、写入或数据改造动作前停止。不得迁移、删除、覆盖或创建同名替代目录。

有效 v2 Profile 与历史 Chromium backup 目录可以共存。backup 目录必须保持原样，不参与 `profile list`、默认 Profile 选择或当前 Profile schema 的全局门禁；只有命令明确选中该旧目录时才返回 `LEGACY_UNSUPPORTED`。这项共存不读取、转换或复用旧 Profile 内容，不构成兼容入口。

### 场景：有效 Profile 与 backup 共存

- Given `profile/` 同时包含当前 v2 Profile 和历史 Chromium backup 目录
- When 用户列出 Profile 或选择当前 v2 Profile 执行业务命令
- Then backup 目录不出现在结果中且不阻断当前命令
- And backup 内容保持原样

### 场景：明确选择旧 Profile

- Given 用户命令明确使用历史 Chromium Profile 的目录名
- When CLI 加载该 Profile
- Then 返回 `LEGACY_UNSUPPORTED`
- And 不启动浏览器、不迁移、不删除也不新建同名 Profile

### 场景：发现旧数据

- Given 当前项目或用户缓存出现旧格式数据
- When doctor、setup 或业务命令检查环境
- Then 返回 `LEGACY_UNSUPPORTED` 并保持旧数据不变
- And 不启动业务浏览器、不启动第二个 broker、不执行 identity migrate 或其他改造
