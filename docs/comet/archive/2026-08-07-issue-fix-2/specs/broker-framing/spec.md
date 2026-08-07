# Broker framing

## Purpose

保证 broker 的单行 JSON 请求/响应协议在会话和采集结果超过 asyncio 默认 64 KiB 行限制时仍可预测、可诊断且不会无限占用内存。

## Requirements

### 有界大响应

客户端必须为 broker 响应配置显式最大帧，并成功读取明显超过 64 KiB、但未超过该上限的完整换行分隔 JSON。

#### Scenario: 大型会话列表

- Given broker 返回一条至少 128 KiB 的有效 JSON 响应
- When 客户端等待该响应
- Then 客户端完整解码响应
- And 不暴露 `LimitOverrunError`、`ValueError` 或截断 JSON

### 超限分类

客户端必须在响应超过最大帧时停止读取并返回稳定的 Doudou 错误；不得采用无限制缓冲。

#### Scenario: 读响应超限

- Given 读命令的 broker 响应超过最大帧
- When 客户端读取响应
- Then 返回稳定的响应过大错误码
- And 错误详情不包含完整响应内容

#### Scenario: 写响应超限

- Given 写命令可能已经执行且其 broker 响应超过最大帧
- When 客户端读取响应
- Then 返回写结果未知
- And 不自动重放该写命令

### 连接收尾

成功、超时、超限、无完整换行帧和 JSON 解码失败路径都必须关闭 writer，并保留既有读写重试边界。

