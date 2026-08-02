from __future__ import annotations

from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from ..common.core import atomic_write_json, random_id, utc_now
from ..runtime.paths import profile_paths


def write_receipt(
    data_root: Path,
    profile: str,
    domain: str,
    operation: str,
    record: dict[str, Any],
    send: bool = False,
) -> dict[str, str]:
    paths = profile_paths(data_root, profile)
    now = datetime.now(timezone.utc)
    identifier = random_id("receipt")
    base = paths.sends if send else paths.interactions / domain
    directory = (
        base / now.date().isoformat() / f"{now.strftime('%H%M%S%f')}__{identifier}"
    )
    receipt = {
        "schema": "doudou.write-receipt.v1",
        "receipt_id": identifier,
        "profile_id": paths.profile_id,
        "domain": domain,
        "operation": operation,
        "completed_at": utc_now(),
        **record,
    }
    atomic_write_json(directory / "receipt.json", receipt)
    atomic_write_json(
        directory / "verification.json", receipt.get("verification") or {}
    )
    return {
        "directory": str(directory.relative_to(data_root)),
        "receipt": str((directory / "receipt.json").relative_to(data_root)),
        "verification": str((directory / "verification.json").relative_to(data_root)),
    }
