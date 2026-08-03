from __future__ import annotations

import asyncio
import json
import os
import unittest
from unittest.mock import AsyncMock, MagicMock, patch

from doudou.runtime import process
from doudou.runtime.browser.session import FIREFOX_PREFS
from doudou.runtime.platform import windows
from doudou.runtime.profile import platform_background_mode


class ProcessIdentityTests(unittest.TestCase):
    def test_windows_process_identity_pins_utf8_and_handles_empty_output(self) -> None:
        result = MagicMock(returncode=0, stdout=None)
        with (
            patch.object(process.os, "name", "nt"),
            patch.object(process.subprocess, "CREATE_NO_WINDOW", 0, create=True),
            patch.object(process.subprocess, "run", return_value=result) as run,
        ):
            self.assertIsNone(process.process_identity(123))

        kwargs = run.call_args.kwargs
        self.assertEqual(kwargs["encoding"], "utf-8")
        self.assertEqual(kwargs["errors"], "replace")
        script = run.call_args.args[0][-1]
        self.assertIn("UTF8Encoding]::new($false)", script)

    def test_windows_process_identity_parses_utf8_json(self) -> None:
        result = MagicMock(
            returncode=0,
            stdout=json.dumps(
                {"pid": 123, "start": "2026-08-04T00:00:00.0000000Z", "command": "中文"},
                ensure_ascii=False,
            ),
        )
        with (
            patch.object(process.os, "name", "nt"),
            patch.object(process.subprocess, "CREATE_NO_WINDOW", 0, create=True),
            patch.object(process.subprocess, "run", return_value=result),
        ):
            self.assertEqual(process.process_identity(123)["command"], "中文")


@unittest.skipUnless(os.name == "nt", "Windows process integration tests")
class WindowsProcessTests(unittest.IsolatedAsyncioTestCase):
    def test_windows_background_mode_is_true_headless(self) -> None:
        self.assertEqual(platform_background_mode(), "windows_headless")

    def test_firefox_startup_skeleton_window_is_disabled(self) -> None:
        self.assertIs(FIREFOX_PREFS["browser.startup.preXulSkeletonUI"], False)

    async def test_asyncio_browser_driver_uses_no_console_flag(self) -> None:
        original = asyncio.create_subprocess_exec
        process = object()
        create = AsyncMock(return_value=process)
        startupinfo = MagicMock()
        startupinfo.dwFlags = 0

        with (
            patch.object(windows.os, "name", "nt"),
            patch.object(windows.subprocess, "STARTUPINFO", return_value=startupinfo),
            patch.object(windows.subprocess, "STARTF_USESHOWWINDOW", 1, create=True),
            patch.object(asyncio, "create_subprocess_exec", create),
        ):
            with windows.suppress_asyncio_console_windows():
                result = await asyncio.create_subprocess_exec("driver.exe")
            self.assertIs(result, process)
            kwargs = create.await_args.kwargs
            self.assertEqual(
                kwargs["creationflags"] & windows.CREATE_NO_WINDOW,
                windows.CREATE_NO_WINDOW,
            )
            self.assertEqual(kwargs["startupinfo"].wShowWindow, 0)

        self.assertIs(asyncio.create_subprocess_exec, original)


if __name__ == "__main__":
    unittest.main()
