from __future__ import annotations

import hashlib
import os
import sys
from pathlib import Path
from typing import Any

from ...common.core import atomic_write_json
from ..environment import runtime_root
from ..paths import ProfilePaths


def broker_registry_root() -> Path:
    return runtime_root().parent / "brokers"


def _registration_path(data_root: Path, profile_id: str) -> Path:
    identity = f"{data_root.resolve()}\0{profile_id}".encode()
    return broker_registry_root() / f"{hashlib.sha256(identity).hexdigest()}.json"


def register_broker(paths: ProfilePaths, descriptor: dict[str, Any]) -> Path | None:
    managed_runtime = runtime_root().resolve()
    interpreter_runtime = Path(sys.prefix).resolve()
    if not interpreter_runtime.is_relative_to(managed_runtime):
        return None
    registration = _registration_path(paths.data_root, paths.profile_id)
    atomic_write_json(
        registration,
        {
            "schema": "doudou.broker-registration.v1",
            "pid": os.getpid(),
            "process_start": descriptor.get("process_start"),
            "runtime": str(interpreter_runtime),
            "executable": str(Path(sys.executable).resolve()),
            "descriptor": str(paths.broker_descriptor.resolve()),
            "token": str(paths.broker_token.resolve()),
        },
        mode=0o600,
    )
    return registration


def unregister_broker(registration: Path | None) -> None:
    if registration is not None:
        registration.unlink(missing_ok=True)
