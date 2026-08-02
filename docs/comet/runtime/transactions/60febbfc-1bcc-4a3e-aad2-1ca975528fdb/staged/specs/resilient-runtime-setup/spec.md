# Resilient runtime setup

## Purpose

定义可观察、可取消、可重试且不会遗留孤儿进程或半成品 runtime 的首次安装流程。

## Requirements

### 用户级安装必须收敛到 DOUDOU_HOME

默认 `DOUDOU_HOME` 为用户主目录下 `.doudou`，环境变量可覆盖。专用 Python、venv、下载、setup lock/state、current pointer、日志、rollback 和 Doudou 专用浏览器必须全部位于该根；Skill 仍位于 Agent Skill 目录，项目 Profile/message 仍位于调用项目。

#### Scenario: Windows 已安装 runtime 被后续命令复用

- Given `current.json` 指向已验证的 Windows runtime
- And Python 位于 `venv/Scripts/python.exe`
- When 用户执行任一业务命令
- Then 就绪检测返回 true 并直接运行
- And 不检查 `venv/bin/python` 或重复 setup

#### Scenario: 覆盖默认安装根

- Given 用户设置 `DOUDOU_HOME` 为自定义路径
- When setup、doctor、update 或 uninstall 运行
- Then 所有受管 runtime 状态只使用解析后的该目录
- And 项目 Profile/message 不移动

### setup 必须自带专用 Python

npm 启动器必须按 runtime manifest 下载并校验固定的 python-build-standalone CPython 3.12.13，不得要求或选择用户系统 Python。专用 Python 和由其创建的 venv 必须位于同一个版本化 runtime 内。

#### Scenario: 用户电脑没有 Python

- Given npm 和 Node 可用但 PATH 中没有 Python
- When 用户首次执行 `doudou setup`
- Then setup 下载并校验平台对应的专用 Python
- And 使用该 Python 创建并验证同级 venv

#### Scenario: 用户 Python 安装损坏

- Given `py` 启动器指向不存在的 Python
- When 用户执行 Doudou setup 或业务命令
- Then Doudou 不调用该启动器
- And 受管 runtime 的结果不受其影响

### setup 必须是用户级单实例任务

同一用户缓存中同时只能有一个 Doudou setup 所有者；其他调用必须等待并显示所有者阶段，完成后复用同一结果。

#### Scenario: 两个首次 setup 并发

- Given Doudou runtime 尚未安装
- When 两个 CLI 同时启动 setup
- Then 只有一个进程创建临时 runtime 并执行安装
- And 第二个进程报告等待状态并在所有者完成后复用结果

### 子进程生命周期必须受父任务控制

pip、Camoufox 和下载器必须属于可统一终止的进程树。取消、阶段超时或父进程退出时，系统必须终止并确认子进程退出。

#### Scenario: 浏览器下载期间取消

- Given setup 正在下载浏览器
- When 调用方取消 setup
- Then 下载进程树退出且没有孤立安装进程
- And current pointer 和原 runtime 不变
- And 下一次 setup 可干净重试

### setup 必须分阶段显示进度并分类错误

venv、dependencies、browser selection、browser download、verification 和 switch 阶段必须输出可观察状态；下载阶段应转发可用的原生进度或传输量。

#### Scenario: 慢速下载仍有活动

- Given 浏览器下载耗时较长但仍在传输
- When 用户观察 setup 输出
- Then 输出持续显示当前阶段和可获得的下载进度
- And 不因等待最终返回而表现为无响应

#### Scenario: 依赖安装失败

- Given pip 按 hash 锁安装失败
- When setup 结束
- Then 返回 `DEPENDENCY_INSTALL_FAILED`
- And 不把错误归类为浏览器下载失败

### runtime 切换必须原子且可回滚

只有新 runtime 的依赖、关键 import、浏览器版本与真实启动验证全部通过后才能替换版本目录和 current pointer。

#### Scenario: 新 runtime 验证失败

- Given 已存在可用 runtime
- And 新 runtime 的浏览器启动验证失败
- When setup 清理失败结果
- Then 旧 runtime 和 current pointer 仍可用
- And 临时 runtime 不会成为 current

### 锁定依赖必须在专用 Python 和诊断矩阵验证

生产发布门禁必须在固定专用 Python 3.12.13 中按 hash 锁安装、运行 `pip check`、导入 browserforge Fingerprint 和 Camoufox AsyncCamoufox，并完成浏览器启动验证。诊断实验仍在干净 Python 3.12 与 3.13 中证明截图故障不是版本天然不兼容。

#### Scenario: 干净 Python 3.13

- Given 没有复用旧 site-packages 的 Python 3.13 venv
- When 按 `requirements.lock` 安装
- Then 锁定版本和 hash 校验通过
- And 关键 import 与浏览器启动验证通过

### update 必须并排构建新 runtime

runtime manifest 变化时，update 必须在临时目录完成专用 Python、venv、依赖和浏览器验证，再原子切换 current pointer；失败不得破坏旧 runtime。

#### Scenario: 专用 Python patch 升级失败

- Given current pointer 指向可用旧 runtime
- And 新 Python 资产下载或校验失败
- When update 返回失败
- Then current pointer 仍指向旧 runtime
- And 旧 CLI 运行能力保留
