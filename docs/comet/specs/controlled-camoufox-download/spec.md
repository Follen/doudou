# Controlled Camoufox download

## Purpose

用 Doudou 可观察、可校验、可并发协调的下载边界封装 Camoufox 0.5.4，避免官方 CLI 的缓冲、锁和退出码缺口变成半安装或孤儿进程。

## Requirements

### 浏览器必须是 Doudou 专用资产

Camoufox 浏览器只允许安装到 `${DOUDOU_HOME:-~/.doudou}/browsers/`，不得读取或复用系统 Camoufox、其他 Agent 或其他工具的缓存。浏览器版本、资产大小、SHA-256 与 launch probe 均绑定当前 runtime contract。

#### Scenario: 系统共享缓存已有相同版本

- Given 系统 Camoufox 缓存已有相同浏览器版本
- And Doudou 专用 browsers 目录为空
- When Doudou setup 运行
- Then Doudou 下载并验证自己的浏览器资产
- And 不把系统缓存视为命中

### 下载必须实时流式报告

浏览器下载的 stdout/stderr、阶段心跳、已下载字节和总字节必须实时产生结构化进度，不得等子进程结束后一次性返回。

#### Scenario: 最终缓存目录暂时不增长

- Given Camoufox 正把 zip 写入系统临时文件
- When 最终浏览器缓存大小暂时不变
- Then Doudou 仍根据下载流量和心跳报告活动
- And 不把缓存目录不增长单独判定为卡死

### 下载必须具有停滞和总超时

连接超时固定为 15 秒、连续 90 秒没有新字节判定读停滞、浏览器下载阶段总时限为 45 分钟。可重试错误最多尝试 3 次并按 1/3/10 秒有限退避；任一终止都要回收完整进程树。结构化状态每秒更新，终端至少每 5 秒输出一次心跳。

#### Scenario: 连接建立但不再传输

- Given HTTP 连接未关闭但下载字节长期不增长
- When 达到停滞超时
- Then 返回 `BROWSER_DOWNLOAD_STALLED`
- And 下载进程树与 partial 状态被清理或隔离

### 下载必须支持安全断点续传

下载正文必须写入隔离 `.part` 并保存不含签名 URL 的资产身份、预期大小、ETag/Last-Modified 与已下载长度。重试时优先使用 Range；服务端忽略 Range、身份变化或返回不一致时必须清零安全重下。

#### Scenario: Range 续传成功

- Given `.part` 与官方资产身份和长度一致
- And 服务端返回匹配的 206 Content-Range
- When setup 重试下载
- Then 从已有长度继续并在完成后校验整个文件 SHA-256

#### Scenario: 服务端不支持 Range

- Given 已存在 `.part`
- And 服务端对 Range 返回完整 200
- When setup 重试下载
- Then 丢弃旧 partial 内容并从零写入
- And 不把新响应追加到旧文件

### 下载必须遵循并诊断代理与 TLS

直连、系统代理和显式 `HTTP_PROXY`/`HTTPS_PROXY` 必须按明确优先级传递；代理不可达、TLS/证书异常和 HTTP 状态必须分类，不记录签名 URL。

#### Scenario: 显式代理不可达

- Given `HTTPS_PROXY` 指向不可连接端点
- When setup 下载浏览器
- Then 返回稳定的代理/连接错误分类
- And 不回退为无限等待或伪成功

### 下载错误必须稳定分类

DNS、代理连接、TLS/证书、HTTP 403、HTTP 404、连接超时、读停滞、总超时、磁盘空间不足/写入失败和 SHA-256 不匹配必须返回不同稳定错误码。

#### Scenario: 资产哈希不匹配

- Given 下载字节数达到官方 Content-Length
- But SHA-256 与 runtime manifest 不同
- When 下载阶段结束
- Then 返回 `BROWSER_DOWNLOAD_CHECKSUM_MISMATCH`
- And 不解压或激活该资产

### partial 缓存不得冒充成功

下载、解压和元数据写入必须先在隔离临时位置完成。只有 zip SHA-256、官方资产元数据、Camoufox browser version 和 launch probe 全部通过后才可激活。

#### Scenario: 下载中断后重试

- Given 前一次下载只留下 partial 文件或不完整版本目录
- When 用户再次 setup
- Then partial 状态不会被识别为已安装
- And setup 安全续传或重新下载后完成验证

#### Scenario: 已有有效共享缓存

- Given 共享 Camoufox 缓存中的精确版本、资产 SHA 元数据和可执行文件均通过验证
- When 新 runtime 执行 setup
- Then 复用该缓存且仍执行真实 launch probe
- And 不重复下载约 492 MB 的资产

### 两个 setup 必须共享同一下载 owner

同一用户的并发 setup 只能有一个 owner 执行 Camoufox 下载；等待者必须读取同一结构化阶段和进度。

#### Scenario: 并发首次下载

- Given 两个 setup 同时发现浏览器缺失
- When 二者进入 browser download 阶段
- Then 只有 owner 创建下载进程
- And 等待者显示 owner 进度并复用最终结果

### 专项实验必须覆盖故障矩阵

验证必须在隔离 HOME/cache 下覆盖直连、继承系统代理、显式代理、代理不可达、TLS 异常、下载中断、慢速/停滞、磁盘不足、只读目录、杀死外层 setup、杀死 fetch 子进程和并发 setup。

#### Scenario: 外层 setup 被杀死

- Given setup 正在执行官方 fetch 或受控下载
- When 外层进程被终止
- Then 子进程树被回收
- And 下一次 clean retry 不复用半成品为成功

### doctor 必须报告下载和缓存状态

doctor 必须报告 setup owner、阶段、最后进度时间、partial 长度、预期总大小、重试状态、缓存版本/哈希/launch 结果与稳定修复命令；输出不得包含代理凭据、Cookie 或 URL query。

#### Scenario: partial 长时间无增长

- Given setup 状态存在且 `.part` 长度超过零
- And 最后新字节时间超过停滞阈值
- When 用户执行 doctor
- Then doctor 报告下载停滞而不是登录或依赖错误
- And 给出终止陈旧 owner 后安全续传的修复建议
