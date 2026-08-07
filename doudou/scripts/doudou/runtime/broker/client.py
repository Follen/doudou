from __future__ import annotations

import asyncio
import json
import os
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

from ...common.core import random_id, read_json
from ...common.errors import DoudouError
from ..process import process_identity, same_process
from ..profile import load_profile

DEFAULT_COMMAND_TIMEOUT_SECONDS = 90.0
BROKER_RESPONSE_LIMIT_BYTES = 16 * 1024 * 1024
LONG_RUNNING_COMMANDS = {"login.wait", "protocol.capture"}
COLLECTION_COMMAND_TIMEOUTS = {
    "videos.collect": 600.0,
    "video.collect": 900.0,
}


def command_timeout_seconds(command: str, params: dict[str, Any]) -> float:
    if command == "page.doctor":
        return 180.0
    if command in COLLECTION_COMMAND_TIMEOUTS:
        return COLLECTION_COMMAND_TIMEOUTS[command]
    if command in LONG_RUNNING_COMMANDS:
        return min(max(float(params.get("timeout_seconds") or 600) + 30, 30), 900)
    return DEFAULT_COMMAND_TIMEOUT_SECONDS


def descriptor_is_live(
    descriptor: dict[str, Any] | None, data_root: Path, profile_id: str
) -> bool:
    if (
        not isinstance(descriptor, dict)
        or descriptor.get("schema") != "doudou.broker-descriptor.v1"
    ):
        return False
    if (
        Path(str(descriptor.get("data_root", ""))).resolve() != data_root.resolve()
        or descriptor.get("profile_id") != profile_id
    ):
        return False
    expected = {"pid": descriptor.get("pid"), "start": descriptor.get("process_start")}
    return same_process(expected, process_identity(int(descriptor.get("pid", 0))))


class BrokerClient:
    def __init__(
        self,
        data_root: Path,
        profile_name: str,
        start_timeout: float = 90.0,
        fake_browser: bool = False,
    ) -> None:
        self.data_root = data_root.resolve()
        self.profile_name = profile_name
        _, self.paths = load_profile(self.data_root, profile_name)
        self.start_timeout = start_timeout
        self.fake_browser = fake_browser
        self.last_process_identity: dict[str, Any] | None = None
        self.launcher_process_identity: dict[str, Any] | None = None

    def _descriptor(self) -> dict[str, Any] | None:
        descriptor = read_json(self.paths.broker_descriptor, optional=True)
        if descriptor_is_live(descriptor, self.data_root, self.paths.profile_id):
            if not self.paths.broker_token.is_file():
                return None
            self.last_process_identity = {
                "pid": descriptor.get("pid"),
                "start": descriptor.get("process_start"),
            }
            return descriptor
        if descriptor:
            self.paths.broker_descriptor.unlink(missing_ok=True)
            self.paths.broker_token.unlink(missing_ok=True)
        return None

    def _token(self, descriptor: dict[str, Any]) -> str:
        deadline = time.monotonic() + 1.0
        while time.monotonic() < deadline:
            try:
                return self.paths.broker_token.read_text(encoding="utf-8").strip()
            except FileNotFoundError:
                current = read_json(self.paths.broker_descriptor, optional=True)
                if not descriptor_is_live(
                    current, self.data_root, self.paths.profile_id
                ):
                    break
                time.sleep(0.02)
        raise FileNotFoundError(self.paths.broker_token)

    def ensure_started(self) -> dict[str, Any]:
        descriptor = self._descriptor()
        if descriptor:
            return descriptor
        current = read_json(self.paths.broker_descriptor, optional=True)
        if descriptor_is_live(current, self.data_root, self.paths.profile_id):
            deadline = time.monotonic() + 1.0
            while time.monotonic() < deadline:
                descriptor = self._descriptor()
                if descriptor:
                    return descriptor
                current = read_json(self.paths.broker_descriptor, optional=True)
                if not descriptor_is_live(
                    current, self.data_root, self.paths.profile_id
                ):
                    break
                time.sleep(0.02)
        self.paths.runtime.mkdir(parents=True, exist_ok=True)
        stdout = self.paths.broker_stdout.open("ab")
        stderr = self.paths.broker_stderr.open("ab")
        broker_python = Path(sys.executable)
        if os.name == "nt":
            pythonw = broker_python.with_name("pythonw.exe")
            if pythonw.exists():
                broker_python = pythonw
        command = [
            str(broker_python),
            "-m",
            "doudou.features.broker",
            "--data-root",
            str(self.data_root),
            "--profile",
            self.profile_name,
            "--idle-timeout",
            os.environ.get("DOUDOU_BROKER_IDLE_SECONDS", "1200"),
        ]
        if self.fake_browser:
            command.append("--fake-browser")
        environment = os.environ.copy()
        source_root = str(Path(__file__).resolve().parents[3])
        environment["PYTHONPATH"] = os.pathsep.join(
            filter(None, [source_root, environment.get("PYTHONPATH")])
        )
        creationflags = 0
        startupinfo = None
        if os.name == "nt":
            creationflags = subprocess.CREATE_NO_WINDOW
            startupinfo = subprocess.STARTUPINFO()
            startupinfo.dwFlags |= subprocess.STARTF_USESHOWWINDOW
            startupinfo.wShowWindow = 0
        process = subprocess.Popen(
            command,
            cwd=str(self.data_root),
            stdin=subprocess.DEVNULL,
            stdout=stdout,
            stderr=stderr,
            close_fds=True,
            creationflags=creationflags,
            startupinfo=startupinfo,
            env=environment,
        )
        self.launcher_process_identity = process_identity(process.pid)
        if os.name == "nt":
            process._handle.Close()  # type: ignore[attr-defined]
            process.returncode = 0
        stdout.close()
        stderr.close()
        deadline = time.monotonic() + self.start_timeout
        while time.monotonic() < deadline:
            descriptor = self._descriptor()
            if descriptor:
                return descriptor
            time.sleep(0.1)
        tail = (
            self.paths.broker_stderr.read_text(encoding="utf-8", errors="replace")[
                -4000:
            ]
            if self.paths.broker_stderr.exists()
            else ""
        )
        raise DoudouError(
            "BROKER_START_TIMEOUT",
            "The Profile broker did not become ready before the timeout.",
            {"stderr_tail": tail},
        )

    async def request_async(
        self,
        command: str,
        params: dict[str, Any] | None = None,
        semantics: str = "read",
        request_id: str | None = None,
        start: bool = True,
    ) -> dict[str, Any]:
        descriptor = self.ensure_started() if start else self._descriptor()
        if descriptor is None:
            raise DoudouError(
                "BROKER_NOT_RUNNING", "The Profile broker is not running."
            )
        endpoint = descriptor["endpoint"]
        token = self._token(descriptor)
        payload = {
            "token": token,
            "request_id": request_id or random_id("request"),
            "command": command,
            "semantics": semantics,
            "params": params or {},
        }
        payload["command_timeout_seconds"] = command_timeout_seconds(
            command, payload["params"]
        )
        reader, writer = await asyncio.open_connection(
            endpoint["host"],
            int(endpoint["port"]),
            limit=BROKER_RESPONSE_LIMIT_BYTES,
        )
        writer.write(
            json.dumps(payload, ensure_ascii=False, separators=(",", ":")).encode(
                "utf-8"
            )
            + b"\n"
        )
        await writer.drain()
        try:
            try:
                line = await asyncio.wait_for(
                    reader.readuntil(b"\n"),
                    timeout=float(payload["command_timeout_seconds"]) + 5,
                )
            except asyncio.IncompleteReadError as error:
                line = error.partial
            except asyncio.LimitOverrunError as error:
                details = {
                    "request_id": payload["request_id"],
                    "response_limit_bytes": BROKER_RESPONSE_LIMIT_BYTES,
                    "write_started": True,
                }
                if semantics == "write":
                    raise DoudouError(
                        "WRITE_RESULT_UNKNOWN",
                        "The broker response exceeded the client limit after the write started; it was not replayed.",
                        details,
                    ) from error
                details.pop("write_started")
                raise DoudouError(
                    "BROKER_RESPONSE_TOO_LARGE",
                    "The broker response exceeded the client limit.",
                    details,
                ) from error
        except TimeoutError as error:
            if semantics == "write":
                raise DoudouError(
                    "WRITE_RESULT_UNKNOWN",
                    "The broker response timed out after a write may have started; it was not replayed.",
                    {
                        "request_id": payload["request_id"],
                        "timeout_seconds": payload["command_timeout_seconds"],
                        "write_started": True,
                    },
                ) from error
            raise DoudouError(
                "BROKER_COMMAND_TIMEOUT",
                "The broker response exceeded its total time limit.",
                {
                    "request_id": payload["request_id"],
                    "timeout_seconds": payload["command_timeout_seconds"],
                },
            ) from error
        finally:
            writer.close()
            await writer.wait_closed()
        if not line:
            if semantics == "write":
                raise DoudouError(
                    "WRITE_RESULT_UNKNOWN",
                    "The broker disconnected after the write started; the request was not replayed.",
                    {"request_id": payload["request_id"]},
                )
            raise DoudouError(
                "BROKER_DISCONNECTED",
                "The broker disconnected before returning a response.",
            )
        response = json.loads(line)
        if not response.get("ok"):
            error = response.get("error") or {}
            raise DoudouError(
                error.get("code", "BROKER_REQUEST_FAILED"),
                error.get("message", "Broker request failed."),
                error.get("details"),
            )
        return response

    def request(
        self,
        command: str,
        params: dict[str, Any] | None = None,
        semantics: str = "read",
        request_id: str | None = None,
        start: bool = True,
    ) -> dict[str, Any]:
        try:
            return asyncio.run(
                self.request_async(
                    command,
                    params=params,
                    semantics=semantics,
                    request_id=request_id,
                    start=start,
                )
            )
        except (ConnectionError, OSError) as error:
            if semantics == "write":
                raise DoudouError(
                    "WRITE_RESULT_UNKNOWN",
                    "The broker connection failed after a write may have started; the request was not replayed.",
                    {"request_id": request_id},
                ) from error
            if start:
                self.paths.broker_descriptor.unlink(missing_ok=True)
                return asyncio.run(
                    self.request_async(
                        command,
                        params=params,
                        semantics=semantics,
                        request_id=request_id,
                        start=True,
                    )
                )
            raise

    def wait_stopped(self, timeout: float = 10.0) -> bool:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            process_stopped = True
            if self.last_process_identity:
                pid = int(self.last_process_identity.get("pid") or 0)
                process_stopped = not same_process(
                    self.last_process_identity, process_identity(pid)
                )
            launcher_stopped = True
            if self.launcher_process_identity:
                pid = int(self.launcher_process_identity.get("pid") or 0)
                launcher_stopped = not same_process(
                    self.launcher_process_identity, process_identity(pid)
                )
            if (
                not self.paths.broker_descriptor.exists()
                and not self.paths.lock.exists()
                and process_stopped
                and launcher_stopped
            ):
                return True
            time.sleep(0.05)
        return False
