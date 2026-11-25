---
name: doudou
description: 通过本地 Chrome Profile (Rust CDP) 读取抖音群聊、私聊、图片、系统事件、用户、视频和评论，并执行经确认的页面写操作。适用于登录接管、聊天与内容采集、多 Profile 管理、经确认写入和运行时诊断。
---

# Doudou

`doudou` 是自解释 CLI，不靠文档背参数。需要找命令或查参数、事件、错误码时直接问 CLI：

```bash
doudou --help
doudou cli search <query>      # 按意图检索命令与参数
doudou cli schema [command]    # 输出机器可读 JSON Schema（参数/事件/错误码）
doudou <command> -h
```

## 必须遵守的约束

1. **门禁顺序**：先 `doudou doctor`（`status=ok` 再继续），再 `doudou profile list` 复用已登录且空闲的 Profile（无则 `profile create` + `session login`）。业务命令自行维护 broker，不要并行抢占同一 Profile。
2. **数据边界**：Profile、Chrome 数据与采集记录（`message/profiles/<name>/`）默认统一存在系统标准 AppData（`%LOCALAPPDATA%\doudou` / `~/Library/Application Support/doudou` / `~/.local/share/doudou`），可分别用 `--home`（`DOUDOU_HOME`）和 `--data-root`（`DOUDOU_DATA_ROOT`）覆盖。
3. **两阶段写入**：写命令默认生成一次性 plan；向用户展示精确预览并获确认后，带 `--confirm --plan-id <id>` 执行一次。遇到 `WRITE_RESULT_UNKNOWN` 先只读核对，禁止自动重试。
4. **结构化输出与隐私**：stdout 为 NDJSON，逐行解析后只投影必要字段；默认不输出 Cookie、token、`sec_uid` 或未要求的聊天正文。
5. **只信真实错误码**：保持 `PROFILE_*`、`AUTH_*`、`WRITE_RESULT_UNKNOWN`、`PROTOCOL_*`、`UI_*` 原始分类，不猜测绕过。
