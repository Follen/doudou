# Broker authentication state

## Purpose

让常驻 broker 用账号会话、验证页面和 IM 捕获三类独立状态驱动登录与人工接管，避免已登录页面因 Cookie 名称或监听时序被误报。

## Requirements

### 登录有效性必须使用结构化页面或会话证据

登录分类不得依赖固定 Cookie 名称白名单。有效 IM 会话响应或 `/chat` 已加载会话列表且无登录/验证组件，均可证明账号已登录；Cookie 只作为诊断计数，不作为唯一判据。

#### Scenario: 未知认证 Cookie 但会话页面已加载

- Given `/chat` 左侧已加载多个会话项
- And 页面没有二维码、短信验证码、滑块或验证对话框
- And Cookie 名称不在旧白名单
- When broker 分类登录状态
- Then Profile 标记为 `valid`
- And 不返回 `LOGIN_REQUIRED`

### IM 捕获缺失必须独立分类

账号已登录但未捕获 init request 时返回 `IM_CAPTURE_MISSING`；已捕获请求但没有可用 Protobuf 响应时返回 `IM_INITIALIZATION_MISSING`。两者不得改写 Profile 为未登录或需要验证。

#### Scenario: 已登录但监听错过 IM 初始化

- Given 页面账号状态有效且没有验证组件
- And 本次 broker 没有 init request/response
- When 读操作要求 IM envelope
- Then 返回 `IM_CAPTURE_MISSING`
- And Profile 仍记录 `login_status=valid`

### 登录成功事件必须异步收敛状态

浏览器网络响应和页面状态变化必须触发异步状态重算；有效账号证据出现后立即记录 Profile 并结束接管，不等待固定 Cookie 名称的秒级轮询。

事件状态机必须使用单调 generation 或等价机制避免 `classify` 后清 Event 的丢唤醒；重算任务执行期间到达的新事件必须标记 pending，并在退出前至少处理最新 generation。主 frame 导航后必须按当前 takeover 状态恢复页面观察器。MutationObserver 只负责唤醒，最终分类仍通过版本化组件契约和结构化会话证据完成。

#### Scenario: 验证完成后业务读取失败

- Given 人工接管期间完成短信或扫码登录
- And 随后的业务读取返回 IM 捕获或上游错误
- When 页面已经没有登录或验证组件
- Then broker 仍结束接管并隐藏同一窗口
- And 业务错误保持原分类

#### Scenario: 分类期间连续事件

- Given broker 正在分类登录状态
- And 网络响应与验证码 DOM 变化连续到达
- When 第一轮分类完成
- Then 最新 event generation 仍被处理
- And 不因 Event 清理或已有 reconcile task 丢失最终登录成功状态

#### Scenario: 人工接管期间页面导航

- Given takeover active 且页面观察器已启用
- When 主 frame 导航创建新的 document
- Then 新 document 重新启用轻量状态观察器
- And 验证组件消失后仍异步结束接管

### 接管收尾必须覆盖成功、超时和异常

`login.wait`、读恢复和 broker 错误路径必须统一执行接管收尾。只有页面仍显示二维码、验证码、滑块或验证对话框时窗口才保持可见。

#### Scenario: 验证仍然可见

- Given 页面仍显示短信验证码或滑块
- When 当前等待超时或读操作失败
- Then 接管保持 active 且窗口保持可见
- And 返回对应登录或验证错误

#### Scenario: 验证已消失但 IM 初始化失败

- Given 页面已登录且验证组件消失
- And IM 初始化随后失败
- When broker 完成错误处理
- Then 接管 inactive 且窗口、任务栏和 Alt+Tab 恢复后台隐藏
- And Profile 登录状态保持 valid
