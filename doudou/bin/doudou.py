#!/usr/bin/env python3
from __future__ import annotations

import os
import sys
from pathlib import Path


SKILL_ROOT = Path(__file__).resolve().parents[1]
SOURCE_ROOT = SKILL_ROOT / "src"
sys.path.insert(0, str(SOURCE_ROOT))

from doudou.runtime.environment import reexec_in_runtime  # noqa: E402


def main() -> int:
    command = next((item for item in sys.argv[1:] if item in {"doctor", "setup"}), None)
    if command not in {"doctor", "setup"}:
        if not reexec_in_runtime([str(Path(__file__).resolve()), *sys.argv[1:]]):
            import json

            print(json.dumps({"event": "error", "code": "RUNTIME_SETUP_REQUIRED", "message": "Run doudou setup before business commands."}, separators=(",", ":")), flush=True)
            return 1
    from doudou.cli import main as cli_main

    return cli_main(sys.argv[1:], skill_root=SKILL_ROOT)


if __name__ == "__main__":
    raise SystemExit(main())
