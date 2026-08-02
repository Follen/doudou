from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from ..common.core import safe_segment


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


def initialize_data_root(data_root: Path) -> None:
    data_root.mkdir(parents=True, exist_ok=True)
    profile = data_root / "profile"
    legacy = data_root / "Profile"
    if legacy.exists() and profile.exists() and not legacy.samefile(profile):
        raise RuntimeError("PROFILE_ROOT_CONFLICT")
    if legacy.exists() and not profile.exists():
        legacy.replace(profile)
    profile.mkdir(parents=True, exist_ok=True)
    (data_root / "message").mkdir(parents=True, exist_ok=True)
