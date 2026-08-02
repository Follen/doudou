from __future__ import annotations

import json
import os
from pathlib import Path
from unittest.mock import patch

from doudou.runtime.camoufox_asset import browser_status
from doudou.runtime.environment import browser_version, doctor, venv_python
from doudou.runtime.home import browser_root, doudou_home


def _browser_fixture(root: Path) -> Path:
    active = root / "browsers" / "official" / "152.0.4-beta.28-fixture"
    active.mkdir(parents=True)
    executable = active / ("camoufox.exe" if os.name == "nt" else "camoufox-bin")
    executable.write_bytes(b"fixture")
    (active / "version.json").write_text(
        json.dumps(
            {
                "version": "152.0.4",
                "build": "beta.28",
                "sha256": "fixture-sha",
            }
        ),
        encoding="utf-8",
    )
    (root / "config.json").write_text(
        json.dumps(
            {
                "active_version": "browsers/official/152.0.4-beta.28-fixture"
            }
        ),
        encoding="utf-8",
    )
    return active


def test_doudou_home_controls_runtime_and_browser_roots(tmp_path: Path) -> None:
    configured = tmp_path / "managed"
    with patch.dict(os.environ, {"DOUDOU_HOME": str(configured)}):
        assert doudou_home() == configured
        assert browser_root() == configured / "browsers"


def test_doctor_reads_only_the_dedicated_browser_metadata(tmp_path: Path) -> None:
    root = tmp_path / "browsers"
    active = _browser_fixture(root)
    direct = browser_status(root, "152.0.4-beta.28")
    assert direct["matches"] is True
    assert direct["path"] == str(active)
    with patch.dict(os.environ, {"DOUDOU_HOME": str(tmp_path)}):
        status = browser_version()
    assert status["matches"] is True
    assert status["path"] == str(active)
    assert Path(str(status["path"])).is_relative_to(root)


def test_doctor_marks_the_current_pointer_active_outside_reexec(tmp_path: Path) -> None:
    home = tmp_path / "managed"
    runtime = home / "runtimes" / "runtime-a"
    python = venv_python(runtime)
    python.parent.mkdir(parents=True)
    python.write_bytes(b"fixture")
    (home / "runtimes" / "current.json").write_text(
        json.dumps(
            {
                "schema": "doudou.runtime-pointer.v2",
                "runtime_id": "runtime-a",
                "path": str(runtime),
            }
        ),
        encoding="utf-8",
    )
    with (
        patch.dict(os.environ, {"DOUDOU_HOME": str(home)}, clear=False),
        patch("doudou.runtime.environment.environment_package_version", return_value="0.5.4"),
        patch(
            "doudou.runtime.environment.browser_version",
            return_value={"available": True, "matches": True},
        ),
    ):
        result = doctor(tmp_path)
    assert result["runtime"]["active"] is True
    assert result["runtime"]["process_active"] is False
