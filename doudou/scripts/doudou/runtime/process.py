from __future__ import annotations

import os
import subprocess
from datetime import datetime, timezone
from typing import Any


def process_identity(pid: int) -> dict[str, Any] | None:
    if pid <= 0:
        return None
    if os.name == "nt":
        script = (
            "[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); "
            f'$p = Get-CimInstance Win32_Process -Filter "ProcessId = {pid}"; '
            "if ($null -eq $p) { exit 3 }; "
            '$o = [ordered]@{ pid=[int]$p.ProcessId; start=[string]$p.CreationDate.ToUniversalTime().ToString("o"); command=[string]$p.CommandLine }; '
            "$o | ConvertTo-Json -Compress"
        )
        result = subprocess.run(
            ["powershell", "-NoProfile", "-NonInteractive", "-Command", script],
            capture_output=True,
            check=False,
            text=True,
            encoding="utf-8",
            errors="replace",
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        if result.returncode:
            return None
        import json

        try:
            return json.loads(result.stdout or "")
        except (TypeError, json.JSONDecodeError):
            return None
    try:
        os.kill(pid, 0)
    except OSError:
        return None
    proc_path = f"/proc/{pid}"
    if os.path.exists(proc_path):
        try:
            with open(f"{proc_path}/stat", encoding="utf-8") as stat_file:
                process_stat = stat_file.read()
        except OSError:
            return None
        closing_parenthesis = process_stat.rfind(")")
        if closing_parenthesis >= 0:
            fields = process_stat[closing_parenthesis + 2 :].split()
            if fields and fields[0] == "Z":
                return None
    stat = os.stat(proc_path) if os.path.exists(proc_path) else None
    return {
        "pid": pid,
        "start": datetime.fromtimestamp(stat.st_ctime, timezone.utc).isoformat()
        if stat
        else None,
        "command": None,
    }


def same_process(
    expected: dict[str, Any] | None, actual: dict[str, Any] | None
) -> bool:
    if not expected or not actual or expected.get("pid") != actual.get("pid"):
        return False
    if not expected.get("start") or not actual.get("start"):
        return True
    try:
        left = datetime.fromisoformat(str(expected["start"]).replace("Z", "+00:00"))
        right = datetime.fromisoformat(str(actual["start"]).replace("Z", "+00:00"))
    except ValueError:
        return False
    return abs((left - right).total_seconds()) < 2
