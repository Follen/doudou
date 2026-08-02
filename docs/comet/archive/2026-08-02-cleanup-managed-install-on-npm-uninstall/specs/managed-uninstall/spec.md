# Managed uninstall

## Purpose

定义 Doudou npm 包的完整卸载契约，使 npm 管理的可执行入口、Skill 和共享 Python 运行时能够一起清理，同时保护项目业务数据和未知目录。

## Requirements

### 单命令清理托管安装

当用户执行 `doudou uninstall` 时，系统必须删除 npm 包自身、有效 Doudou 管理标记所标识的 Skill，以及 Doudou 用户级共享 `runtimes/`。

#### Scenario: 默认安装完整卸载

- Given npm 包安装了默认 Skill 并创建了共享 venv/runtime
- When 用户执行 `doudou uninstall`
- Then npm 包、默认 Skill 和共享 `runtimes/` 均被删除

#### Scenario: 清理对象已经缺失

- Given Skill 或共享 `runtimes/` 已经不存在
- When 用户执行 `doudou uninstall`
- Then 卸载成功且缺失对象被视为已经清理

### 项目数据必须保留

卸载流程不得扫描、修改或删除调用项目中的 `profile/`、`message/` 或其他业务文件。

#### Scenario: 项目数据与安装数据并存

- Given 隔离项目中存在带内容的 `profile/` 和 `message/`
- And 用户级位置存在 npm 管理的 Skill 与共享 venv/runtime
- When 用户执行 `doudou uninstall`
- Then `profile/` 和 `message/` 的文件清单、内容和哈希保持不变

### 未知 Skill 目录必须保留

卸载流程只能删除带有有效 Doudou npm 管理标记且位于已记录目标路径的 Skill。目标不存在、标记损坏、包名不匹配或没有标记时不得删除该目录。

#### Scenario: 目标目录不属于 Doudou npm 包

- Given Skill 路径存在但没有有效 Doudou 管理标记
- When 用户执行 `doudou uninstall`
- Then 该目录保持不变
- And 卸载输出说明该路径因所有权无法确认而被保留

### 共享浏览器缓存必须保留

`doudou uninstall` 不得删除 venv 之外的 Camoufox 浏览器缓存，因为该缓存不由单个 Doudou npm 安装独占，并可能由其他 Camoufox 工具共用。

#### Scenario: Doudou 与其他工具共用浏览器缓存

- Given Doudou setup 已下载 Camoufox 浏览器
- And 共享 `runtimes/` 中存在 Doudou venv
- When 用户执行 `doudou uninstall`
- Then Doudou `runtimes/` 被删除
- And Camoufox 浏览器缓存保持不变

### 卸载必须停止受管 broker

删除共享 `runtimes/` 前，卸载流程必须识别并停止从该受管 runtime 启动的 Doudou broker，等待进程确认退出后再删除文件。进程归属无法证明时不得终止。

#### Scenario: broker 正在使用受管 venv

- Given 一个 Doudou broker 的可执行 Python 位于待删除的受管 runtime 内
- When 用户执行 `doudou uninstall`
- Then 卸载流程请求该 broker 停止并等待其退出
- And broker 退出后删除共享 `runtimes/`

#### Scenario: 相邻的非 Doudou Python 进程

- Given 系统中同时运行其他 Python 或 Camoufox 进程
- And 这些进程的可执行路径不位于 Doudou 受管 runtime 内
- When 用户执行 `doudou uninstall`
- Then 这些进程不被停止

### 显式清理命令与 npm 卸载限制

CLI 必须提供 `doudou uninstall` 作为唯一完整卸载入口。命令必须先执行全部受管清理；清理成功后，使用当前 npm 包的安装 prefix 自动执行禁用 lifecycle 的 npm 全局卸载，从而删除当前包和 `doudou` 启动器。

npm 11 的真实回归确认 `npm uninstall -g` 不执行被卸载包的卸载 lifecycle，因此文档不得宣称直接使用 npm 命令能够清理 Skill 或 runtime。

#### Scenario: 单命令完整卸载

- Given Doudou 通过 npm 全局安装并存在受管 Skill 与共享 runtime
- When 用户执行 `doudou uninstall`
- Then 命令先完成受管清理
- And 随后从当前安装 prefix 删除 `@follenfang/doudou` npm 包与启动器
- And 用户不需要再运行第二条命令

#### Scenario: 清理失败时不删除 npm 包

- Given 受管 Skill、runtime 或 broker 清理失败
- When 用户执行 `doudou uninstall`
- Then 命令返回失败
- And 不调用 npm 删除当前包或启动器

### 清理失败必须阻止半卸载

`doudou uninstall` 的清理步骤必须在调用 npm 删除启动器前完成。任何受管 broker 无法停止、受管 Skill 无法删除或共享 `runtimes/` 无法删除的错误都必须使命令返回失败，不得调用 npm，并保留启动器及稳定错误和未清理路径。

#### Scenario: runtime 文件仍被占用

- Given 卸载流程无法停止某个已确认归属于 Doudou runtime 的 broker
- Or 受管 runtime 文件因占用或权限无法删除
- When 用户执行 `doudou uninstall`
- Then 命令失败且不调用 npm
- And npm 启动器仍可用于诊断或重试
- And 输出列出未清理对象但不包含凭据或项目数据

#### Scenario: 一个登记 Skill 删除失败

- Given 多个有效登记 Skill 中有一个无法删除
- When 用户执行 `doudou uninstall`
- Then 清理结果为失败
- And 不把已删除对象伪报为仍存在
- And 输出逐项区分 `removed`、`missing`、`preserved_unmanaged` 和 `failed`

### 自定义 Skill 路径

安装器必须在用户级 Doudou 状态中登记所有成功采用、安装或升级的 Skill 目标，包括默认位置、`DOUDOU_AGENT_HOME` 和 `--skill-dir` 指定位置。`doudou uninstall` 必须逐个处理全部登记路径；每个路径只有在管理标记仍有效且与 Doudou 包身份一致时才能删除。

#### Scenario: 多个托管 Skill 目标

- Given 用户先后在默认位置和两个自定义位置安装了 Doudou Skill
- And 三个位置均存在匹配的 Doudou npm 管理标记
- When 用户执行 `doudou uninstall`
- Then 三个 Skill 目录均被删除

#### Scenario: 登记目标已经变为未知目录

- Given 某个已登记路径的管理标记缺失、损坏或包名不匹配
- When 用户执行 `doudou uninstall`
- Then 该路径保持不变
- And 其他标记有效的登记路径仍正常删除
