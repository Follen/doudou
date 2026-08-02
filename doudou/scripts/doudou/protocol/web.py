from __future__ import annotations

import json
from typing import Any
from urllib.parse import unquote

from ..models.identity import normalize_identity


def first_url(value: Any) -> str | None:
    if isinstance(value, str) and value:
        return value
    if isinstance(value, list):
        return next((found for item in value if (found := first_url(item))), None)
    if isinstance(value, dict):
        return first_url(
            value.get("url_list")
            or value.get("urlList")
            or value.get("url")
            or value.get("src")
        )
    return None


def identity_from(value: dict[str, Any]) -> dict[str, Any] | None:
    unique_id = value.get("unique_id", value.get("uniqueId"))
    sec_uid = value.get("sec_uid", value.get("secUid"))
    if unique_id is None and sec_uid is None and not value.get("uid"):
        return None
    return normalize_identity(value, source="web-protocol")


def identities_in(value: Any) -> list[dict[str, Any]]:
    output: dict[tuple[str, str, str], dict[str, Any]] = {}
    pending = [(value, 0)]
    while pending:
        current, depth = pending.pop()
        if depth > 8:
            continue
        if isinstance(current, dict):
            identity = identity_from(current)
            if identity:
                output[
                    (identity["unique_id"], identity["sec_uid"], identity["uid"])
                ] = identity
            pending.extend((item, depth + 1) for item in current.values())
        elif isinstance(current, list):
            pending.extend((item, depth + 1) for item in current)
    return sorted(
        output.values(),
        key=lambda item: (
            (item["unique_id"] or "").casefold(),
            item["sec_uid"] or "",
            item["uid"] or "",
        ),
    )


def normalize_video(raw: dict[str, Any]) -> dict[str, Any]:
    author = raw.get("author") or raw.get("author_info") or {}
    video = raw.get("video") or {}
    return {
        "schema": "doudou.video-index-item.v1",
        "aweme_id": str(raw.get("aweme_id") or raw.get("awemeId") or ""),
        "description": raw.get("desc") or "",
        "create_time": raw.get("create_time"),
        "aweme_type": raw.get("aweme_type"),
        "author": normalize_identity(author, source="video-list"),
        "statistics": raw.get("statistics") or raw.get("stats") or {},
        "cover_url": first_url(
            video.get("cover") or video.get("origin_cover") or video.get("originCover")
        ),
        "duration_ms": video.get("duration"),
        "width": video.get("width"),
        "height": video.get("height"),
    }


def parse_render_data(text: str) -> dict[str, Any]:
    for candidate in (text, unquote(text)):
        try:
            value = json.loads(candidate)
            if isinstance(value, dict):
                return value
        except (json.JSONDecodeError, UnicodeDecodeError):
            continue
    raise ValueError("#RENDER_DATA was not valid JSON")


def normalize_video_detail(detail: dict[str, Any]) -> dict[str, Any]:
    author = detail.get("authorInfo") or detail.get("author") or {}
    video = detail.get("video") or {}
    return {
        "schema": "doudou.video.v1",
        "aweme_id": str(detail.get("awemeId") or detail.get("aweme_id") or ""),
        "title": detail.get("itemTitle") or detail.get("title"),
        "description": detail.get("desc") or "",
        "hashtags": [
            item.get("hashtagName")
            for item in detail.get("textExtra") or []
            if item.get("hashtagName")
        ],
        "create_time": detail.get("createTime") or detail.get("create_time"),
        "author": normalize_identity(author, source="video-detail"),
        "statistics": detail.get("stats") or detail.get("statistics") or {},
        "duration_ms": video.get("duration"),
        "width": video.get("width"),
        "height": video.get("height"),
        "cover_url": first_url(video.get("coverUrlList") or video.get("cover")),
        "play_url": first_url(video.get("playAddr") or video.get("play_addr")),
    }


def normalize_comment(raw: dict[str, Any], aweme_id: str) -> dict[str, Any]:
    user = raw.get("user") or {}
    cid = str(raw.get("cid") or "")
    raw_root_id = raw.get("reply_id")
    root_id = cid if raw_root_id in (None, "", 0, "0") else str(raw_root_id)
    raw_parent_id = raw.get("reply_to_reply_id")
    direct_parent = (
        None
        if raw_parent_id in (None, "", 0, "0")
        else str(raw_parent_id)
    )
    return {
        "schema": "doudou.comment.v1",
        "aweme_id": aweme_id,
        "cid": cid,
        "text": raw.get("text") or "",
        "create_time": raw.get("create_time"),
        "digg_count": raw.get("digg_count"),
        "user_digged": bool(raw.get("user_digged")),
        "reported_reply_count": raw.get("reply_comment_total"),
        "reply_id": root_id,
        "reply_to_reply_id": direct_parent,
        "parent_resolved": direct_parent is None,
        "user": normalize_identity(user, source="comment"),
    }


def resolve_comment_parents(comments: list[dict[str, Any]]) -> list[dict[str, Any]]:
    known = {item["cid"] for item in comments}
    for item in comments:
        parent = item.get("reply_to_reply_id")
        item["parent_resolved"] = parent is None or parent in known
    return comments
