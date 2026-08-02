# CLI version and update

## Purpose

定义无需运行 Python 的版本查询，以及可检查、升级并保护用户数据的 npm 更新入口。

## Requirements

### 三个版本参数必须等价

`doudou --version`、`doudou -version` 和 `doudou -v` 必须输出相同纯版本号并以成功状态退出。

#### Scenario: runtime 尚未安装

- Given npm 启动器已安装但 Python runtime 和浏览器不存在
- When 用户执行任一版本参数
- Then CLI 只输出 npm 包版本
- And 不启动 Python、setup 或浏览器

### 更新检查不得修改安装

CLI 必须提供只读更新检查，报告当前版本、npm latest 与是否有更新。

#### Scenario: 已是最新版本

- Given 当前版本等于 npm latest
- When 用户检查更新
- Then 输出已是最新且不修改 npm 包、Skill 或 runtime

### 更新不得修改项目数据

任何检查、成功升级或失败回滚都不得扫描或修改项目 `profile/`、`message/`。

#### Scenario: 更新失败

- Given 项目中存在 Profile 和消息数据
- When npm 包或 Skill 升级失败
- Then 项目数据内容与哈希不变
- And 原可用安装仍可诊断或重试

### 全局安装必须支持原地更新

`doudou update` 必须查询 npm latest；存在新版本时升级当前全局 npm 包并原子升级受管 Skill，已是最新时成功返回且不重复安装。`doudou update --check` 只能查询和报告，不得修改安装。

#### Scenario: 全局安装存在新版本

- Given Doudou 由 npm 全局安装且 npm latest 更新
- When 用户执行 `doudou update`
- Then 全局 npm 包和受管 Skill 升级到 latest
- And 项目 Profile 与消息数据保持不变

#### Scenario: 只读检查发现更新

- Given npm latest 高于当前版本
- When 用户执行 `doudou update --check`
- Then 输出当前版本、latest 和有更新状态
- And npm 包、Skill 与 runtime 均不改变

### npx 更新不得创建全局安装

通过 npx 临时运行时，`doudou update` 和 `doudou update --check` 只能报告临时运行方式、当前版本和 npm latest，不得安装或升级全局包。

#### Scenario: npx 调用 update

- Given Doudou 正通过 npx 临时运行
- When 用户执行 `doudou update`
- Then CLI 不创建或修改全局 Doudou 安装
- And 输出说明 npx 应显式请求目标版本或 latest

### runtime 只在契约变化时重建

更新后的 runtime manifest 与当前 runtime ID 一致时必须复用现有 venv/浏览器；契约变化时才通过 resilient setup 构建并原子切换。

#### Scenario: 仅 Skill 代码更新

- Given 新版本的 runtime manifest 与当前一致
- When 更新成功
- Then 受管 Skill 原子升级
- And 不重新下载依赖或浏览器
