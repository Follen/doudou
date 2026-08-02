from __future__ import annotations

import importlib.metadata
import json
import os
import platform
import subprocess
import sys
import threading
from pathlib import Path
from typing import Any

from .camoufox_asset import browser_status
from .home import browser_root, doudou_home
from .platform.windows import WindowVisibilityGuard
from .process import process_identity
from .profile import (
    CAMOUFOX_BROWSER_VERSION,
    CAMOUFOX_PACKAGE_VERSION,
    platform_background_mode,
)

RUNTIME_ID = f"py{sys.version_info.major}{sys.version_info.minor}-camoufox-{CAMOUFOX_PACKAGE_VERSION}-browser-{CAMOUFOX_BROWSER_VERSION}"


def user_cache_root() -> Path:
    return doudou_home()


def runtime_root() -> Path:
    return user_cache_root() / "runtimes"


def current_pointer() -> Path:
    return runtime_root() / "current.json"


def runtime_directory(runtime_id: str = RUNTIME_ID) -> Path:
    return runtime_root() / runtime_id


def venv_python(directory: Path) -> Path:
    return directory / "venv" / (
        "Scripts/python.exe" if os.name == "nt" else "bin/python"
    )


def hidden_process_options() -> dict[str, Any]:
    if os.name != "nt":
        return {}
    startupinfo = subprocess.STARTUPINFO()
    startupinfo.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    startupinfo.wShowWindow = 0
    return {"creationflags": subprocess.CREATE_NO_WINDOW, "startupinfo": startupinfo}


def package_version(name: str) -> str | None:
    try:
        return importlib.metadata.version(name)
    except importlib.metadata.PackageNotFoundError:
        return None


def environment_package_version(python: Path, name: str) -> str | None:
    result = subprocess.run(
        [
            str(python),
            "-c",
            "import importlib.metadata,sys; print(importlib.metadata.version(sys.argv[1]))",
            name,
        ],
        capture_output=True,
        check=False,
        text=True,
        timeout=30,
        **hidden_process_options(),
    )
    return result.stdout.strip() if result.returncode == 0 else None


def launch_browser_probe(python: Path) -> dict[str, Any]:
    headless = "'virtual'" if platform.system() == "Linux" else "False"
    script = (
        "import asyncio,subprocess,sys\nfrom pathlib import Path\n"
        "from camoufox import multiversion,pkgman\nfrom camoufox.async_api import AsyncCamoufox\n"
        f"cache=Path({str(browser_root())!r})\n"
        "pkgman.INSTALL_DIR=cache; multiversion.INSTALL_DIR=cache\n"
        "multiversion.BROWSERS_DIR=cache/'browsers'; multiversion.CONFIG_FILE=cache/'config.json'; multiversion.COMPAT_FLAG=cache/'.0.5_FLAG'\n"
        "if sys.platform == 'win32':\n"
        " original_create = asyncio.create_subprocess_exec\n"
        " async def hidden_create(*args, **kwargs):\n"
        "  kwargs['creationflags'] = int(kwargs.get('creationflags', 0)) | subprocess.CREATE_NO_WINDOW\n"
        "  startupinfo = kwargs.get('startupinfo') or subprocess.STARTUPINFO()\n"
        "  startupinfo.dwFlags |= subprocess.STARTF_USESHOWWINDOW\n"
        "  startupinfo.wShowWindow = 0\n"
        "  kwargs['startupinfo'] = startupinfo\n"
        "  return await original_create(*args, **kwargs)\n"
        " asyncio.create_subprocess_exec = hidden_create\n"
        f"async def probe():\n async with AsyncCamoufox(headless={headless}, browser='{CAMOUFOX_BROWSER_VERSION}') as browser:\n"
        "  page=await browser.new_page(); await page.goto('about:blank'); print(await page.title())\n"
        "asyncio.run(probe())"
    )
    guard = WindowVisibilityGuard(os.getpid(), visible=False)
    guard.start()
    try:
        process = subprocess.Popen(
            [str(python), "-c", script],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            **hidden_process_options(),
        )
        stdout, stderr = process.communicate(timeout=90)
    except (OSError, subprocess.TimeoutExpired) as error:
        if "process" in locals() and process.poll() is None:
            process.kill()
        return {"passed": False, "error": type(error).__name__}
    finally:
        guard.stop()
    return {
        "passed": process.returncode == 0,
        "exit_code": process.returncode,
        "stderr_tail": stderr[-1000:] if process.returncode else "",
        "window_guard": {
            "events_seen": guard.events_seen,
            "hidden_before_show": guard.windows_hidden_before_show,
            "create_events_seen": guard.create_events_seen,
            "show_events_seen": guard.show_events_seen,
            "show_events_prepared": guard.show_events_prepared,
            "show_events_unprepared": guard.show_events_unprepared,
            "max_callback_latency_ms": guard.max_callback_latency_ms,
            "job_filter_active": guard.job_filter_active,
            "window_events": list(guard.window_events),
            "startup_worker_ready": guard.startup_worker_ready,
            "startup_worker_max_callback_latency_ms": (
                guard.startup_worker_max_callback_latency_ms
            ),
            "startup_worker_events": list(guard.startup_worker_events),
            "startup_worker_poll_events": list(
                guard.startup_worker_poll_events
            ),
        },
    }


def browser_version(python: Path | None = None) -> dict[str, Any]:
    return browser_status(browser_root(), CAMOUFOX_BROWSER_VERSION)


def setup_download_status() -> dict[str, Any]:
    root = user_cache_root()
    state: dict[str, Any] | None = None
    try:
        state = json.loads((root / "setup-state.json").read_text(encoding="utf-8"))
    except (FileNotFoundError, json.JSONDecodeError):
        pass
    partials = []
    downloads = root / "downloads"
    if downloads.exists():
        partials = [
            {"name": item.name, "bytes": item.stat().st_size}
            for item in sorted(downloads.glob("*.part"))
        ]
    owner_alive = bool(
        state
        and state.get("stage") not in {"complete", "failed"}
        and process_identity(int(state.get("pid") or 0))
    )
    return {
        "state": state,
        "owner_alive": owner_alive,
        "partials": partials,
        "repair_command": "doudou setup"
        if not state or state.get("stage") == "failed"
        else None,
    }


def doctor(data_root: Path, launch_probe: bool = False) -> dict[str, Any]:
    pointer: dict[str, Any] | None = None
    try:
        pointer = json.loads(current_pointer().read_text(encoding="utf-8"))
    except (FileNotFoundError, json.JSONDecodeError):
        pass
    selected_dir = Path(pointer["path"]) if pointer and pointer.get("path") else runtime_directory()
    selected_python = venv_python(selected_dir)
    selected_runtime_id = pointer.get("runtime_id") if pointer else RUNTIME_ID
    process_active = (
        os.environ.get("DOUDOU_RUNTIME_ACTIVE") == selected_runtime_id
    )
    pointer_active = bool(pointer and selected_python.exists())
    current_camoufox = package_version("camoufox")
    selected_camoufox = (
        environment_package_version(selected_python, "camoufox")
        if selected_python.exists()
        else None
    )
    camoufox_matches = selected_camoufox == CAMOUFOX_PACKAGE_VERSION
    browser = (
        browser_version(selected_python)
        if selected_python.exists()
        else {
            "available": False,
            "matches": False,
            "expected": CAMOUFOX_BROWSER_VERSION,
            "summary": "",
        }
    )
    python_supported = (3, 10) <= sys.version_info[:2] < (4, 0)
    issues: list[dict[str, Any]] = []
    if not python_supported:
        issues.append({"code": "PYTHON_VERSION_UNSUPPORTED", "expected": ">=3.10,<4.0"})
    if not selected_python.exists():
        issues.append({"code": "RUNTIME_SETUP_REQUIRED", "path": str(selected_dir)})
    if selected_python.exists() and not camoufox_matches:
        issues.append(
            {
                "code": "CAMOUFOX_PACKAGE_VERSION_MISMATCH",
                "expected": CAMOUFOX_PACKAGE_VERSION,
                "actual": selected_camoufox,
            }
        )
    if selected_python.exists() and not browser["matches"]:
        issues.append(
            {
                "code": "CAMOUFOX_BROWSER_VERSION_MISMATCH",
                "expected": CAMOUFOX_BROWSER_VERSION,
            }
        )
    writable = True
    try:
        data_root.mkdir(parents=True, exist_ok=True)
        probe = data_root / ".doudou-write-probe"
        probe.write_text("ok", encoding="ascii")
        probe.unlink()
    except OSError:
        writable = False
        issues.append({"code": "DATA_ROOT_NOT_WRITABLE", "path": str(data_root)})
    launch = {
        "requested": launch_probe,
        "passed": None,
        "mode": platform_background_mode(),
    }
    if (
        launch_probe
        and selected_python.exists()
        and camoufox_matches
        and browser["matches"]
    ):
        launch.update(launch_browser_probe(selected_python))
        if not launch["passed"]:
            issues.append(
                {"code": "CAMOUFOX_LAUNCH_FAILED", "mode": platform_background_mode()}
            )
    status = "ok" if not issues else "setup_required"
    return {
        "schema": "doudou.doctor.v2",
        "status": status,
        "python": {
            "executable": sys.executable,
            "version": platform.python_version(),
            "supported": python_supported,
        },
        "runtime": {
            "runtime_id": selected_runtime_id,
            "path": str(selected_dir),
            "python": str(selected_python),
            "exists": selected_python.exists(),
            "active": pointer_active,
            "process_active": process_active,
        },
        "camoufox": {
            "expected": CAMOUFOX_PACKAGE_VERSION,
            "actual": selected_camoufox,
            "host_actual": current_camoufox,
            "matches": camoufox_matches,
        },
        "browser": browser,
        "setup": setup_download_status(),
        "background_headed": launch,
        "filesystem": {
            "data_root": str(data_root),
            "writable": writable,
            "profiles": str(data_root / "profile"),
            "message": str(data_root / "message"),
        },
        "issues": issues,
        "repair_command": "doudou setup" if issues else None,
    }


def reexec_in_runtime(argv: list[str]) -> bool:
    try:
        pointer = json.loads(current_pointer().read_text(encoding="utf-8"))
    except (FileNotFoundError, json.JSONDecodeError):
        return False
    runtime_id = pointer.get("runtime_id")
    if runtime_id and os.environ.get("DOUDOU_RUNTIME_ACTIVE") == runtime_id:
        return True
    python = venv_python(Path(pointer["path"]))
    if not python.exists():
        return False
    environment = os.environ.copy()
    environment["DOUDOU_RUNTIME_ACTIVE"] = str(runtime_id)
    if os.name == "nt":
        startupinfo = subprocess.STARTUPINFO()
        startupinfo.dwFlags |= subprocess.STARTF_USESHOWWINDOW
        startupinfo.wShowWindow = 0
        process = subprocess.Popen(
            [str(python), *argv],
            env=environment,
            creationflags=subprocess.CREATE_NO_WINDOW,
            startupinfo=startupinfo,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )

        def relay(source: Any, target: Any) -> None:
            while chunk := source.read(4096):
                target.write(chunk)
                target.flush()

        stdout_target = getattr(sys.stdout, "buffer", sys.stdout)
        stderr_target = getattr(sys.stderr, "buffer", sys.stderr)
        threads = (
            threading.Thread(
                target=relay, args=(process.stdout, stdout_target), daemon=True
            ),
            threading.Thread(
                target=relay, args=(process.stderr, stderr_target), daemon=True
            ),
        )
        for thread in threads:
            thread.start()
        return_code = process.wait()
        for thread in threads:
            thread.join()
        raise SystemExit(return_code)
    os.execve(str(python), [str(python), *argv], environment)
    return True
