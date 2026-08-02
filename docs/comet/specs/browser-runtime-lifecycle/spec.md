# Browser runtime lifecycle

## Purpose

让 Doudou 在 Windows 日常运行时使用真正无头 Camoufox，彻底删除透明有头窗口方案；只有需要人工登录或验证时才串行启动同一 Profile 的可见有头浏览器。Linux 继续使用 Xvfb 虚拟有头。

## Requirements

### Windows 日常 broker 必须使用真正无头 Camoufox

Windows broker 在账号有效、页面没有人工认证组件时必须以 Camoufox 真无头模式启动和保持业务 context。不得创建透明、离屏、ToolWindow、从任务栏删除或 alpha=0 的有头主窗口，也不得把 Win32 窗口样式控制作为生产 fallback。broker descriptor、doctor 和诊断必须明确报告 `headless`。

#### Scenario: Windows 正常业务启动

- Given Profile 登录有效且没有认证组件
- When broker 启动并进入 `/chat`
- Then Camoufox 以真无头模式运行
- And Windows 不存在对应可见或透明的浏览器主窗口
- And 协议读取与页面写入复用该 context

### Linux 必须保留 Xvfb 虚拟有头

Linux 正常业务 broker 必须继续使用 Camoufox 虚拟有头与 Xvfb。doctor 必须分别检查 Xvfb 能力与人工认证显示能力；该平台不得因 Windows 运行模式迁移而被强制改成原生 headless。

#### Scenario: Linux 日常启动

- Given Linux 安装已通过 doctor
- When broker 启动正常业务会话
- Then 使用 Xvfb 虚拟有头
- And broker descriptor 报告 Linux 虚拟显示模式

### 人工认证必须串行切换到同 Profile 可见有头

账户未登录或页面三级认证组件确认存在二维码、短信、快捷登录短信、滑块、Passkey 或通用验证时，Windows broker 必须先安全关闭无头 context，再用相同 `browser-data`、Camoufox 版本和持久指纹配置启动可见有头 context。两个 context 不得重叠占用 Profile，不得复制 Cookie 或创建临时 Profile。

Linux 必须从虚拟显示切换到用户可见显示，同时维持同样的 Profile 互斥约束。

#### Scenario: 无头过程中出现短信验证

- Given Windows broker 正在真无头运行
- And AuthChallengeComponent 唯一确认短信输入组件可见
- When broker 进入 takeover
- Then 无头 context 先完整关闭
- And 同一 Profile 的可见有头 Camoufox 启动并显示短信页面
- And descriptor 报告认证类型、模式和 takeover 状态

#### Scenario: KICK 但没有认证组件

- Given 写请求返回 `decision=KICK`
- And 页面没有真实认证组件
- When broker 分类结果
- Then 返回 `WRITE_VERIFICATION_REQUIRED`
- And 不启动可见有头浏览器
- And 不通过额外消息、评论或点击制造验证界面

### 认证完成后必须异步回到后台模式

认证页面网络事件、主 frame 导航和轻量 MutationObserver 只负责提升 generation；状态机必须处理到最新 generation，并用结构化账号证据与 AuthChallengeComponent 重算。验证消失且登录有效后，可见有头 context 必须关闭；Windows 重建真无头 context，Linux 回到 Xvfb。原只读任务最多恢复一次；任何已开始写任务不得自动重放。

#### Scenario: 用户完成 Passkey

- Given 可见有头 takeover 正在等待 Passkey
- When 页面导航并出现有效账号证据且认证组件消失
- Then 异步状态机完成最新 generation 的重算
- And 可见 context 自动关闭
- And 平台默认后台 context 使用同一 Profile 重建
- And Profile 标记为 valid

#### Scenario: 业务恢复失败

- Given 用户已经完成认证且可见 context 已关闭
- And 重建后台 context 或后续 IM 初始化失败
- When 命令结束
- Then 返回真实的 IM 或业务错误
- And 不把 Profile 降级为未登录
- And 不把可见窗口遗留给用户

### 未完成认证必须保留可见接管

等待超时或业务错误发生时，如果认证组件仍真实可见，可见 context 必须保持供用户继续处理并报告 takeover active。用户手动关闭窗口时必须记录明确状态并释放 context；下一次业务命令可以再次进入认证接管，但不得宣称登录成功。

#### Scenario: 滑块仍未完成

- Given takeover 中滑块组件仍可见
- When 登录等待达到命令超时
- Then 返回 `VERIFICATION_REQUIRED` 与 `verification_kind=slider`
- And 可见浏览器保持供用户接管
- And Profile 不标记为 valid

### Broker 生命周期和写入语义必须跨模式保持一致

同一 data root/Profile 仍只能有一个 broker 和一个 Profile lock owner；命令严格串行，空闲 20 分钟后退出。读取继续通过页面上下文协议；写入继续通过受控页面组件并监听唯一网络请求。聊天文本/引用和视频评论/回复在编辑器聚焦、正文与状态校验后必须按一次 `Enter`，图片和点赞继续使用其组件动作。模式切换不得绕过 plan/confirm、不得重放未知写结果。

#### Scenario: Windows 真无头发送文字

- Given Windows headless broker 已选择唯一会话
- And 用户确认的文字发送 plan 未漂移
- When ChatComposerComponent 填写正文并提交
- Then 编辑器聚焦和正文匹配后只按一次 `Enter`
- And 只观察到一个匹配会话与正文的发送请求
- And 网络回执与只读复查写入 receipt

### 评论成功回执不得被慢速复查覆盖

评论创建或回复的页面网络响应在 HTTP、业务状态、视频 ID、正文和稳定评论 ID 全部匹配后必须立即作为权威成功结果保存 receipt 并完成 plan。协议读回只允许执行有界补充检查，不得同步全量分页评论。补充检查超时、取消、空响应、格式错误或排名列表暂未包含新评论时，必须记录非权威复查状态，不得把成功降级为 unknown。

#### Scenario: 评论已发布但复查超时

- Given 页面发布响应返回业务成功和稳定评论 ID
- And 后续协议复查超过有界时限
- When 写命令结束
- Then plan 与 receipt 状态仍为 `created`
- And verification 记录 `protocol_recheck` 非权威超时
- And 命令不重发评论

### 分发文档和诊断必须使用同一模式契约

doctor、setup launch probe、broker descriptor、Page Doctor、Skill、README、CLI reference 和 npm manifest 必须统一说明 Windows 真无头、认证时可见有头、Linux Xvfb。Windows 平台代码不得再报告或依赖 `windows_hidden_headed`、透明度、任务栏或 Alt+Tab 指标。

#### Scenario: Windows doctor

- Given Windows Doudou runtime 已安装
- When 用户运行 doctor launch probe
- Then 结果验证真无头 Camoufox 可启动
- And 不创建透明有头窗口
- And repair 建议只在认证需要时引导可见有头接管
