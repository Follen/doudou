from __future__ import annotations

import argparse
import asyncio
import contextlib
import json
import os
import secrets
import signal
from collections.abc import Awaitable, Callable
from pathlib import Path
from typing import Any

from ...common.core import atomic_write_json, atomic_write_text, read_json, utc_now
from ...common.errors import DoudouError
from ..browser.session import BrowserSession
from ..lock import ProfileLock
from ..process import process_identity
from ..profile import load_profile
from .registration import register_broker, unregister_broker

MAX_REQUEST_BYTES = 4 * 1024 * 1024
DETERMINISTIC_WRITE_ERRORS = {
    "PAGE_ADAPTER_DRIFTED",
    "PROTOCOL_SCHEMA_DRIFTED",
    "PLAN_DRIFTED",
    "PLAN_EXPIRED",
    "PLAN_NOT_FOUND",
    "IMAGE_NOT_FOUND",
    "COMPOSER_PLAN_MISMATCH",
    "CONVERSATION_UI_AMBIGUOUS",
    "REPLY_TARGET_NOT_VISIBLE",
    "MENTION_TARGET_NOT_VISIBLE",
    "MESSAGE_TARGET_NOT_VISIBLE",
    "COMMENT_TARGET_NOT_VISIBLE",
}


class BrokerServer:
    def __init__(
        self,
        data_root: Path,
        profile_name: str,
        idle_timeout: float = 1200.0,
        fake_browser: bool = False,
        handler: Callable[
            [str, dict[str, Any], BrowserSession, Path, str],
            Awaitable[dict[str, Any] | None],
        ]
        | None = None,
    ) -> None:
        self.data_root = data_root.resolve()
        self.profile_name = profile_name
        _, self.paths = load_profile(self.data_root, profile_name)
        self.idle_timeout = idle_timeout
        self.fake_browser = fake_browser
        self.token = secrets.token_urlsafe(48)
        self.browser = BrowserSession(self.paths, visible=False, fake=fake_browser)
        self.lock = ProfileLock(self.paths.lock, self.paths.profile_id, self.data_root)
        self.server: asyncio.AbstractServer | None = None
        self.command_lock = asyncio.Lock()
        self.queue_count = 0
        self.active_request: str | None = None
        self.last_completed_monotonic = asyncio.get_running_loop().time()
        self.last_completed_at = utc_now()
        self.stop_event = asyncio.Event()
        self.started_at = utc_now()
        self.request_results: dict[str, dict[str, Any]] = {}
        self.handler = handler
        self.registration: Path | None = None

    async def start(self) -> None:
        self.lock.acquire()
        try:
            await self.browser.start()
            self.server = await asyncio.start_server(
                self._handle_client, "127.0.0.1", 0
            )
            host, port = self.server.sockets[0].getsockname()[:2]
            identity = process_identity(os.getpid()) or {
                "pid": os.getpid(),
                "start": None,
            }
            self.paths.runtime.mkdir(parents=True, exist_ok=True)
            atomic_write_text(self.paths.broker_token, self.token + "\n", mode=0o600)
            descriptor = {
                "schema": "doudou.broker-descriptor.v1",
                "pid": os.getpid(),
                "process_start": identity.get("start"),
                "data_root": str(self.data_root),
                "profile_id": self.paths.profile_id,
                "endpoint": {"transport": "tcp", "host": host, "port": port},
                "token_file": self.paths.broker_token.name,
                "mode": self.browser.mode,
                "browser_version": (await self.browser.status())["browser_version"],
                "created_at": self.started_at,
                "last_activity_at": self.last_completed_at,
            }
            atomic_write_json(self.paths.broker_descriptor, descriptor, mode=0o600)
            self.registration = register_broker(self.paths, descriptor)
        except BaseException:
            unregister_broker(self.registration)
            await self.browser.close()
            self.lock.release()
            raise

    async def run(self) -> None:
        await self.start()
        idle_task = asyncio.create_task(self._idle_watch())
        try:
            await self.stop_event.wait()
        finally:
            idle_task.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await idle_task
            await self.close()

    async def _idle_watch(self) -> None:
        interval = min(max(self.idle_timeout / 10, 0.05), 5.0)
        while not self.stop_event.is_set():
            await asyncio.sleep(interval)
            idle_for = asyncio.get_running_loop().time() - self.last_completed_monotonic
            if (
                not self.command_lock.locked()
                and self.queue_count == 0
                and idle_for >= self.idle_timeout
            ):
                self.stop_event.set()

    async def _handle_client(
        self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter
    ) -> None:
        try:
            line = await reader.readline()
            if not line or len(line) > MAX_REQUEST_BYTES:
                raise DoudouError(
                    "BROKER_REQUEST_INVALID",
                    "The broker request is empty or too large.",
                )
            request = json.loads(line)
            if not secrets.compare_digest(str(request.get("token", "")), self.token):
                raise DoudouError("BROKER_AUTH_FAILED", "Broker authentication failed.")
            response = await self.execute(request)
        except DoudouError as error:
            response = {"ok": False, "error": error.as_event()}
        except (json.JSONDecodeError, UnicodeDecodeError):
            response = {
                "ok": False,
                "error": DoudouError(
                    "BROKER_REQUEST_INVALID", "The broker request is not valid JSON."
                ).as_event(),
            }
        except Exception as error:  # noqa: BLE001 - broker protocol boundary
            response = {
                "ok": False,
                "error": DoudouError("BROKER_INTERNAL_ERROR", str(error)).as_event(),
            }
        writer.write(
            json.dumps(response, ensure_ascii=False, separators=(",", ":")).encode(
                "utf-8"
            )
            + b"\n"
        )
        await writer.drain()
        writer.close()
        with contextlib.suppress(Exception):
            await writer.wait_closed()

    async def execute(self, request: dict[str, Any]) -> dict[str, Any]:
        request_id = str(request.get("request_id") or "")
        command = str(request.get("command") or "")
        semantics = str(request.get("semantics") or "read")
        if (
            not request_id
            or not command
            or semantics not in {"read", "write", "control"}
        ):
            raise DoudouError(
                "BROKER_REQUEST_INVALID",
                "request_id, command and valid semantics are required.",
            )
        if request_id in self.request_results:
            return self.request_results[request_id]
        self.queue_count += 1
        try:
            async with self.command_lock:
                self.queue_count -= 1
                self.active_request = request_id
                started = utc_now()
                try:
                    result = await self._dispatch_with_read_recovery(
                        command, request.get("params") or {}, semantics
                    )
                    response = {
                        "ok": True,
                        "request_id": request_id,
                        "semantics": semantics,
                        "started_at": started,
                        "completed_at": utc_now(),
                        "result": result,
                    }
                except asyncio.CancelledError:
                    if semantics != "write" or self.stop_event.is_set():
                        raise
                    response = {
                        "ok": False,
                        "request_id": request_id,
                        "error": DoudouError(
                            "WRITE_RESULT_UNKNOWN",
                            "The write task was cancelled after it may have started; it was not replayed.",
                            {"request_id": request_id, "write_started": True},
                        ).as_event(),
                    }
                    self.request_results[request_id] = response
                    return response
                except Exception as error:
                    if isinstance(error, DoudouError) and error.code in {
                        "LOGIN_REQUIRED",
                        "VERIFICATION_REQUIRED",
                        "WRITE_VERIFICATION_REQUIRED",
                    }:
                        details = dict(error.details or {})
                        kind = await self.browser._verification_kind()
                        if error.code == "LOGIN_REQUIRED" or kind is not None:
                            details[
                                "takeover"
                            ] = await self.browser.present_for_takeover(
                                error.code, kind
                            )
                        else:
                            details["takeover"] = {
                                "active": False,
                                "reason": error.code,
                                "verification_kind": None,
                            }
                        response = {
                            "ok": False,
                            "request_id": request_id,
                            "error": DoudouError(
                                error.code, error.message, details
                            ).as_event(),
                        }
                        if semantics == "write":
                            self.request_results[request_id] = response
                        return response
                    details = (
                        error.details
                        if isinstance(error, DoudouError)
                        and isinstance(error.details, dict)
                        else {}
                    )
                    if isinstance(error, DoudouError) and (
                        error.code in DETERMINISTIC_WRITE_ERRORS
                        or not details.get("write_started")
                    ):
                        response = {
                            "ok": False,
                            "request_id": request_id,
                            "error": error.as_event(),
                        }
                        self.request_results[request_id] = response
                        return response
                    if semantics == "write":
                        response = {
                            "ok": False,
                            "request_id": request_id,
                            "error": DoudouError(
                                "WRITE_RESULT_UNKNOWN",
                                "The write started but a complete receipt was not obtained; it will not be replayed automatically.",
                                {
                                    "request_id": request_id,
                                    "cause_type": type(error).__name__,
                                    "cause_code": error.code
                                    if isinstance(error, DoudouError)
                                    else None,
                                    "cause_details": error.details
                                    if isinstance(error, DoudouError)
                                    else None,
                                },
                            ).as_event(),
                        }
                        self.request_results[request_id] = response
                        return response
                    raise
                finally:
                    self.active_request = None
                    self.last_completed_monotonic = asyncio.get_running_loop().time()
                    self.last_completed_at = utc_now()
                    self._refresh_descriptor()
                self.request_results[request_id] = response
                if len(self.request_results) > 256:
                    self.request_results.pop(next(iter(self.request_results)))
                return response
        except asyncio.CancelledError:
            if self.queue_count:
                self.queue_count -= 1
            raise

    async def _dispatch_with_read_recovery(
        self, command: str, params: dict[str, Any], semantics: str
    ) -> dict[str, Any]:
        try:
            return await self._dispatch(command, params)
        except DoudouError as error:
            if semantics != "read" or error.code not in {
                "LOGIN_REQUIRED",
                "VERIFICATION_REQUIRED",
            }:
                raise
            kind = await self.browser._verification_kind()
            if error.code == "VERIFICATION_REQUIRED" and kind is None:
                raise
            await self.browser.present_for_takeover(error.code, kind)
            await self.browser.wait_for_takeover_completion(600)
            return await self._dispatch(command, params)

    async def _dispatch(self, command: str, params: dict[str, Any]) -> dict[str, Any]:
        if command == "ping":
            return {"pong": True, "pid": os.getpid()}
        if command == "status":
            return {
                "pid": os.getpid(),
                "profile_id": self.paths.profile_id,
                "data_root": str(self.data_root),
                "queue_count": self.queue_count,
                "active_request": self.active_request,
                "idle_timeout_seconds": self.idle_timeout,
                "last_completed_at": self.last_completed_at,
                "browser": await self.browser.status(),
            }
        if command == "browser.visible":
            return await self.browser.set_visible(bool(params.get("visible", True)))
        if command == "page.open":
            url = str(params.get("url") or "")
            if not url.startswith(
                ("https://www.douyin.com/", "https://www.douyin.com")
            ):
                raise DoudouError(
                    "BROKER_URL_NOT_ALLOWED",
                    "The broker only opens supported Douyin pages.",
                )
            return await self.browser.open(url)
        if command == "login.wait":
            return await self.browser.wait_for_login(
                float(params.get("timeout_seconds") or 600)
            )
        if self.handler:
            handled = await self.handler(
                command, params, self.browser, self.data_root, self.profile_name
            )
            if handled is not None:
                return handled
        if command == "session.status":
            return {"status": "ok", "browser": await self.browser.status()}
        if command == "session.refresh":
            return await self.browser.ensure_ready(force_refresh=True)
        if command == "shutdown":
            self.stop_event.set()
            return {"stopping": True}
        raise DoudouError(
            "BROKER_COMMAND_UNKNOWN", f"Unknown broker command: {command}."
        )

    def _refresh_descriptor(self) -> None:
        descriptor = read_json(self.paths.broker_descriptor, optional=True)
        if isinstance(descriptor, dict) and descriptor.get("pid") == os.getpid():
            descriptor["last_activity_at"] = self.last_completed_at
            atomic_write_json(self.paths.broker_descriptor, descriptor, mode=0o600)

    async def close(self) -> None:
        if self.server:
            self.server.close()
            await self.server.wait_closed()
        await self.browser.close()
        self.paths.broker_descriptor.unlink(missing_ok=True)
        self.paths.broker_token.unlink(missing_ok=True)
        unregister_broker(self.registration)
        self.registration = None
        self.lock.release()


async def serve(
    data_root: Path,
    profile: str,
    idle_timeout: float,
    fake_browser: bool,
    handler: Callable[
        [str, dict[str, Any], BrowserSession, Path, str],
        Awaitable[dict[str, Any] | None],
    ]
    | None = None,
) -> None:
    broker = BrokerServer(
        data_root,
        profile,
        idle_timeout=idle_timeout,
        fake_browser=fake_browser,
        handler=handler,
    )
    loop = asyncio.get_running_loop()
    for name in ("SIGINT", "SIGTERM"):
        if hasattr(signal, name):
            with contextlib.suppress(NotImplementedError):
                loop.add_signal_handler(getattr(signal, name), broker.stop_event.set)
    await broker.run()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--data-root", required=True, type=Path)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--idle-timeout", type=float, default=1200.0)
    parser.add_argument("--fake-browser", action="store_true")
    args = parser.parse_args(argv)
    asyncio.run(
        serve(args.data_root, args.profile, args.idle_timeout, args.fake_browser)
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
