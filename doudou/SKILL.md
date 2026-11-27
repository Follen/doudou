---
name: doudou
description: 通过本地 Chrome Profile (Rust CDP) 读取抖音群聊、私聊、图片、系统事件、用户、视频和评论，并执行经确认的页面写操作。适用于登录接管、无头扫码/短信登录、聊天与内容采集、多 Profile 管理、经确认写入和运行时诊断。
---

# Doudou

`doudou` 是自解释 CLI，不靠文档背参数。查命令、参数、事件与错误码直接问 CLI：

```bash
doudou --help
doudou cli search <query>      # 按意图检索命令与参数
doudou cli schema [command]    # 输出机器可读 JSON Schema（参数/事件/错误码）
doudou <command> -h
```

## 必须遵守的约束

1. **执行顺序**：先 `doudou doctor`（`status=ok` 再继续），再 `doudou profile list` 复用已登录且空闲的 Profile；不要并行抢占同一 Profile。
2. **无头登录**：无界面或远程环境优先后台运行无头登录以保活浏览器并自动回调；按宿主能力展示二维码（HTML 倒计时卡片 / 本地图片 / 终端字符码），遇到滑块或 Passkey 按原错误码中止，不绕过。
3. **两阶段写入**：写操作先生成预览，经用户明确确认后再执行一次；遇到 `WRITE_RESULT_UNKNOWN` 先只读核对，禁止自动重试。
4. **隐私与输出**：stdout 为 NDJSON，只投影必要字段；严禁在日志或回复中暴露 Cookie、token、手机号、短信验证码、`sec_uid` 或未要求的聊天正文。
5. **原样信错误**：按 CLI 返回的原始错误码处理，不臆造或静默绕过。
