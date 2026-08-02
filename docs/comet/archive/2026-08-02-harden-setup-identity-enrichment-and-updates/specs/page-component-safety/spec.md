# Page component safety

## Purpose

让所有 DOM 动作与网页状态判断通过同一套可诊断、可版本化、失败关闭的三级组件契约，避免同名文本、隐藏元素或错误容器造成误判和误操作。

## Requirements

### 所有页面判断必须通过三级漏斗

每个 Component Adapter 必须先用 role、aria-label、data-e2e、稳定文案或版本化结构 selector 形成语义候选；再锁定所属局部组件、容器、弹层、菜单、列表或目标项；最后校验唯一可见、动作控件 enabled、预期状态以及包含/相邻/方向/距离/区域等几何关系。无法唯一闭合时返回 `PAGE_ADAPTER_DRIFTED`，不得使用全页模糊命中、任意 first/nth、固定坐标、React/Fiber 或猜测点击。

#### Scenario: 同名文本位于错误容器

- Given 页面有一个隐藏同名文本和一个目标组件内的可见候选
- When Adapter 解析动作目标
- Then 只接受所属局部组件内通过状态和几何校验的候选
- And 错误容器、多候选或 disabled 均 fail closed

### 认证和聊天组件必须局部闭合

AuthChallengeComponent 必须覆盖二维码、短信、快捷登录短信、滑块、通用验证框和获取/发送验证码按钮。ConversationListComponent 必须先锁定左侧会话列表，再按协议目标选择会话。引用/点赞只在目标消息触发的上下文菜单内解析；图片发送只在唯一预览弹窗内解析发送控件。发送群聊 @ 不属于产品能力，不得打开成员弹层或执行写操作。

#### Scenario: 旧客户端传入群聊 @ 参数

- Given 调用方仍向文字发送命令传入旧 `--at` 参数
- When CLI 解析命令
- Then 返回稳定错误码 `MENTION_UNSUPPORTED`
- And 不创建或连接 broker、不启动浏览器、不创建 plan
- And Skill 与公开 CLI 文档不再展示该参数

#### Scenario: 从视频页发送聊天消息

- Given broker 当前位于视频页
- When 执行聊天写 plan
- Then 先打开 `/chat` 并确认左侧会话列表完成渲染
- And 只在列表局部选择协议解析的目标会话

### 视频读写触发器必须使用组件适配器

视频操作必须先锁定当前视频操作栏和评论入口；教程提示只在唯一弹窗内关闭；展开回复必须绑定具体评论项及内部展开控件；评论点赞必须绑定具体评论项、局部 stats/like 控件、图标/语义和当前点赞状态。`reads/video.py` 不得直接点击旧入口。

#### Scenario: 多个评论都有展开回复

- Given 页面同时存在多个“展开回复”候选
- When 协议分页需要展开某个根评论
- Then Adapter 只操作该 comment ID 对应评论项内部的控件
- And 不选择全页第一个候选

### Page Doctor 必须报告操作级契约

Page Doctor 必须逐组件报告三层候选、局部容器关系、唯一性、可见/启用/预期状态、几何结论、定位策略和漂移影响。必须覆盖 auth、conversation list、send、message context menu、image preview、video action bar、评论 composer、回复和点赞；未进入所需页面或未建立前置状态时报告 `not_exercised`。

#### Scenario: 单一组件漂移

- Given 图片预览发送控件漂移
- When Page Doctor 检查当前页面
- Then 报告图片发送能力受影响及失败层级
- And 不阻止不依赖该组件的协议读取或其他写能力

### 全量 DOM 审计必须可回归

源码中所有 click、fill、press、keyboard、file chooser、mouse、scroll 及基于 DOM 的业务判断必须由 Component Adapter 封装并接受同一三级契约。测试必须覆盖同名冲突、多候选、隐藏、disabled、错误容器、错误几何和页面结构漂移。

#### Scenario: 读操作需要滚动触发协议分页

- Given 读协议下一页必须由页面滚动触发
- When 采集器执行滚动
- Then 先确认版本化滚动组件、可见状态和所属页面区域
- And 不能直接对任意页面元素或固定坐标滚动
