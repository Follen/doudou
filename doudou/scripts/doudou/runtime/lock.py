from __future__ import annotations

import os
from pathlib import Path
from typing import Any

from ..common.core import read_json, utc_now
from ..common.errors import DoudouError
from .process import process_identity, same_process


class ProfileLock:
    def __init__(
        self, path: Path, profile_id: str, data_root: Path, command: str = "broker"
    ) -> None:
        self.path = path
        self.profile_id = profile_id
        self.data_root = data_root
        self.command = command
        self.owner: dict[str, Any] | None = None

    def acquire(self) -> dict[str, Any]:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        identity = process_identity(os.getpid()) or {
            "pid": os.getpid(),
            "start": None,
            "command": None,
        }
        owner = {
            "schema": "doudou.profile-lock.v2",
            "profile_id": self.profile_id,
            "data_root": str(self.data_root),
            "pid": os.getpid(),
            "process_start": identity.get("start"),
            "command": self.command,
            "created_at": utc_now(),
        }
        try:
            descriptor = os.open(self.path, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
        except FileExistsError as error:
            existing = read_json(self.path, optional=True)
            actual = (
                process_identity(int(existing.get("pid", 0)))
                if isinstance(existing, dict)
                else None
            )
            expected = (
                {"pid": existing.get("pid"), "start": existing.get("process_start")}
                if isinstance(existing, dict)
                else None
            )
            if not same_process(expected, actual):
                self.path.unlink(missing_ok=True)
                return self.acquire()
            raise DoudouError(
                "PROFILE_IN_USE",
                f"Profile {self.profile_id} is already owned by a broker.",
                {
                    "owner": {
                        key: existing.get(key)
                        for key in (
                            "pid",
                            "process_start",
                            "command",
                            "profile_id",
                            "data_root",
                        )
                    }
                },
            ) from error
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            import json

            json.dump(owner, handle, ensure_ascii=False, indent=2)
            handle.write("\n")
        self.owner = owner
        return owner

    def release(self) -> None:
        if self.owner is None:
            return
        current = read_json(self.path, optional=True)
        if (
            isinstance(current, dict)
            and current.get("pid") == self.owner["pid"]
            and current.get("process_start") == self.owner["process_start"]
        ):
            self.path.unlink(missing_ok=True)
        self.owner = None


def lock_is_live(path: Path) -> bool:
    owner = read_json(path, optional=True)
    if not isinstance(owner, dict):
        return False
    return same_process(
        {"pid": owner.get("pid"), "start": owner.get("process_start")},
        process_identity(int(owner.get("pid", 0))),
    )
