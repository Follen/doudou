from __future__ import annotations

import asyncio
import os
import unittest
from unittest.mock import AsyncMock, MagicMock, patch

from doudou.runtime.browser.session import FIREFOX_PREFS
from doudou.runtime.platform import windows
from doudou.runtime.profile import platform_background_mode


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
