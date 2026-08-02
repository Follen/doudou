# Runtime layout

所有路径相对调用项目的数据根，默认是当前目录。Skill 和共享 venv 不保存业务数据。Skill 本身也不包含 venv、Python 包或浏览器二进制。

```text
profile/<profile>/
  profile.json
  camoufox-config.json
  browser-data/
  runtime/
    broker.json
    broker.token
    broker.stdout.log
    broker.stderr.log
  .lock.json

message/profiles/<profile>/
  conversations.json
  session-index/latest.json
  session-index/YYYY-MM-DD/*__session-index.json
  plans/<plan-id>.json
  conversations/<kind>__<name>__<conversation-id>/
    conversation.json
    messages.jsonl
    conflicts.jsonl
    media/YYYY-MM-DD/<message-id>/<message-id>.<ext>
  content/users/<sec-uid>/
    identity.json
    videos.jsonl
    video-run.json
  content/videos/<aweme-id>/
    metadata.json
    cover.bin
    comment-checkpoint.json
    comments.partial.jsonl
    comments.jsonl
    comment-run.json
  sends/YYYY-MM-DD/<time>__<receipt-id>/
  interactions/chat/YYYY-MM-DD/<time>__<receipt-id>/
  interactions/video/YYYY-MM-DD/<time>__<receipt-id>/
  diagnostics/page-doctor/<id>/report.json
  diagnostics/captures/<id>/
    manifest.json
    request.bin
    response.bin
  migrations/
```

`kind` 为 `group` 或 `direct`。所有 JSON/JSONL 使用原子替换；不完整 JSONL 会停止覆盖。回执与诊断不保存 Cookie、token、动态签名、上传凭证或完整签名 URL。

用户级运行时位于 `%LOCALAPPDATA%/doudou/runtimes/`（Windows）或 `${XDG_CACHE_HOME:-~/.cache}/doudou/runtimes/`（Linux）；Camoufox 浏览器保存在 Camoufox 的用户级缓存中。这些目录不进入调用项目或 Skill 包。

旧 Chromium Profile 不和 Camoufox 数据库合并。首次使用时整个目录原子改名为 `profile/<name>__chromium-backup-<UTC>/`，随后创建同名 Camoufox Profile；`message/profiles/<name>/` 保持不变。
