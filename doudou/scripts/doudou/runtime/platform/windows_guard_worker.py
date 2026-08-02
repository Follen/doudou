from __future__ import annotations

import argparse
import ctypes
import hashlib
import json
import os
import threading
import time
from ctypes import wintypes
from pathlib import Path

from .windows import (
    BROWSER_WINDOW_CLASS,
    WS_EX_APPWINDOW,
    WS_EX_TOOLWINDOW,
    _set_browser_window_visible,
)


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root-pid", type=int, required=True)
    parser.add_argument("--job-name", required=True)
    parser.add_argument("--ready-path", type=Path, required=True)
    parser.add_argument("--stop-path", type=Path, required=True)
    parser.add_argument("--result-path", type=Path, required=True)
    return parser


def run(args: argparse.Namespace) -> int:
    if os.name != "nt":
        return 0
    EVENT_OBJECT_CREATE = 0x8000
    EVENT_OBJECT_SHOW = 0x8002
    WINEVENT_OUTOFCONTEXT = 0x0000
    OBJID_WINDOW = 0
    CHILDID_SELF = 0
    PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
    JOB_OBJECT_QUERY = 0x0004
    WM_QUIT = 0x0012

    user32 = ctypes.windll.user32
    kernel32 = ctypes.windll.kernel32
    kernel32.OpenJobObjectW.restype = wintypes.HANDLE
    kernel32.OpenProcess.restype = wintypes.HANDLE
    job = kernel32.OpenJobObjectW(JOB_OBJECT_QUERY, False, args.job_name)
    if not job:
        return 2

    events: list[dict[str, object]] = []
    poll_events: list[dict[str, object]] = []
    max_latency = 0
    event_lock = threading.Lock()
    stop_event = threading.Event()
    callback_type = ctypes.WINFUNCTYPE(
        None,
        wintypes.HANDLE,
        wintypes.DWORD,
        wintypes.HWND,
        wintypes.LONG,
        wintypes.LONG,
        wintypes.DWORD,
        wintypes.DWORD,
    )

    def belongs_to_job(hwnd: int) -> bool:
        pid = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        process = kernel32.OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION, False, int(pid.value)
        )
        if not process:
            return False
        try:
            member = wintypes.BOOL()
            if not kernel32.IsProcessInJob(process, job, ctypes.byref(member)):
                return False
            if not member.value or int(pid.value) == args.root_pid:
                return False
        finally:
            kernel32.CloseHandle(process)
        return True

    def window_metadata(hwnd: int, event: str, latency: int) -> dict[str, object]:
        class_name = ctypes.create_unicode_buffer(256)
        title = ctypes.create_unicode_buffer(512)
        user32.GetClassNameW(hwnd, class_name, len(class_name))
        user32.GetWindowTextW(hwnd, title, len(title))
        style = int(user32.GetWindowLongW(hwnd, -20))
        return {
            "event": event,
            "hwnd": hwnd,
            "class_name": class_name.value,
            "visible_before": bool(user32.IsWindowVisible(hwnd)),
            "extended_style_before": style,
            "tool_window_before": bool(style & WS_EX_TOOLWINDOW),
            "app_window_before": bool(style & WS_EX_APPWINDOW),
            "title_length": len(title.value),
            "title_sha256": hashlib.sha256(
                title.value.encode("utf-8", errors="replace")
            ).hexdigest(),
            "callback_latency_ms": latency,
        }

    def is_primary_window(hwnd: int) -> bool:
        class_name = ctypes.create_unicode_buffer(256)
        user32.GetClassNameW(hwnd, class_name, len(class_name))
        return class_name.value == BROWSER_WINDOW_CLASS

    @callback_type
    def on_window_event(_hook, event, hwnd, object_id, child_id, _thread_id, stamp):
        nonlocal max_latency
        if not hwnd or object_id != OBJID_WINDOW or child_id != CHILDID_SELF:
            return
        hwnd_value = int(hwnd)
        if not is_primary_window(hwnd_value) or not belongs_to_job(hwnd_value):
            return

        latency = (int(kernel32.GetTickCount()) - int(stamp)) & 0xFFFFFFFF
        max_latency = max(max_latency, latency)
        with event_lock:
            if len(events) < 64:
                events.append(
                    window_metadata(
                        hwnd_value,
                        "create" if event == EVENT_OBJECT_CREATE else "show",
                        latency,
                    )
                )
        _set_browser_window_visible(hwnd_value, False, show_background=False)

    def poll_windows() -> None:
        scanned: set[tuple[int, int]] = set()

        @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
        def visit(hwnd, _parameter):
            hwnd_value = int(hwnd)
            pid = wintypes.DWORD()
            user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
            identity = (hwnd_value, int(pid.value))
            if identity in scanned:
                return True
            scanned.add(identity)
            if not is_primary_window(hwnd_value) or not belongs_to_job(hwnd_value):
                return True
            with event_lock:
                if len(poll_events) < 64:
                    poll_events.append(window_metadata(hwnd_value, "poll", 0))
            _set_browser_window_visible(
                hwnd_value, False, show_background=False
            )
            return True

        while not stop_event.is_set():
            user32.EnumWindows(visit, 0)
            stop_event.wait(0.001)

    hook = user32.SetWinEventHook(
        EVENT_OBJECT_CREATE,
        EVENT_OBJECT_SHOW,
        None,
        on_window_event,
        0,
        0,
        WINEVENT_OUTOFCONTEXT,
    )
    if not hook:
        kernel32.CloseHandle(job)
        return 3

    thread_id = int(kernel32.GetCurrentThreadId())

    def stop_watch() -> None:
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline and not args.stop_path.exists():
            time.sleep(0.01)
        stop_event.set()
        user32.PostThreadMessageW(thread_id, WM_QUIT, 0, 0)

    polling_thread = threading.Thread(target=poll_windows, daemon=True)
    polling_thread.start()
    threading.Thread(target=stop_watch, daemon=True).start()
    args.ready_path.write_text("ready\n", encoding="ascii")
    message = wintypes.MSG()
    while user32.GetMessageW(ctypes.byref(message), None, 0, 0) > 0:
        user32.TranslateMessage(ctypes.byref(message))
        user32.DispatchMessageW(ctypes.byref(message))
    stop_event.set()
    polling_thread.join(timeout=1)
    user32.UnhookWinEvent(hook)
    kernel32.CloseHandle(job)
    args.result_path.write_text(
        json.dumps(
            {
                "events": events,
                "poll_events": poll_events,
                "max_callback_latency_ms": max_latency,
            },
            ensure_ascii=True,
            separators=(",", ":"),
        ),
        encoding="utf-8",
    )
    return 0


def main() -> int:
    return run(_parser().parse_args())


if __name__ == "__main__":
    raise SystemExit(main())
