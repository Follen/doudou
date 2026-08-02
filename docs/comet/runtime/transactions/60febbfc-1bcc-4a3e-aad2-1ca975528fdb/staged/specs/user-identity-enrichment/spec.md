# User identity enrichment

## Purpose

为所有业务记录提供一致、可追踪、可增量补齐的用户身份结构。

## Requirements

### 所有身份位置必须使用统一结构

身份结构必须包含 `uid`、`sec_uid`、`unique_id`、`nickname`、`remark_name`、`display_name` 及来源/补全状态。

#### Scenario: 普通群聊发送者

- Given IM 消息只有 sender uid 与 sec_uid
- And user-info 响应包含抖音号和昵称
- When 消息采集完成
- Then sender 保存完整统一身份结构
- And unique_id 不会在名称映射时被丢弃

#### Scenario: API 缺少抖音号

- Given 可用响应只有 nickname
- When 身份记录落盘
- Then unique_id 保持 null 并记录缺失状态
- And nickname 不会被写入 unique_id

### 身份补齐必须覆盖全部业务对象

补齐管线必须覆盖群聊和私聊消息、会话参与者、群成员、进群 source/joined users、引用/@用户、视频作者、评论与回复用户。

#### Scenario: 进群系统事件

- Given member_joined 事件只携带部分 joined user 字段
- When 同一采集任务可通过结构化用户接口补全
- Then source 和 joined users 使用统一身份结构并合并补全结果

#### Scenario: 真实 Profile 全量只读回归

- Given 现有 Profile 可读取群聊、私聊、成员、视频和评论协议
- When 执行身份回归但不执行任何写操作
- Then sender、参与者、成员、member_joined、引用/@、作者、评论与回复用户均落盘统一字段和来源状态
- And 缺失 unique_id 保持 null，重复迁移不降级或重复

### Profile 级身份缓存必须合并而不降级

同一 Profile 内的身份缓存必须按稳定标识关联候选，并按字段合并；空值和低信息记录不得覆盖已有完整字段。

#### Scenario: 新旧响应顺序相反

- Given 缓存已有完整 unique_id 与 nickname
- When 后续响应只含 uid/sec_uid 和空名称
- Then 缓存和业务记录保留原完整值

### 既有记录必须可重复迁移

系统必须提供幂等迁移/merge，使旧 conversations、messages、member_joined、视频和评论记录能够引用缓存或重新补齐。

#### Scenario: 旧消息补齐

- Given 已落盘消息 sender 只有 uid/sec_uid/name
- When 执行身份迁移并存在匹配缓存
- Then sender 升级为统一身份结构
- And 重复执行迁移不产生重复记录或字段回退

### 身份补全不得依赖深层前端对象遍历

补全只能使用协议响应、已知结构化存储和有界页面接口，不得遍历 React/Fiber 或任意对象图。

#### Scenario: 用户接口暂时失败

- Given 业务主响应可用但补全接口失败
- When 读取任务落盘
- Then 主业务记录仍保存部分身份和失败状态
- And 不切换到消息 DOM 深遍历
