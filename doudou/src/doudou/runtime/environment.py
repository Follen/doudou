from __future__ import annotations

import importlib.metadata
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import threading
import venv
from pathlib import Path
from typing import Any

from ..common.core import atomic_write_json, utc_now
from .platform.windows import WindowVisibilityGuard
from .profile import (
    CAMOUFOX_BROWSER_VERSION,
    CAMOUFOX_PACKAGE_VERSION,
    platform_background_mode,
)

RUNTIME_ID = f"py{sys.version_info.major}{sys.version_info.minor}-camoufox-{CAMOUFOX_PACKAGE_VERSION}-browser-{CAMOUFOX_BROWSER_VERSION}"


def user_cache_root() -> Path:
    if os.name == "nt":
        return (
            Path(os.environ.get("LOCALAPPDATA", Path.home() / "AppData" / "Local"))
            / "doudou"
        )
    return Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "doudou"


def runtime_root() -> Path:
    return user_cache_root() / "runtimes"


def current_pointer() -> Path:
    return runtime_root() / "current.json"


def runtime_directory(runtime_id: str = RUNTIME_ID) -> Path:
    return runtime_root() / runtime_id


def venv_python(directory: Path) -> Path:
    return directory / ("Scripts/python.exe" if os.name == "nt" else "bin/python")


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
        "import asyncio,subprocess,sys\nfrom camoufox.async_api import AsyncCamoufox\n"
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
        },
    }


def browser_version(python: Path | None = None) -> dict[str, Any]:
    executable = str(python or Path(sys.executable))
    command = [executable, "-m", "camoufox", "version"]
    result = subprocess.run(
        command,
        capture_output=True,
        check=False,
        text=True,
        timeout=30,
        **hidden_process_options(),
    )
    output = (result.stdout + result.stderr).strip()
    return {
        "available": result.returncode == 0,
        "matches": result.returncode == 0 and f"v{CAMOUFOX_BROWSER_VERSION}" in output,
        "expected": CAMOUFOX_BROWSER_VERSION,
        "summary": output,
    }


def doctor(data_root: Path, launch_probe: bool = False) -> dict[str, Any]:
    pointer: dict[str, Any] | None = None
    try:
        pointer = json.loads(current_pointer().read_text(encoding="utf-8"))
    except (FileNotFoundError, json.JSONDecodeError):
        pass
    selected_dir = (
        Path(pointer["path"])
        if pointer and pointer.get("runtime_id") == RUNTIME_ID
        else runtime_directory()
    )
    selected_python = venv_python(selected_dir)
    current_environment = os.environ.get("DOUDOU_RUNTIME_ACTIVE") == RUNTIME_ID
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
            "runtime_id": RUNTIME_ID,
            "path": str(selected_dir),
            "python": str(selected_python),
            "exists": selected_python.exists(),
            "active": current_environment,
        },
        "camoufox": {
            "expected": CAMOUFOX_PACKAGE_VERSION,
            "actual": selected_camoufox,
            "host_actual": current_camoufox,
            "matches": camoufox_matches,
        },
        "browser": browser,
        "background_headed": launch,
        "filesystem": {
            "data_root": str(data_root),
            "writable": writable,
            "profiles": str(data_root / "profile"),
            "message": str(data_root / "message"),
        },
        "issues": issues,
        "repair_command": "python <skill>/bin/doudou.py setup" if issues else None,
    }


def setup(skill_root: Path) -> dict[str, Any]:
    root = runtime_root()
    root.mkdir(parents=True, exist_ok=True)
    final = runtime_directory()
    previous: dict[str, Any] | None = None
    try:
        previous = json.loads(current_pointer().read_text(encoding="utf-8"))
    except (FileNotFoundError, json.JSONDecodeError):
        pass
    temporary = Path(tempfile.mkdtemp(prefix=f".{RUNTIME_ID}-", dir=root))
    stages: dict[str, str] = {}
    rollback_id = f"setup-{utc_now().replace(':', '').replace('-', '')}"
    rollback_path = user_cache_root() / "rollbacks" / f"{rollback_id}.json"
    try:
        venv.EnvBuilder(with_pip=True, clear=True).create(temporary)
        stages["venv"] = "passed"
        python = venv_python(temporary)
        lock = skill_root / "requirements.lock"
        install = subprocess.run(
            [str(python), "-m", "pip", "install", "--require-hashes", "-r", str(lock)],
            capture_output=True,
            check=False,
            text=True,
            **hidden_process_options(),
        )
        if install.returncode:
            raise RuntimeError(f"dependency install failed: {install.stderr[-3000:]}")
        stages["dependencies"] = "passed"
        set_result = subprocess.run(
            [
                str(python),
                "-m",
                "camoufox",
                "set",
                f"official/stable/{CAMOUFOX_BROWSER_VERSION}",
            ],
            capture_output=True,
            check=False,
            text=True,
            **hidden_process_options(),
        )
        if set_result.returncode:
            raise RuntimeError(f"browser selection failed: {set_result.stderr[-3000:]}")
        fetch_result = subprocess.run(
            [str(python), "-m", "camoufox", "fetch"],
            capture_output=True,
            check=False,
            text=True,
            **hidden_process_options(),
        )
        if fetch_result.returncode:
            raise RuntimeError(f"browser fetch failed: {fetch_result.stderr[-3000:]}")
        stages["browser"] = "passed"
        version_check = browser_version(python)
        launch_check = launch_browser_probe(python)
        if (
            environment_package_version(python, "camoufox") != CAMOUFOX_PACKAGE_VERSION
            or not version_check["matches"]
            or not launch_check["passed"]
        ):
            raise RuntimeError(
                "new runtime failed the real package/browser launch verification"
            )
        stages["verification"] = "passed"
        manifest = {
            "schema": "doudou.runtime-install.v1",
            "runtime_id": RUNTIME_ID,
            "python": platform.python_version(),
            "camoufox": CAMOUFOX_PACKAGE_VERSION,
            "browser": CAMOUFOX_BROWSER_VERSION,
            "installed_at": utc_now(),
        }
        atomic_write_json(temporary / "runtime.json", manifest)
        replaced: str | None = None
        if final.exists():
            existing_python = venv_python(final)
            existing_valid = (
                existing_python.exists()
                and environment_package_version(existing_python, "camoufox")
                == CAMOUFOX_PACKAGE_VERSION
                and browser_version(existing_python)["matches"]
                and launch_browser_probe(existing_python)["passed"]
            )
            if existing_valid:
                shutil.rmtree(temporary)
                temporary = final
            else:
                replaced_path = final.with_name(f"{final.name}.replaced-{rollback_id}")
                os.replace(final, replaced_path)
                replaced = str(replaced_path)
                os.replace(temporary, final)
                temporary = final
        else:
            os.replace(temporary, final)
            temporary = final
        pointer = {
            "schema": "doudou.runtime-pointer.v1",
            "runtime_id": RUNTIME_ID,
            "path": str(final),
            "updated_at": utc_now(),
        }
        atomic_write_json(current_pointer(), pointer)
        stages["switch"] = "passed"
        rollback = {
            "schema": "doudou.runtime-rollback.v1",
            "rollback_id": rollback_id,
            "completed_at": utc_now(),
            "status": "switched",
            "previous": previous,
            "current": pointer,
            "replaced_runtime": replaced,
            "stages": stages,
        }
        atomic_write_json(rollback_path, rollback)
        return {
            "status": "ok",
            "runtime": pointer,
            "stages": stages,
            "previous": previous,
            "rollback_record": str(rollback_path),
        }
    except Exception as error:  # noqa: BLE001 - setup must always emit rollback evidence
        if temporary.exists() and temporary != final:
            shutil.rmtree(temporary, ignore_errors=True)
        rollback = {
            "schema": "doudou.runtime-rollback.v1",
            "rollback_id": rollback_id,
            "completed_at": utc_now(),
            "status": "failed_before_switch",
            "previous": previous,
            "stages": stages,
            "error_type": type(error).__name__,
        }
        atomic_write_json(rollback_path, rollback)
        return {
            "status": "failed",
            "error": str(error),
            "stages": stages,
            "previous": previous,
            "rollback_record": str(rollback_path),
        }


def reexec_in_runtime(argv: list[str]) -> bool:
    if os.environ.get("DOUDOU_RUNTIME_ACTIVE") == RUNTIME_ID:
        return True
    try:
        pointer = json.loads(current_pointer().read_text(encoding="utf-8"))
    except (FileNotFoundError, json.JSONDecodeError):
        return False
    if pointer.get("runtime_id") != RUNTIME_ID:
        return False
    python = venv_python(Path(pointer["path"]))
    if not python.exists():
        return False
    environment = os.environ.copy()
    environment["DOUDOU_RUNTIME_ACTIVE"] = RUNTIME_ID
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
