from __future__ import annotations

import json
from copy import deepcopy
from pathlib import Path
from typing import Any

from ..common.core import (
    atomic_write_bytes,
    atomic_write_json,
    atomic_write_jsonl,
    read_json,
    safe_segment,
    utc_now,
)
from ..runtime.paths import profile_paths


def _merge_comment_value(existing: Any, incoming: Any) -> Any:
    if incoming is None or incoming == "":
        return deepcopy(existing)
    if isinstance(existing, dict) and isinstance(incoming, dict):
        merged = deepcopy(existing)
        for key, value in incoming.items():
            merged[key] = (
                _merge_comment_value(merged[key], value)
                if key in merged
                else deepcopy(value)
            )
        return merged
    return deepcopy(incoming)


def merge_video_comments(
    existing: list[dict[str, Any]], current: list[dict[str, Any]]
) -> list[dict[str, Any]]:
    """Merge normalized comments by cid while preserving stable input order."""
    merged: list[dict[str, Any]] = []
    positions: dict[str, int] = {}
    for comment in [*existing, *current]:
        cid = comment.get("cid")
        if cid is None or cid == "":
            continue
        key = str(cid)
        if key not in positions:
            positions[key] = len(merged)
            merged.append(deepcopy(comment))
            continue
        index = positions[key]
        merged[index] = _merge_comment_value(merged[index], comment)
    return merged


def user_directory(data_root: Path, profile: str, identity: dict[str, Any]) -> Path:
    stable = identity.get("sec_uid") or identity.get("uid") or identity.get("unique_id")
    return (
        profile_paths(data_root, profile).message
        / "content"
        / "users"
        / safe_segment(stable, 128)
    )


def video_directory(data_root: Path, profile: str, aweme_id: str) -> Path:
    return (
        profile_paths(data_root, profile).message
        / "content"
        / "videos"
        / safe_segment(aweme_id, 64)
    )


def read_comment_checkpoint(
    data_root: Path, profile: str, aweme_id: str
) -> tuple[dict[str, Any] | None, list[dict[str, Any]]]:
    directory = video_directory(data_root, profile, aweme_id)
    checkpoint = read_json(directory / "comment-checkpoint.json", optional=True)
    comments: list[dict[str, Any]] = []
    partial = directory / "comments.partial.jsonl"
    if isinstance(checkpoint, dict) and partial.exists():
        for line_number, line in enumerate(
            partial.read_text(encoding="utf-8").splitlines(), 1
        ):
            if not line.strip():
                continue
            value = json.loads(line)
            if not isinstance(value, dict) or not value.get("cid"):
                raise ValueError(
                    f"Invalid comment checkpoint record at line {line_number}"
                )
            comments.append(value)
    return (checkpoint if isinstance(checkpoint, dict) else None), comments


def read_video_comments(
    data_root: Path, profile: str, aweme_id: str
) -> list[dict[str, Any]]:
    path = video_directory(data_root, profile, aweme_id) / "comments.jsonl"
    if not path.exists():
        return []
    comments: list[dict[str, Any]] = []
    for line_number, line in enumerate(
        path.read_text(encoding="utf-8").splitlines(), 1
    ):
        if not line.strip():
            continue
        value = json.loads(line)
        if not isinstance(value, dict) or not value.get("cid"):
            raise ValueError(f"Invalid comment record at line {line_number}")
        comments.append(value)
    return comments


def write_comment_checkpoint(
    data_root: Path,
    profile: str,
    aweme_id: str,
    checkpoint: dict[str, Any],
    comments: list[dict[str, Any]],
) -> dict[str, str]:
    directory = video_directory(data_root, profile, aweme_id)
    ordered = sorted(
        {item["cid"]: item for item in comments if item.get("cid")}.values(),
        key=lambda item: (item.get("create_time") or 0, item["cid"]),
    )
    atomic_write_jsonl(directory / "comments.partial.jsonl", ordered)
    record = {
        "schema": "doudou.comment-checkpoint.v1",
        "aweme_id": aweme_id,
        **checkpoint,
        "comment_ids": [item["cid"] for item in ordered],
        "comment_count": len(ordered),
        "updated_at": utc_now(),
    }
    atomic_write_json(directory / "comment-checkpoint.json", record)
    return {
        "checkpoint": str(
            (directory / "comment-checkpoint.json").relative_to(data_root)
        ),
        "partial_comments": str(
            (directory / "comments.partial.jsonl").relative_to(data_root)
        ),
    }


def write_identity(
    data_root: Path,
    profile: str,
    identity: dict[str, Any],
    candidates: list[dict[str, Any]],
) -> dict[str, str]:
    directory = user_directory(data_root, profile, identity)
    record = {
        "schema": "doudou.identity.v1",
        **identity,
        "candidates": candidates,
        "resolved_at": utc_now(),
    }
    atomic_write_json(directory / "identity.json", record)
    return {
        "directory": str(directory.relative_to(data_root)),
        "identity": str((directory / "identity.json").relative_to(data_root)),
    }


def write_video_index(
    data_root: Path,
    profile: str,
    identity: dict[str, Any],
    videos: list[dict[str, Any]],
    pages: list[dict[str, Any]],
) -> dict[str, Any]:
    directory = user_directory(data_root, profile, identity)
    atomic_write_jsonl(directory / "videos.jsonl", videos)
    atomic_write_json(
        directory / "video-run.json",
        {
            "schema": "doudou.video-index-run.v1",
            "completed_at": utc_now(),
            "count": len(videos),
            "pages": pages,
        },
    )
    return {
        "directory": str(directory.relative_to(data_root)),
        "count": len(videos),
        "videos": str((directory / "videos.jsonl").relative_to(data_root)),
    }


def write_video(
    data_root: Path,
    profile: str,
    detail: dict[str, Any],
    cover: bytes | None,
    comments: list[dict[str, Any]],
    pages: list[dict[str, Any]],
) -> dict[str, Any]:
    directory = video_directory(data_root, profile, detail["aweme_id"])
    atomic_write_json(
        directory / "metadata.json", {**detail, "collected_at": utc_now()}
    )
    if cover is not None:
        atomic_write_bytes(directory / "cover.bin", cover)
    atomic_write_jsonl(directory / "comments.jsonl", comments)
    atomic_write_json(
        directory / "comment-run.json",
        {
            "schema": "doudou.comment-run.v1",
            "completed_at": utc_now(),
            "count": len(comments),
            "pages": pages,
        },
    )
    return {
        "directory": str(directory.relative_to(data_root)),
        "comments": len(comments),
        "cover_downloaded": cover is not None,
    }
