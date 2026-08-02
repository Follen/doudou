# Camoufox 生产迁移验收

日期：2026-08-02

## 环境与分发

- 仓库外临时项目依次运行 `doctor`、`setup`、`doctor`、`profile create`、`profile list`，全部退出 0。
- Python 3.12.9、`camoufox==0.5.4`、官方浏览器 `152.0.4-beta.28` 均匹配；真实隐藏有头启动探测通过。
- `setup` 的 venv、依赖、浏览器、验证和原子切换阶段全部为 `passed`，并生成 rollback record。
- 外部项目只在自身创建 `profile/external-smoke/` 与 `message/profiles/external-smoke/`。
- 源 Skill 与 `C:/Users/follen/.agents/skills/doudou/` 均为 76 个文件，清单 SHA-256 为 `a691b2479721640831bd9db064876929bb48f14a825d7444a81fdef90f450ffa`，逐文件一致。
- 两棵 Skill 中均不存在 `profile`、`message`、`analyze`、venv、浏览器、Node 模块、构建目录或 Python 缓存。

## 浏览器与 Broker

- Windows 冷启动在窗口首次 SHOW 前隐藏主窗口；后台状态为 alpha 0、`WS_EX_TOOLWINDOW=true`、`WS_EX_APPWINDOW=false`、任务栏登记 removed、Alt+Tab candidate false。
- 同一 HWND 接管时恢复 alpha 255、普通应用窗口样式、任务栏登记和 Alt+Tab；结束后重新隐藏成功。
- 最终真实连续命令使用同一 broker PID，页面数始终为 1，`initialization_count=1`，空闲超时配置为 1200 秒。
- 后台主窗口未移动到屏幕外，`MozillaBatteryClass` 等辅助窗口未作为业务窗口。

## 读取回归

- 会话协议发现 22 个会话，同时包含 `conversation_type=1` 私聊和 `conversation_type=2` 群聊。
- 授权群目标 ID：SHA-256 `e8204a0564075237edefd0f2732e3203bb64ac0eecc9b442616ea040c9d5c181`，后四位 `3660`。
- 当天群消息 20 条；`member_joined` 2 条；图片 3 张全部下载、AES-GCM 解密、MD5/大小/格式校验通过；未知字段 inventory 正常为空。
- 私聊协议读取成功；所选私聊在当天时间窗内为 0 条消息。
- 抖音号精确解析成功；视频列表通过 Profile Component 受控滚动收集 6 页、95 个视频。
- 视频目标 ID：SHA-256 `a1064a51e3a5d7e04e286cf3947f52fe2e8df69118399d051eb589ad1b80b8df`，后四位 `9417`。
- 视频 SSR 详情与封面下载成功；根评论、回复和父引用共 432 条，分页与检查点完成。
- Page Doctor 在 `/user/` 与 `/chat` 均返回 compatible；未选择会话时聊天组件明确为 `not_exercised`；React/Fiber traversal 为 false。

## 写入回归

- 聊天普通文字、引用、群聊 @、图片各执行一个确认 plan，四个服务端消息均唯一读回；类型、引用 ID 和 @ UID 全部匹配。
- 聊天写读回报告 SHA-256：`5c5775dd173886c255ddb39c35b37f44ec1010a723f7885738d43ab6f7d9cbb6`。
- 聊天点赞回执为 `liked`，协议复查目标用户 reaction 成功。
- 视频发表评论、回复评论和评论点赞均通过页面动作、网络业务状态和协议复查；目标评论 ID SHA-256 为 `445d2688aa00c2d17b4550a2163323c9db139c88ac477b293ecdd9e6d4a2319b`，后四位 `0808`。
- batch 可控时钟测试证明三次成功尝试发生于 0/5/10 秒；第二次失败时第三项为 `skipped` 且没有第三次页面写调用。
- 所有线上写操作失败不自动重放；旧 `unknown` plan 未复用。

## 质量检查

- `python -m pytest -q`：70 passed，退出 0。
- `python -m ruff check src tests`：All checks passed，退出 0。
- `python -m pip wheel . --no-deps`：构建 `doudou-2.0.0-py3-none-any.whl` 成功，94,535 字节，SHA-256 `4c2f952ea3258e0429c3d77df2442c4544ba77c61680d4cbea82cc5d35d2eba3`。
- `quick_validate.py`：源码 Skill 与安装 Skill 均 valid。
- 凭据扫描：245 个交付文件，0 个密码、令牌、Cookie、带认证 URL 或私钥模式命中。
- `git diff --check`：无空白错误。

## 研究证据摘要

- 图片摘要样本报告 SHA-256：`d8a8c1c4267d11f192541585649ad52a71ee19513536ab8a11b2679078855f86`。
- Windows 冷启动证据 SHA-256：`dc3ec435757e14e422c9abd79093d7154a6d61d72d26d9feefd48a77f2e8aa1c`。
- 研究原始文件保留在 Git 忽略的 `analyze/`，不进入 Skill、提交或远端。
