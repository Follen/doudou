from __future__ import annotations

import os
from pathlib import Path


def doudou_home() -> Path:
    configured = os.environ.get("DOUDOU_HOME")
    if not configured:
        return Path.home() / ".doudou"
    candidate = Path(configured).expanduser()
    if not candidate.is_absolute():
        candidate = Path.home() / candidate
    return candidate.resolve()


def browser_root() -> Path:
    return doudou_home() / "browsers"
