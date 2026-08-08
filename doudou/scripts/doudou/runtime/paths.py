from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from ..common.core import safe_segment
from ..common.errors import DoudouError

CURRENT_PROFILE_SCHEMA = "doudou.profile.v2"
CURRENT_PROFILE_RUNTIME = "camoufox-python"
CURRENT_IDENTITY_SCHEMA = "doudou.user-identity.v1"

_IDENTITY_KEYS = {
    "sender",
    "author",
    "user",
    "mention",
    "mentions",
    "reference_user",
    "source_users",
    "joined_users",
    "participants",
    "members",
    "mentioned_users",
}
_IDENTITY_CONTAINER_KEYS = {
    "candidates",
    "comments",
    "conversations",
    "messages",
    "records",
    "replies",
    "reply",
    "videos",
}


@dataclass(frozen=True)
class ProfilePaths:
    data_root: Path
    profile_id: str
    profile: Path
    browser_data: Path
    metadata: Path
    camoufox_config: Path
    lock: Path
    runtime: Path
    broker_descriptor: Path
    broker_token: Path
    broker_stdout: Path
    broker_stderr: Path
    message: Path
    conversations: Path
    session_index: Path
    runs: Path
    plans: Path
    sends: Path
    interactions: Path
    diagnostics: Path
    identities: Path


def _profile_paths(data_root: Path, profile_id: str) -> ProfilePaths:
    profile = data_root / "profile" / profile_id
    runtime = profile / "runtime"
    message = data_root / "message" / "profiles" / profile_id
    return ProfilePaths(
        data_root=data_root,
        profile_id=profile_id,
        profile=profile,
        browser_data=profile / "browser-data",
        metadata=profile / "profile.json",
        camoufox_config=profile / "camoufox-config.json",
        lock=profile / ".lock.json",
        runtime=runtime,
        broker_descriptor=runtime / "broker.json",
        broker_token=runtime / "broker.token",
        broker_stdout=runtime / "broker.stdout.log",
        broker_stderr=runtime / "broker.stderr.log",
        message=message,
        conversations=message / "conversations",
        session_index=message / "session-index",
        runs=message / "runs",
        plans=message / "plans",
        sends=message / "sends",
        interactions=message / "interactions",
        diagnostics=message / "diagnostics",
        identities=message / "identities",
    )


def profile_paths(data_root: Path, profile_name: str) -> ProfilePaths:
    return _profile_paths(data_root, safe_segment(profile_name))


def stored_profile_paths(data_root: Path, profile_id: str) -> ProfilePaths:
    if not profile_id or profile_id in {".", ".."} or Path(profile_id).name != profile_id:
        raise ValueError("invalid stored profile id")
    return _profile_paths(data_root, profile_id)


def _legacy_unsupported(
    legacy_format: str, path: str, supported_path: str
) -> DoudouError:
    return DoudouError(
        "LEGACY_UNSUPPORTED",
        "Legacy Doudou data is not supported by this version.",
        {
            "legacy_format": legacy_format,
            "path": path,
            "supported_path": supported_path,
        },
    )


def _exact_child(parent: Path, name: str) -> Path | None:
    if not parent.is_dir():
        return None
    return next((child for child in parent.iterdir() if child.name == name), None)


def is_current_profile_metadata(metadata: Any) -> bool:
    return (
        isinstance(metadata, dict)
        and metadata.get("schema") == CURRENT_PROFILE_SCHEMA
        and metadata.get("runtime") == CURRENT_PROFILE_RUNTIME
    )


def assert_supported_profile_metadata(metadata: Any) -> None:
    if not is_current_profile_metadata(metadata):
        raise _legacy_unsupported(
            "browser_profile",
            "profile/<PROFILE>/profile.json",
            f"{CURRENT_PROFILE_SCHEMA}:{CURRENT_PROFILE_RUNTIME}",
        )


def initialize_data_root(data_root: Path) -> None:
    legacy = _exact_child(data_root, "Profile")
    if legacy is not None:
        raise _legacy_unsupported("profile_root", "Profile/", "profile/")
    data_root.mkdir(parents=True, exist_ok=True)
    profile = data_root / "profile"
    profile.mkdir(parents=True, exist_ok=True)
    (data_root / "message").mkdir(parents=True, exist_ok=True)


def _has_legacy_identity(value: Any, identity_context: bool = False) -> bool:
    if isinstance(value, list):
        return any(_has_legacy_identity(item, identity_context) for item in value)
    if not isinstance(value, dict):
        return False
    if (
        identity_context
        and any(value.get(key) for key in ("uid", "sec_uid", "unique_id", "secUid"))
        and value.get("schema") != CURRENT_IDENTITY_SCHEMA
    ):
        return True
    for key in _IDENTITY_KEYS:
        if key in value and _has_legacy_identity(value[key], True):
            return True
    for key in _IDENTITY_CONTAINER_KEYS:
        if key in value and _has_legacy_identity(value[key]):
            return True
    if isinstance(value.get("system_event"), dict) and _has_legacy_identity(
        value["system_event"]
    ):
        return True
    return isinstance(value.get("property_list"), dict) and _has_legacy_identity(
        value["property_list"], True
    )


def _file_has_legacy_identity(filename: Path) -> bool:
    try:
        if filename.suffix == ".json":
            return _has_legacy_identity(
                json.loads(filename.read_text(encoding="utf-8"))
            )
        with filename.open(encoding="utf-8") as stream:
            return any(
                _has_legacy_identity(json.loads(line))
                for line in stream
                if line.strip()
            )
    except (OSError, UnicodeDecodeError, json.JSONDecodeError):
        return False


def _assert_supported_identity_root(message_root: Path) -> None:
    if not message_root.is_dir():
        return
    for filename in message_root.rglob("*"):
        if not filename.is_file() or filename.suffix not in {".json", ".jsonl"}:
            continue
        if _file_has_legacy_identity(filename):
            raise _legacy_unsupported(
                "identity_record",
                "message/profiles/<PROFILE>/<RECORD>",
                CURRENT_IDENTITY_SCHEMA,
            )


def assert_supported_identity_records(data_root: Path, profile_name: str) -> None:
    _assert_supported_identity_root(profile_paths(data_root, profile_name).message)
