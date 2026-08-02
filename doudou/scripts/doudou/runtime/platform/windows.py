from __future__ import annotations

import asyncio
import contextlib
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
import uuid
from collections.abc import Iterator
from contextlib import contextmanager
from typing import Any

CREATE_NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0x08000000)
BROWSER_PROCESS_NAMES = {"camoufox.exe", "firefox.exe"}
BROWSER_WINDOW_CLASS = "MozillaWindowClass"
WS_EX_LAYERED = 0x00080000
WS_EX_TOOLWINDOW = 0x00000080
WS_EX_APPWINDOW = 0x00040000
LWA_ALPHA = 0x00000002
_WINDOW_EX_STYLES: dict[int, int] = {}
_TASKBAR_STATE: dict[int, str] = {}


def background_extended_style(original_style: int) -> int:
    return (original_style | WS_EX_LAYERED | WS_EX_TOOLWINDOW) & ~WS_EX_APPWINDOW


class DescendantProcessJob:
    """Track future descendants without scanning the process table in window hooks."""

    def __init__(self, root_pid: int) -> None:
        self.root_pid = root_pid
        self.handle: Any = None
        self.name = f"Local\\DoudouBrowser-{root_pid}-{uuid.uuid4().hex}"

    def start(self) -> bool:
        if os.name != "nt" or self.handle is not None:
            return False
        import ctypes
        from ctypes import wintypes

        PROCESS_TERMINATE = 0x0001
        PROCESS_SET_QUOTA = 0x0100
        PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
        kernel32 = ctypes.windll.kernel32
        kernel32.CreateJobObjectW.restype = wintypes.HANDLE
        kernel32.OpenProcess.restype = wintypes.HANDLE
        job = kernel32.CreateJobObjectW(None, self.name)
        if not job:
            return False
        process = kernel32.OpenProcess(
            PROCESS_TERMINATE
            | PROCESS_SET_QUOTA
            | PROCESS_QUERY_LIMITED_INFORMATION,
            False,
            self.root_pid,
        )
        if not process:
            kernel32.CloseHandle(job)
            return False
        try:
            if not kernel32.AssignProcessToJobObject(job, process):
                kernel32.CloseHandle(job)
                return False
        finally:
            kernel32.CloseHandle(process)
        self.handle = wintypes.HANDLE(job)
        return True

    def contains(self, pid: int) -> bool | None:
        if os.name != "nt" or self.handle is None:
            return None
        import ctypes
        from ctypes import wintypes

        PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
        kernel32 = ctypes.windll.kernel32
        kernel32.OpenProcess.restype = wintypes.HANDLE
        process = kernel32.OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION, False, pid
        )
        if not process:
            return False
        try:
            member = wintypes.BOOL()
            if not kernel32.IsProcessInJob(
                process, self.handle, ctypes.byref(member)
            ):
                return None
            return bool(member.value)
        finally:
            kernel32.CloseHandle(process)

    def close(self) -> None:
        if os.name != "nt" or self.handle is None:
            return
        import ctypes

        ctypes.windll.kernel32.CloseHandle(self.handle)
        self.handle = None


class StartupWindowGuardProcess:
    """Run the time-critical startup hook outside the busy broker process."""

    def __init__(self, root_pid: int, job_name: str) -> None:
        token = f"{root_pid}-{uuid.uuid4().hex}"
        temporary = os.path.abspath(tempfile.gettempdir())
        self.ready_path = os.path.join(temporary, f"doudou-window-{token}.ready")
        self.stop_path = os.path.join(temporary, f"doudou-window-{token}.stop")
        self.result_path = os.path.join(temporary, f"doudou-window-{token}.json")
        self.root_pid = root_pid
        self.job_name = job_name
        self.process: subprocess.Popen[Any] | None = None
        self.ready = False
        self.events: list[dict[str, Any]] = []
        self.poll_events: list[dict[str, Any]] = []
        self.max_callback_latency_ms = 0

    def start(self) -> bool:
        if os.name != "nt" or self.process is not None:
            return False
        python = os.path.join(os.path.dirname(sys.executable), "pythonw.exe")
        if not os.path.exists(python):
            python = sys.executable
        startupinfo = subprocess.STARTUPINFO()
        startupinfo.dwFlags |= subprocess.STARTF_USESHOWWINDOW
        startupinfo.wShowWindow = 0
        environment = os.environ.copy()
        package_root = os.path.abspath(
            os.path.join(os.path.dirname(__file__), "..", "..", "..")
        )
        existing_path = environment.get("PYTHONPATH")
        environment["PYTHONPATH"] = (
            package_root
            if not existing_path
            else os.pathsep.join((package_root, existing_path))
        )
        self.process = subprocess.Popen(
            [
                python,
                "-m",
                "doudou.runtime.platform.windows_guard_worker",
                "--root-pid",
                str(self.root_pid),
                "--job-name",
                self.job_name,
                "--ready-path",
                self.ready_path,
                "--stop-path",
                self.stop_path,
                "--result-path",
                self.result_path,
            ],
            env=environment,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            creationflags=CREATE_NO_WINDOW,
            startupinfo=startupinfo,
        )
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            if os.path.exists(self.ready_path):
                self.ready = True
                return True
            if self.process.poll() is not None:
                break
            time.sleep(0.01)
        return False

    def stop(self) -> None:
        if self.process is None:
            return
        try:
            with open(self.stop_path, "w", encoding="ascii") as handle:
                handle.write("stop\n")
            self.process.wait(timeout=3)
        except (OSError, subprocess.TimeoutExpired):
            self.process.terminate()
            with contextlib.suppress(subprocess.TimeoutExpired):
                self.process.wait(timeout=2)
        try:
            with open(self.result_path, encoding="utf-8") as handle:
                result = json.load(handle)
            self.events = list(result.get("events") or [])
            self.poll_events = list(result.get("poll_events") or [])
            self.max_callback_latency_ms = int(
                result.get("max_callback_latency_ms") or 0
            )
        except (OSError, ValueError, TypeError):
            pass
        for path in (self.ready_path, self.stop_path, self.result_path):
            with contextlib.suppress(OSError):
                os.unlink(path)
        self.process = None


def _set_taskbar_tab(hwnd: int, visible: bool) -> bool:
    if os.name != "nt":
        return False
    import ctypes
    import uuid
    from ctypes import wintypes

    class GUID(ctypes.Structure):
        _fields_ = [
            ("Data1", wintypes.DWORD),
            ("Data2", wintypes.WORD),
            ("Data3", wintypes.WORD),
            ("Data4", ctypes.c_ubyte * 8),
        ]

        @classmethod
        def from_text(cls, value: str) -> GUID:
            raw = uuid.UUID(value).bytes_le
            return cls.from_buffer_copy(raw)

    ole32 = ctypes.windll.ole32
    pointer = ctypes.c_void_p()
    clsid = GUID.from_text("56FDF344-FD6D-11D0-958A-006097C9A090")
    iid = GUID.from_text("56FDF342-FD6D-11D0-958A-006097C9A090")
    initialized = ole32.CoInitializeEx(None, 0x2) in (0, 1)
    try:
        result = ole32.CoCreateInstance(
            ctypes.byref(clsid), None, 1, ctypes.byref(iid), ctypes.byref(pointer)
        )
        if result != 0 or not pointer.value:
            return False
        vtable = ctypes.cast(
            pointer, ctypes.POINTER(ctypes.POINTER(ctypes.c_void_p))
        ).contents
        call_type = ctypes.WINFUNCTYPE(ctypes.c_long, ctypes.c_void_p, wintypes.HWND)
        init_type = ctypes.WINFUNCTYPE(ctypes.c_long, ctypes.c_void_p)
        release_type = ctypes.WINFUNCTYPE(ctypes.c_ulong, ctypes.c_void_p)
        try:
            if init_type(vtable[3])(pointer) != 0:
                return False
            method = call_type(vtable[4 if visible else 5])
            success = method(pointer, hwnd) == 0
            if success:
                _TASKBAR_STATE[hwnd] = "registered" if visible else "removed"
            return success
        finally:
            release_type(vtable[2])(pointer)
    finally:
        if initialized:
            ole32.CoUninitialize()


class WindowVisibilityGuard:
    """Hide Firefox top-level windows before their first show event."""

    def __init__(self, root_pid: int, visible: bool = False) -> None:
        self.root_pid = root_pid
        self.visible = visible
        self._ready = threading.Event()
        self._thread: threading.Thread | None = None
        self._thread_id: int | None = None
        self._hook: Any = None
        self._callback: Any = None
        self._job = DescendantProcessJob(root_pid)
        self._startup_worker: StartupWindowGuardProcess | None = None
        self._prepared_windows: set[int] = set()
        self.events_seen = 0
        self.windows_hidden_before_show = 0
        self.create_events_seen = 0
        self.show_events_seen = 0
        self.show_events_prepared = 0
        self.show_events_unprepared = 0
        self.max_callback_latency_ms = 0
        self.job_filter_active = False
        self.window_events: list[dict[str, Any]] = []
        self.startup_worker_ready = False
        self.startup_worker_events: list[dict[str, Any]] = []
        self.startup_worker_poll_events: list[dict[str, Any]] = []
        self.startup_worker_max_callback_latency_ms = 0

    def start(self) -> None:
        if os.name != "nt" or self._thread is not None:
            return
        self.job_filter_active = self._job.start()
        if self.job_filter_active:
            self._startup_worker = StartupWindowGuardProcess(
                self.root_pid, self._job.name
            )
            self.startup_worker_ready = self._startup_worker.start()
        self._thread = threading.Thread(
            target=self._run, name="doudou-window-guard", daemon=True
        )
        self._thread.start()
        self._ready.wait(timeout=2)

    def set_visible(self, visible: bool) -> int:
        self.visible = visible
        return set_descendant_windows_visible(self.root_pid, visible)

    def stop(self) -> None:
        if os.name != "nt" or self._thread is None:
            return
        import ctypes

        if self._thread_id is not None:
            ctypes.windll.user32.PostThreadMessageW(self._thread_id, 0x0012, 0, 0)
        self._thread.join(timeout=2)
        self._thread = None
        self._thread_id = None
        self.finish_startup()
        self._job.close()

    def finish_startup(self) -> None:
        if self._startup_worker is None:
            return
        self._startup_worker.stop()
        self.startup_worker_events = list(self._startup_worker.events)
        self.startup_worker_poll_events = list(self._startup_worker.poll_events)
        self.startup_worker_max_callback_latency_ms = (
            self._startup_worker.max_callback_latency_ms
        )
        self._startup_worker = None

    def _owns_descendant_window(self, hwnd: int) -> bool:
        import ctypes
        from ctypes import wintypes

        user32 = ctypes.windll.user32
        pid = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        membership = self._job.contains(int(pid.value))
        if membership is not None:
            return membership
        return int(pid.value) in _descendant_pids(self.root_pid)

    def _record_window_event(self, hwnd: int, event: int, latency: int) -> None:
        if len(self.window_events) >= 64:
            return
        import ctypes

        user32 = ctypes.windll.user32
        class_name = ctypes.create_unicode_buffer(256)
        title = ctypes.create_unicode_buffer(512)
        user32.GetClassNameW(hwnd, class_name, len(class_name))
        user32.GetWindowTextW(hwnd, title, len(title))
        style = int(user32.GetWindowLongW(hwnd, -20))
        title_bytes = title.value.encode("utf-8", errors="replace")
        self.window_events.append(
            {
                "event": "create" if event == 0x8000 else "show",
                "hwnd": hwnd,
                "class_name": class_name.value,
                "visible_before": bool(user32.IsWindowVisible(hwnd)),
                "extended_style_before": style,
                "tool_window_before": bool(style & WS_EX_TOOLWINDOW),
                "app_window_before": bool(style & WS_EX_APPWINDOW),
                "title_length": len(title.value),
                "title_sha256": hashlib.sha256(title_bytes).hexdigest(),
                "callback_latency_ms": latency,
            }
        )

    def _run(self) -> None:
        import ctypes
        from ctypes import wintypes

        EVENT_OBJECT_CREATE = 0x8000
        EVENT_OBJECT_SHOW = 0x8002
        WINEVENT_OUTOFCONTEXT = 0x0000
        WINEVENT_SKIPOWNPROCESS = 0x0002
        OBJID_WINDOW = 0
        CHILDID_SELF = 0
        user32 = ctypes.windll.user32
        kernel32 = ctypes.windll.kernel32
        self._thread_id = int(kernel32.GetCurrentThreadId())
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

        @callback_type
        def on_window_event(_hook, event, hwnd, object_id, child_id, _thread_id, _time):
            if not hwnd or object_id != OBJID_WINDOW or child_id != CHILDID_SELF:
                return
            hwnd_value = int(hwnd)
            if not self._owns_descendant_window(hwnd_value):
                return
            self.events_seen += 1
            latency = (int(kernel32.GetTickCount()) - int(_time)) & 0xFFFFFFFF
            self.max_callback_latency_ms = max(self.max_callback_latency_ms, latency)
            self._record_window_event(hwnd_value, int(event), latency)
            if event == EVENT_OBJECT_CREATE:
                self.create_events_seen += 1
                self._prepared_windows.add(hwnd_value)
            elif event == EVENT_OBJECT_SHOW:
                self.show_events_seen += 1
                if hwnd_value in self._prepared_windows:
                    self.show_events_prepared += 1
                else:
                    self.show_events_unprepared += 1
            if not self.visible:
                _set_browser_window_visible(
                    hwnd_value, False, show_background=False
                )
                if event == EVENT_OBJECT_CREATE:
                    self.windows_hidden_before_show += 1

        self._callback = on_window_event
        self._hook = user32.SetWinEventHook(
            EVENT_OBJECT_CREATE,
            EVENT_OBJECT_SHOW,
            None,
            self._callback,
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
        self._ready.set()
        if not self._hook:
            return
        message = wintypes.MSG()
        while user32.GetMessageW(ctypes.byref(message), None, 0, 0) > 0:
            user32.TranslateMessage(ctypes.byref(message))
            user32.DispatchMessageW(ctypes.byref(message))
        user32.UnhookWinEvent(self._hook)
        self._hook = None
        self._callback = None


@contextmanager
def suppress_asyncio_console_windows() -> Iterator[None]:
    """Prevent console-mode browser drivers from creating a terminal window."""
    if os.name != "nt":
        yield
        return

    original = asyncio.create_subprocess_exec

    async def create_hidden_subprocess(*args: Any, **kwargs: Any) -> Any:
        kwargs["creationflags"] = int(kwargs.get("creationflags", 0)) | CREATE_NO_WINDOW
        startupinfo = kwargs.get("startupinfo") or subprocess.STARTUPINFO()
        startupinfo.dwFlags |= subprocess.STARTF_USESHOWWINDOW
        startupinfo.wShowWindow = 0
        kwargs["startupinfo"] = startupinfo
        return await original(*args, **kwargs)

    asyncio.create_subprocess_exec = create_hidden_subprocess
    try:
        yield
    finally:
        asyncio.create_subprocess_exec = original


def _descendant_processes(root_pid: int) -> dict[int, str]:
    if os.name != "nt":
        return {}
    import ctypes
    from ctypes import wintypes

    TH32CS_SNAPPROCESS = 0x00000002

    class PROCESSENTRY32W(ctypes.Structure):
        _fields_ = [
            ("dwSize", wintypes.DWORD),
            ("cntUsage", wintypes.DWORD),
            ("th32ProcessID", wintypes.DWORD),
            ("th32DefaultHeapID", ctypes.POINTER(ctypes.c_ulong)),
            ("th32ModuleID", wintypes.DWORD),
            ("cntThreads", wintypes.DWORD),
            ("th32ParentProcessID", wintypes.DWORD),
            ("pcPriClassBase", ctypes.c_long),
            ("dwFlags", wintypes.DWORD),
            ("szExeFile", wintypes.WCHAR * 260),
        ]

    kernel32 = ctypes.windll.kernel32
    snapshot = kernel32.CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
    if snapshot == wintypes.HANDLE(-1).value:
        return set()
    parents: dict[int, int] = {}
    names: dict[int, str] = {}
    entry = PROCESSENTRY32W()
    entry.dwSize = ctypes.sizeof(entry)
    try:
        success = kernel32.Process32FirstW(snapshot, ctypes.byref(entry))
        while success:
            parents[int(entry.th32ProcessID)] = int(entry.th32ParentProcessID)
            names[int(entry.th32ProcessID)] = str(entry.szExeFile).lower()
            success = kernel32.Process32NextW(snapshot, ctypes.byref(entry))
    finally:
        kernel32.CloseHandle(snapshot)
    descendants = {root_pid}
    changed = True
    while changed:
        changed = False
        for pid, parent in parents.items():
            if parent in descendants and pid not in descendants:
                descendants.add(pid)
                changed = True
    descendants.discard(root_pid)
    return {pid: names.get(pid, "") for pid in descendants}


def _descendant_pids(root_pid: int) -> set[int]:
    return set(_descendant_processes(root_pid))


def _is_browser_window(hwnd: int, root_pid: int) -> bool:
    if os.name != "nt":
        return False
    import ctypes
    from ctypes import wintypes

    user32 = ctypes.windll.user32
    class_name = ctypes.create_unicode_buffer(256)
    user32.GetClassNameW(hwnd, class_name, len(class_name))
    if class_name.value != BROWSER_WINDOW_CLASS:
        return False
    pid = wintypes.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    return _descendant_processes(root_pid).get(int(pid.value)) in BROWSER_PROCESS_NAMES


def _set_browser_window_visible(
    hwnd: int, visible: bool, *, show_background: bool = True
) -> None:
    import ctypes

    user32 = ctypes.windll.user32
    SW_RESTORE = 9
    SW_SHOWNOACTIVATE = 4
    GWL_EXSTYLE = -20
    SWP_NOSIZE = 0x0001
    SWP_NOMOVE = 0x0002
    SWP_NOZORDER = 0x0004
    SWP_NOACTIVATE = 0x0010
    SWP_FRAMECHANGED = 0x0020
    original_style = _WINDOW_EX_STYLES.setdefault(
        hwnd, int(user32.GetWindowLongW(hwnd, GWL_EXSTYLE))
    )
    if visible:
        user32.SetWindowLongW(hwnd, GWL_EXSTYLE, original_style)
        user32.SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )
        user32.SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA)
        user32.ShowWindow(hwnd, SW_RESTORE)
        _set_taskbar_tab(hwnd, True)
    else:
        user32.SetWindowLongW(
            hwnd, GWL_EXSTYLE, background_extended_style(original_style)
        )
        user32.SetLayeredWindowAttributes(hwnd, 0, 0, LWA_ALPHA)
        user32.SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )
        _set_taskbar_tab(hwnd, False)
        if show_background:
            user32.ShowWindow(hwnd, SW_SHOWNOACTIVATE)
            _set_taskbar_tab(hwnd, False)


def descendant_window_states(root_pid: int) -> list[dict[str, Any]]:
    if os.name != "nt":
        return []
    import ctypes
    from ctypes import wintypes

    processes = _descendant_processes(root_pid)
    browser_pids = {
        pid for pid, name in processes.items() if name in BROWSER_PROCESS_NAMES
    }
    user32 = ctypes.windll.user32
    states: list[dict[str, Any]] = []

    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def callback(hwnd, _):
        pid = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        if int(pid.value) not in browser_pids:
            return True
        class_name = ctypes.create_unicode_buffer(256)
        user32.GetClassNameW(hwnd, class_name, len(class_name))
        if class_name.value != BROWSER_WINDOW_CLASS:
            return True
        style = int(user32.GetWindowLongW(hwnd, -20))
        alpha_value = 255
        if style & WS_EX_LAYERED:
            alpha = ctypes.c_ubyte()
            flags = wintypes.DWORD()
            if user32.GetLayeredWindowAttributes(
                hwnd, None, ctypes.byref(alpha), ctypes.byref(flags)
            ):
                alpha_value = int(alpha.value)
        visible = bool(user32.IsWindowVisible(hwnd))
        tool_window = bool(style & WS_EX_TOOLWINDOW)
        app_window = bool(style & WS_EX_APPWINDOW)
        states.append(
            {
                "hwnd": int(hwnd),
                "class_name": class_name.value,
                "visible": visible,
                "alpha": alpha_value,
                "extended_style": style,
                "tool_window": tool_window,
                "app_window": app_window,
                "alt_tab_candidate": visible and not tool_window,
                "taskbar_registration": _TASKBAR_STATE.get(int(hwnd), "unknown"),
            }
        )
        return True

    user32.EnumWindows(callback, 0)
    return states


def set_descendant_windows_visible(root_pid: int, visible: bool) -> int:
    if os.name != "nt":
        return 0
    import ctypes
    from ctypes import wintypes

    processes = _descendant_processes(root_pid)
    browser_pids = {
        pid for pid, name in processes.items() if name in BROWSER_PROCESS_NAMES
    }
    if not browser_pids:
        return 0
    user32 = ctypes.windll.user32
    changed = 0

    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def callback(hwnd, _):
        nonlocal changed
        pid = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        if int(pid.value) not in browser_pids:
            return True
        class_name = ctypes.create_unicode_buffer(256)
        user32.GetClassNameW(hwnd, class_name, len(class_name))
        if class_name.value != BROWSER_WINDOW_CLASS:
            return True
        _set_browser_window_visible(int(hwnd), visible)
        changed += 1
        return True

    user32.EnumWindows(callback, 0)
    return changed


def settle_window_visibility(
    root_pid: int, visible: bool, attempts: int = 20, delay: float = 0.25
) -> int:
    total = 0
    for _ in range(attempts):
        total += set_descendant_windows_visible(root_pid, visible)
        time.sleep(delay)
    return total
