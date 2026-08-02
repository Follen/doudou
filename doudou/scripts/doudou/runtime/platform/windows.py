from __future__ import annotations

import asyncio
import os
import subprocess
from collections.abc import Iterator
from contextlib import contextmanager
from typing import Any

CREATE_NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0x08000000)


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
