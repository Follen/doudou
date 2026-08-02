#!/usr/bin/env python3
from __future__ import annotations

import json
import sys
from pathlib import Path

sys.dont_write_bytecode = True

SKILL_ROOT = Path(__file__).resolve().parents[1]


def main() -> int:
    from doudou.runtime.environment import reexec_in_runtime

    command = next((item for item in sys.argv[1:] if item == "doctor"), None)
    if command != "doctor" and not reexec_in_runtime(
        [str(Path(__file__).resolve()), *sys.argv[1:]]
    ):
        print(
            json.dumps(
                {
                    "event": "error",
                    "code": "RUNTIME_SETUP_REQUIRED",
                    "message": "Run python <skill>/scripts/doudou.py setup before business commands.",
                },
                separators=(",", ":"),
            ),
            flush=True,
        )
        return 1
    from doudou.cli import main as cli_main

    return cli_main(sys.argv[1:], skill_root=SKILL_ROOT)


if __name__ == "__main__":
    raise SystemExit(main())
