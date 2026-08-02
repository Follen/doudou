from __future__ import annotations

import os
import platform
import secrets
import shutil
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from ..common.core import atomic_write_json, read_json, utc_now
from ..common.errors import DoudouError
from .lock import lock_is_live
from .paths import (
    ProfilePaths,
    initialize_data_root,
    profile_paths,
    stored_profile_paths,
)

CAMOUFOX_PACKAGE_VERSION = "0.5.4"
CAMOUFOX_BROWSER_VERSION = "152.0.4-beta.28"


def default_identity_config(profile_id: str) -> dict[str, Any]:
    target_os = "windows" if os.name == "nt" else "linux"
    return {
        "schema": "doudou.camoufox-config.v1",
        "profile_id": profile_id,
        "version": 1,
        "target_os": target_os,
        "locale": "zh-CN",
        "timezone": "Asia/Shanghai",
        "config": {
            "fonts:spacing_seed": secrets.randbelow(4_294_967_294) + 1,
            "audio:seed": secrets.randbelow(4_294_967_294) + 1,
            "canvas:seed": secrets.randbelow(4_294_967_294) + 1,
        },
        "generated_fingerprint": None,
        "created_at": utc_now(),
        "updated_at": utc_now(),
    }


def create_profile(
    data_root: Path, profile_name: str, display_name: str | None = None
) -> tuple[dict[str, Any], ProfilePaths]:
    initialize_data_root(data_root)
    paths = profile_paths(data_root, profile_name)
    if paths.metadata.exists():
        raise DoudouError(
            "PROFILE_EXISTS",
            f"Profile already exists: {profile_name}.",
            {"profile_id": paths.profile_id},
        )
    for directory in (
        paths.browser_data,
        paths.runtime,
        paths.conversations,
        paths.session_index,
        paths.runs,
        paths.plans,
        paths.sends,
        paths.interactions,
        paths.diagnostics,
    ):
        directory.mkdir(parents=True, exist_ok=True)
    timestamp = utc_now()
    metadata = {
        "schema": "doudou.profile.v2",
        "profile_id": paths.profile_id,
        "display_name": display_name or profile_name,
        "runtime": "camoufox-python",
        "camoufox_package_version": CAMOUFOX_PACKAGE_VERSION,
        "browser_version": CAMOUFOX_BROWSER_VERSION,
        "browser_data_dir": "browser-data",
        "fingerprint_config": "camoufox-config.json",
        "created_at": timestamp,
        "updated_at": timestamp,
        "last_login_verified_at": None,
        "login_status": "login_required",
        "last_error_code": None,
    }
    atomic_write_json(
        paths.camoufox_config, default_identity_config(paths.profile_id), mode=0o600
    )
    atomic_write_json(paths.metadata, metadata)
    return metadata, paths


def load_profile(
    data_root: Path, profile_name: str
) -> tuple[dict[str, Any], ProfilePaths]:
    initialize_data_root(data_root)
    paths = profile_paths(data_root, profile_name)
    metadata = read_json(paths.metadata, optional=True)
    if metadata is None:
        raise DoudouError(
            "PROFILE_NOT_FOUND",
            f"Profile not found: {profile_name}.",
            {"profile_id": paths.profile_id},
        )
    if (
        metadata.get("schema") != "doudou.profile.v2"
        or metadata.get("runtime") != "camoufox-python"
    ):
        return migrate_chromium_profile(data_root, profile_name, metadata)
    if metadata.get("profile_id") != paths.profile_id:
        raise DoudouError(
            "PROFILE_METADATA_MISMATCH",
            "Profile metadata does not match its directory.",
            {"path": str(paths.metadata)},
        )
    return metadata, paths


def migrate_chromium_profile(
    data_root: Path, profile_name: str, legacy_metadata: dict[str, Any]
) -> tuple[dict[str, Any], ProfilePaths]:
    paths = profile_paths(data_root, profile_name)
    if legacy_metadata.get("schema") not in {
        "doudou.profile.v1",
        "douyin-message.profile.v1",
    }:
        raise DoudouError(
            "PROFILE_RUNTIME_UNKNOWN",
            "The existing Profile runtime cannot be identified for migration.",
            {"path": str(paths.metadata), "schema": legacy_metadata.get("schema")},
        )
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    backup = paths.profile.with_name(f"{paths.profile_id}__chromium-backup-{stamp}")
    if backup.exists():
        raise DoudouError(
            "PROFILE_BACKUP_EXISTS",
            "The Chromium Profile backup target already exists.",
            {"backup": str(backup)},
        )
    migration_path = (
        paths.message / "migrations" / f"{stamp}__chromium-to-camoufox.json"
    )
    paths.message.mkdir(parents=True, exist_ok=True)
    try:
        os.replace(paths.profile, backup)
        metadata, new_paths = create_profile(
            data_root, profile_name, legacy_metadata.get("display_name")
        )
        atomic_write_json(
            migration_path,
            {
                "schema": "doudou.profile-migration.v1",
                "profile_id": paths.profile_id,
                "migrated_at": utc_now(),
                "source_runtime": "chromium-playwright",
                "target_runtime": "camoufox-python",
                "source_schema": legacy_metadata.get("schema"),
                "backup_path": str(backup.relative_to(data_root)),
                "new_profile_path": str(new_paths.profile.relative_to(data_root)),
                "atomic_rename": "passed",
                "message_directory_reused": str(paths.message.relative_to(data_root)),
            },
        )
        return metadata, new_paths
    except Exception as error:
        if paths.profile.exists() and backup.exists():
            shutil.rmtree(paths.profile)
        if backup.exists():
            os.replace(backup, paths.profile)
        raise DoudouError(
            "PROFILE_MIGRATION_FAILED",
            "The Chromium Profile was restored after Camoufox migration failed.",
            {"profile_id": paths.profile_id, "error_type": type(error).__name__},
        ) from error


def update_profile(
    data_root: Path, profile_name: str, patch: dict[str, Any]
) -> dict[str, Any]:
    metadata, paths = load_profile(data_root, profile_name)
    metadata.update(patch)
    metadata["updated_at"] = utc_now()
    atomic_write_json(paths.metadata, metadata)
    return metadata


def list_profiles(data_root: Path) -> list[dict[str, Any]]:
    initialize_data_root(data_root)
    profiles: list[dict[str, Any]] = []
    for entry in (data_root / "profile").iterdir():
        if not entry.is_dir():
            continue
        metadata = read_json(entry / "profile.json", optional=True)
        if metadata:
            if entry.name != str(metadata.get("profile_id") or ""):
                continue
            paths = stored_profile_paths(data_root, str(metadata["profile_id"]))
            conversation_index = read_json(
                paths.message / "conversations.json", optional=True
            )
            session_index = read_json(
                paths.session_index / "latest.json", optional=True
            )
            content = paths.message / "content"
            users = content / "users"
            videos = content / "videos"
            receipt_files = list(paths.sends.glob("*/*/receipt.json")) + list(
                paths.interactions.glob("*/*/*/receipt.json")
            )
            recent_write_at = None
            if receipt_files:
                recent_write_at = datetime.fromtimestamp(
                    max(path.stat().st_mtime for path in receipt_files), timezone.utc
                ).isoformat(timespec="milliseconds").replace("+00:00", "Z")
            profiles.append(
                {
                    **metadata,
                    "lock_status": "in_use" if lock_is_live(paths.lock) else "free",
                    "last_session_index_refresh": (
                        session_index.get("completed_at")
                        if isinstance(session_index, dict)
                        else None
                    ),
                    "chat_summary": {
                        "conversation_count": (
                            int(conversation_index.get("count") or 0)
                            if isinstance(conversation_index, dict)
                            else 0
                        ),
                        "stored_conversation_count": sum(
                            child.is_dir()
                            for child in paths.conversations.iterdir()
                            if child.name != "media"
                        ),
                    },
                    "content_summary": {
                        "user_count": (
                            sum(child.is_dir() for child in users.iterdir())
                            if users.is_dir()
                            else 0
                        ),
                        "video_count": (
                            sum(child.is_dir() for child in videos.iterdir())
                            if videos.is_dir()
                            else 0
                        ),
                    },
                    "recent_write_at": recent_write_at,
                }
            )
    return sorted(profiles, key=lambda item: item.get("created_at", ""))


def platform_background_mode() -> str:
    if os.name == "nt":
        return "windows_headless"
    if platform.system() == "Linux":
        return "linux_virtual_headed"
    return "headed"
