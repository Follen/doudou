from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from ..common.errors import DoudouError


@dataclass(frozen=True)
class WebProtocolContract:
    version: str
    paths: dict[str, str]
    fields: dict[str, tuple[str, ...]]

    def path(self, key: str) -> str:
        return self.paths[key]

    def value(self, payload: dict[str, Any], key: str, default: Any = None) -> Any:
        for alias in self.fields[key]:
            if alias in payload:
                return payload[alias]
        return default

    def list_value(
        self, payload: dict[str, Any], key: str, operation: str
    ) -> list[Any]:
        sentinel = object()
        value = self.value(payload, key, sentinel)
        if value is None:
            return []
        if isinstance(value, list):
            return value
        raise DoudouError(
            "PROTOCOL_SCHEMA_DRIFTED",
            "A versioned protocol response no longer matches the expected field contract.",
            {
                "contract_version": self.version,
                "operation": operation,
                "field": key,
                "aliases": list(self.fields[key]),
                "response_keys": sorted(payload),
                "actual_type": "missing" if value is sentinel else type(value).__name__,
            },
        )


WEB_PROTOCOL = WebProtocolContract(
    version="2026-08-02.1",
    paths={
        "video_list": "/aweme/v1/web/aweme/post/",
        "video_detail": "/aweme/v1/web/aweme/detail/",
        "comment_root": "/aweme/v1/web/comment/list/",
        "comment_reply": "/aweme/v1/web/comment/list/reply/",
        "comment_publish": "/aweme/v1/web/comment/publish/",
        "comment_digg": "/aweme/v1/web/comment/digg/",
        "user_info": "/aweme/v1/web/im/user/info/",
    },
    fields={
        "status_code": ("status_code", "statusCode"),
        "video_items": ("aweme_list", "awemeList"),
        "video_cursor": ("max_cursor", "maxCursor"),
        "has_more": ("has_more", "hasMore"),
        "video_detail": ("aweme_detail", "awemeDetail"),
        "comments": ("comments",),
        "comment_cursor": ("cursor",),
        "embedded_replies": ("reply_comment", "reply_comments"),
    },
)
