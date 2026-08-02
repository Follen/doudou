from __future__ import annotations

import asyncio
import unittest
from unittest.mock import AsyncMock, MagicMock, patch

from doudou.runtime.browser.session import FIREFOX_PREFS
from doudou.runtime.platform import windows


class WindowsProcessTests(unittest.IsolatedAsyncioTestCase):
    def test_only_browser_processes_are_eligible_for_window_visibility(self) -> None:
        processes = {
            10: "node.exe",
            11: "conhost.exe",
            12: "camoufox.exe",
            13: "firefox.exe",
        }
        eligible = {
            pid
            for pid, name in processes.items()
            if name in windows.BROWSER_PROCESS_NAMES
        }
        self.assertEqual(eligible, {12, 13})

    def test_only_primary_browser_window_class_is_presented(self) -> None:
        self.assertEqual(windows.BROWSER_WINDOW_CLASS, "MozillaWindowClass")
        self.assertNotEqual(windows.BROWSER_WINDOW_CLASS, "MozillaBatteryClass")
        self.assertNotEqual(windows.BROWSER_WINDOW_CLASS, "MozillaHiddenWindowClass")

    def test_background_window_uses_zero_alpha_layer(self) -> None:
        self.assertEqual(windows.WS_EX_LAYERED, 0x00080000)
        self.assertEqual(windows.LWA_ALPHA, 0x00000002)

    def test_normal_window_without_layered_style_is_inherently_opaque(self) -> None:
        self.assertFalse(0x100 & windows.WS_EX_LAYERED)

    def test_background_window_is_removed_from_task_switching_styles(self) -> None:
        original = windows.WS_EX_APPWINDOW | 0x10
        background = windows.background_extended_style(original)
        self.assertTrue(background & windows.WS_EX_LAYERED)
        self.assertTrue(background & windows.WS_EX_TOOLWINDOW)
        self.assertFalse(background & windows.WS_EX_APPWINDOW)

    def test_firefox_startup_skeleton_window_is_disabled(self) -> None:
        self.assertIs(FIREFOX_PREFS["browser.startup.preXulSkeletonUI"], False)

    def test_window_guard_is_noop_off_windows(self) -> None:
        guard = windows.WindowVisibilityGuard(123)
        with patch.object(windows.os, "name", "posix"):
            guard.start()
            self.assertEqual(guard.set_visible(False), 0)
            guard.stop()
        self.assertIsNone(guard._thread)

    def test_window_guard_starts_hidden(self) -> None:
        guard = windows.WindowVisibilityGuard(123)
        self.assertFalse(guard.visible)
        self.assertEqual(guard.events_seen, 0)
        self.assertEqual(guard.windows_hidden_before_show, 0)

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
