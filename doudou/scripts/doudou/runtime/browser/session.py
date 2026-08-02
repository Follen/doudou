from __future__ import annotations

import asyncio
import base64
import contextlib
import os
from typing import Any

from ...common.core import atomic_write_json, read_json, utc_now
from ...common.errors import DoudouError
from ..paths import ProfilePaths
from ..platform.windows import (
    WindowVisibilityGuard,
    descendant_window_states,
    set_descendant_windows_visible,
    suppress_asyncio_console_windows,
)
from ..profile import (
    CAMOUFOX_BROWSER_VERSION,
    CAMOUFOX_PACKAGE_VERSION,
    platform_background_mode,
)

FIREFOX_PREFS: dict[str, Any] = {
    "browser.sessionstore.resume_from_crash": False,
    "browser.sessionstore.max_resumed_crashes": 0,
    "browser.sessionstore.max_tabs_undo": 0,
    "browser.sessionstore.max_windows_undo": 0,
    "browser.startup.page": 0,
    "browser.startup.preXulSkeletonUI": False,
    "media.volume_scale": "0.0",
    "media.autoplay.default": 5,
    "media.autoplay.blocking_policy": 2,
    "media.preload.default": 0,
    "media.preload.auto": 0,
}


class BrowserSession:
    def __init__(
        self, paths: ProfilePaths, visible: bool = False, fake: bool = False
    ) -> None:
        self.paths = paths
        self.visible = visible
        self.fake = fake
        self.mode = "fake" if fake else platform_background_mode()
        self.manager: Any = None
        self.context: Any = None
        self.page: Any = None
        self.started_at: str | None = None
        self.blocked_media_requests = 0
        self.ready = False
        self.ready_at: str | None = None
        self.initialization_count = 0
        self.init_request: bytes | None = None
        self.init_response: bytes | None = None
        self.init_endpoint: str | None = None
        self._init_request_future: asyncio.Future[bytes] | None = None
        self._init_response_future: asyncio.Future[bytes] | None = None
        self._network_tasks: set[asyncio.Task[Any]] = set()
        self.takeover_active = False
        self.takeover_reason: str | None = None
        self.takeover_kind: str | None = None
        self.takeover_started_at: str | None = None
        self.takeover_sms_requested = False
        self._visibility_task: asyncio.Task[None] | None = None
        self._window_guard: WindowVisibilityGuard | None = None

    def _record_login_state(self, login: dict[str, Any]) -> None:
        metadata = read_json(self.paths.metadata, optional=True)
        if not isinstance(metadata, dict):
            return
        status = str(login.get("status") or "verification_required")
        metadata["login_status"] = status
        metadata["last_error_code"] = login.get("reason_code")
        if status == "valid":
            metadata["last_login_verified_at"] = utc_now()
        metadata["updated_at"] = utc_now()
        atomic_write_json(self.paths.metadata, metadata)

    def _fingerprint_config(self) -> dict[str, Any]:
        record = read_json(self.paths.camoufox_config)
        generated = record.get("generated_fingerprint")
        if not isinstance(generated, dict):
            try:
                from camoufox.fingerprints import generate_context_fingerprint
            except ImportError as error:
                raise DoudouError(
                    "CAMOUFOX_MISSING",
                    "Camoufox is not installed in the active doudou environment.",
                ) from error
            generated_record = generate_context_fingerprint(
                os=record.get("target_os"),
                timezone=record.get("timezone"),
                locale=record.get("locale"),
                config_overrides=record.get("config") or {},
            )
            generated = generated_record["config"]
            record["generated_fingerprint"] = generated
            record["updated_at"] = utc_now()
            atomic_write_json(self.paths.camoufox_config, record, mode=0o600)
        return generated

    async def start(self) -> None:
        if self.fake:
            self.context = _FakeContext()
            self.page = self.context.pages[0]
            self.started_at = utc_now()
            return
        try:
            from camoufox.async_api import AsyncCamoufox
        except ImportError as error:
            raise DoudouError(
                "CAMOUFOX_MISSING",
                "Camoufox is not installed in the active doudou environment.",
            ) from error
        config_record = read_json(self.paths.camoufox_config)
        config = self._fingerprint_config()
        if os.name == "nt":
            headless: bool | str = False
        elif self.mode == "linux_virtual_headed":
            headless = "virtual"
        else:
            headless = False
        if os.name == "nt" and not self.visible:
            self._window_guard = WindowVisibilityGuard(os.getpid(), visible=False)
            self._window_guard.start()
            self._visibility_task = asyncio.create_task(
                self._maintain_window_visibility()
            )
        self.manager = AsyncCamoufox(
            headless=headless,
            persistent_context=True,
            user_data_dir=str(self.paths.browser_data),
            browser=CAMOUFOX_BROWSER_VERSION,
            os=config_record.get("target_os"),
            locale=config_record.get("locale"),
            config=config,
            firefox_user_prefs=FIREFOX_PREFS,
            enable_cache=True,
        )
        with suppress_asyncio_console_windows():
            self.context = await self.manager.__aenter__()
        await self._adopt_single_business_page()

        async def route_media(route) -> None:
            if route.request.resource_type == "media":
                self.blocked_media_requests += 1
                await route.abort()
            else:
                await route.continue_()

        await self.context.route("**/*", route_media)
        self._attach_im_capture()
        self.started_at = utc_now()

    async def _maintain_window_visibility(self) -> None:
        startup_deadline = asyncio.get_running_loop().time() + 5
        while True:
            if not self.visible:
                await asyncio.to_thread(
                    set_descendant_windows_visible, os.getpid(), False
                )
            delay = (
                0.01 if asyncio.get_running_loop().time() < startup_deadline else 0.25
            )
            await asyncio.sleep(delay)

    async def _adopt_single_business_page(self) -> None:
        pages = [page for page in self.context.pages if not page.is_closed()]
        self.page = pages[0] if pages else await self.context.new_page()
        for extra in pages[1:]:
            with contextlib.suppress(Exception):
                await extra.close()

    async def set_visible(self, visible: bool) -> dict[str, Any]:
        self.visible = visible
        changed = 0
        if os.name == "nt" and not self.fake:
            if self._window_guard is not None:
                changed = await asyncio.to_thread(
                    self._window_guard.set_visible, visible
                )
            else:
                changed = await asyncio.to_thread(
                    set_descendant_windows_visible, os.getpid(), visible
                )
        windows = (
            await asyncio.to_thread(descendant_window_states, os.getpid())
            if os.name == "nt" and not self.fake
            else []
        )
        return {
            "visible": visible,
            "windows_changed": changed,
            "mode": self.mode,
            "windows": windows,
        }

    async def present_for_takeover(
        self, reason: str, kind: str | None
    ) -> dict[str, Any]:
        self.takeover_active = True
        self.takeover_reason = reason
        self.takeover_kind = kind
        self.takeover_started_at = self.takeover_started_at or utc_now()
        presentation = await self.set_visible(True)
        return {
            "active": True,
            "reason": reason,
            "verification_kind": kind,
            "started_at": self.takeover_started_at,
            "presentation": presentation,
            "resume_command": f"profile login --profile {self.paths.profile.name}",
        }

    async def finish_takeover(self) -> None:
        self.takeover_active = False
        self.takeover_reason = None
        self.takeover_kind = None
        self.takeover_started_at = None
        self.takeover_sms_requested = False
        await self.set_visible(False)

    async def request_sms_code_if_available(self) -> bool:
        if self.fake or not self.page or self.takeover_sms_requested:
            return False
        from ...ui.adapters import AUTH_ADAPTER

        for key in ("get_code", "send_code"):
            controls = AUTH_ADAPTER.text(self.page, key, exact=False)
            for index in range(min(await controls.count(), 5)):
                control = controls.nth(index)
                with contextlib.suppress(Exception):
                    if await control.is_visible() and await control.is_enabled():
                        await control.click(timeout=10_000)
                        self.takeover_sms_requested = True
                        return True
        return False

    async def wait_for_takeover_completion(
        self, timeout_seconds: float
    ) -> dict[str, Any]:
        deadline = asyncio.get_running_loop().time() + timeout_seconds
        observed_challenge = False
        clear_since: float | None = None
        while asyncio.get_running_loop().time() < deadline:
            kind = await self._verification_kind()
            if kind is not None:
                observed_challenge = True
                clear_since = None
                if kind in {"sms_code", "quick_login_sms"}:
                    await self.request_sms_code_if_available()
            elif observed_challenge:
                clear_since = clear_since or asyncio.get_running_loop().time()
                if asyncio.get_running_loop().time() - clear_since >= 2:
                    try:
                        result = await self.ensure_ready(force_refresh=True)
                    except DoudouError as error:
                        if error.code not in {
                            "LOGIN_REQUIRED",
                            "VERIFICATION_REQUIRED",
                        }:
                            raise
                    else:
                        await self.finish_takeover()
                        return result
            await self.page.wait_for_timeout(250)
        raise DoudouError(
            "VERIFICATION_TIMEOUT",
            "The verification window remained incomplete before the timeout.",
            {
                "verification_kind": await self._verification_kind(),
                "takeover_active": self.takeover_active,
            },
        )

    async def open(
        self, url: str, wait_until: str = "domcontentloaded"
    ) -> dict[str, Any]:
        if not self.page:
            raise DoudouError(
                "BROKER_BROWSER_UNAVAILABLE", "The broker browser is not running."
            )
        await self.page.goto(url, wait_until=wait_until, timeout=60_000)
        return {"url": self.page.url, "title": await self.page.title()}

    def _attach_im_capture(self) -> None:
        if self.fake or not self.page:
            return
        loop = asyncio.get_running_loop()
        self._init_request_future = loop.create_future()
        self._init_response_future = loop.create_future()

        def on_request(request: Any) -> None:
            if "/v1/message/get_message_by_init" not in request.url:
                return
            value = request.post_data_buffer
            if value:
                self.init_request = bytes(value)
                self.init_endpoint = request.url.split("?", 1)[0]
                if self._init_request_future and not self._init_request_future.done():
                    self._init_request_future.set_result(self.init_request)

        async def consume_response(response: Any) -> None:
            if (
                "/v1/message/get_message_by_init" not in response.url
                or response.status != 200
            ):
                return
            try:
                value = await response.body()
            except Exception:  # noqa: BLE001 - detached Playwright response callback
                return
            self.init_response = bytes(value)
            if self._init_response_future and not self._init_response_future.done():
                self._init_response_future.set_result(self.init_response)

        def on_response(response: Any) -> None:
            task = asyncio.create_task(consume_response(response))
            self._network_tasks.add(task)
            task.add_done_callback(self._network_tasks.discard)

        self.page.on("request", on_request)
        self.page.on("response", on_response)

    async def _verification_kind(self) -> str | None:
        if self.fake or not self.page:
            return None
        from ...ui.adapters import AUTH_ADAPTER

        for kind in ("sms_code", "slider", "qr_login", "generic_verification"):
            locator = AUTH_ADAPTER.locator(self.page, kind)
            for index in range(min(await locator.count(), 5)):
                with contextlib.suppress(Exception):
                    candidate = locator.nth(index)
                    if await candidate.is_visible():
                        if kind == "generic_verification":
                            tag_name = await candidate.evaluate(
                                "element => element.tagName"
                            )
                            text = (await candidate.inner_text(timeout=1000)).casefold()
                            if tag_name != "IFRAME" and not any(
                                marker in text
                                for marker in ("验证", "验证码", "安全", "滑块", "拖动")
                            ):
                                continue
                        if kind == "sms_code":
                            quick = AUTH_ADAPTER.text(
                                self.page, "quick_login", exact=False
                            )
                            for item in range(min(await quick.count(), 5)):
                                if await quick.nth(item).is_visible():
                                    return "quick_login_sms"
                        return kind
        return None

    async def _verification_visible(self) -> bool:
        return await self._verification_kind() is not None

    async def classify_login(self) -> dict[str, Any]:
        if self.fake:
            return {
                "status": "valid",
                "valid": True,
                "reason_code": None,
                "authenticated_cookie_count": 1,
                "im_initialized": True,
                "verification_visible": False,
                "verification_kind": None,
            }
        cookies = await self.context.cookies(
            ["https://www.douyin.com/", "https://imapi.douyin.com/"]
        )
        authenticated = [
            item
            for item in cookies
            if item.get("name")
            in {"sessionid", "sessionid_ss", "sid_guard", "uid_tt", "uid_tt_ss"}
            and item.get("value")
        ]
        verification_kind = await self._verification_kind()
        verification_visible = verification_kind is not None
        im_initialized = bool(
            self.init_request
            and self.init_response
            and not self.init_response.lstrip().startswith((b"{", b"["))
        )
        if not authenticated:
            status, code = "login_required", "LOGIN_REQUIRED"
        elif verification_visible or not im_initialized:
            status, code = "verification_required", "VERIFICATION_REQUIRED"
        else:
            status, code = "valid", None
        return {
            "status": status,
            "valid": status == "valid",
            "reason_code": code,
            "authenticated_cookie_count": len(authenticated),
            "im_initialized": im_initialized,
            "verification_visible": verification_visible,
            "verification_kind": verification_kind,
        }

    async def wait_for_login(self, timeout_seconds: float) -> dict[str, Any]:
        if self.fake:
            return await self.ensure_ready(force_refresh=True)
        deadline = asyncio.get_running_loop().time() + timeout_seconds
        await self.present_for_takeover("LOGIN_REQUIRED", "qr_login")
        if not str(self.page.url).startswith("https://www.douyin.com"):
            await self.open("https://www.douyin.com/")
        while asyncio.get_running_loop().time() < deadline:
            cookies = await self.context.cookies(["https://www.douyin.com/"])
            authenticated = any(
                item.get("name")
                in {"sessionid", "sessionid_ss", "sid_guard", "uid_tt", "uid_tt_ss"}
                and item.get("value")
                for item in cookies
            )
            if authenticated and not await self._verification_visible():
                try:
                    result = await self.ensure_ready(force_refresh=True)
                except DoudouError as error:
                    if error.code != "VERIFICATION_REQUIRED":
                        raise
                else:
                    await self.finish_takeover()
                    return result
            await self.page.wait_for_timeout(1000)
        login = await self.classify_login()
        code = (
            "VERIFICATION_REQUIRED"
            if login.get("authenticated_cookie_count")
            else "LOGIN_TIMEOUT"
        )
        kind = login.get("verification_kind") or (
            "qr_login" if code == "LOGIN_TIMEOUT" else "generic_verification"
        )
        login["takeover"] = await self.present_for_takeover(code, kind)
        raise DoudouError(
            code,
            "Headed login or verification did not complete before the timeout.",
            login,
        )

    async def ensure_ready(self, force_refresh: bool = False) -> dict[str, Any]:
        if not self.page:
            raise DoudouError(
                "BROKER_BROWSER_UNAVAILABLE", "The broker browser is not running."
            )
        page_closed = (
            bool(self.page.is_closed()) if hasattr(self.page, "is_closed") else False
        )
        if (
            self.ready
            and not force_refresh
            and not page_closed
            and str(self.page.url).startswith("https://www.douyin.com/")
        ):
            login = await self.classify_login()
            self._record_login_state(login)
            if not login["valid"]:
                raise DoudouError(
                    login["reason_code"],
                    "The Profile requires headed login or verification.",
                    login,
                )
            return {
                "status": "ready",
                "source": "existing_session",
                "initialized_at": self.ready_at,
                "initialization_count": self.initialization_count,
                "page_url": self.page.url,
                "login": login,
            }
        if page_closed:
            await self._restart_after_user_close()
        if self.fake:
            await self.open("https://www.douyin.com/chat")
        else:
            self.init_request = None
            self.init_response = None
            self._attach_im_capture()
            await self.open("https://www.douyin.com/")
            await self.page.wait_for_timeout(1000)
            await self.open("https://www.douyin.com/chat")
            try:
                await asyncio.gather(
                    asyncio.wait_for(
                        asyncio.shield(self._init_request_future), timeout=30
                    ),
                    asyncio.wait_for(
                        asyncio.shield(self._init_response_future), timeout=30
                    ),
                )
            except TimeoutError:
                pass
        login = await self.classify_login()
        self._record_login_state(login)
        if not login["valid"]:
            raise DoudouError(
                login["reason_code"],
                "The Profile requires headed login or verification.",
                login,
            )
        if self.takeover_active:
            await self.finish_takeover()
        self.ready = True
        self.ready_at = utc_now()
        self.initialization_count += 1
        return {
            "status": "ready",
            "source": "forced_refresh" if force_refresh else "initialized",
            "initialized_at": self.ready_at,
            "initialization_count": self.initialization_count,
            "takeover": {
                "active": self.takeover_active,
                "reason": self.takeover_reason,
                "verification_kind": self.takeover_kind,
                "started_at": self.takeover_started_at,
            },
            "page_url": self.page.url,
            "login": login,
        }

    async def _restart_after_user_close(self) -> None:
        """Replace a context whose last native browser window was closed by the user."""
        with contextlib.suppress(Exception):
            await self.close()
        self.manager = None
        self.context = None
        self.page = None
        self.ready = False
        self.ready_at = None
        await self.start()

    async def post_protobuf(self, endpoint: str, body: bytes) -> tuple[int, bytes]:
        if not self.page or not endpoint.startswith("https://imapi.douyin.com/"):
            raise DoudouError(
                "PROTOCOL_ENDPOINT_INVALID",
                "Only the supported IM protocol endpoint is allowed.",
            )
        result = await self.page.evaluate(
            """async ({endpoint, bodyBase64}) => {
              const bytes = Uint8Array.from(atob(bodyBase64), char => char.charCodeAt(0));
              const response = await fetch(endpoint, {
                method: 'POST', credentials: 'include',
                headers: {'content-type': 'application/x-protobuf'}, body: bytes,
              });
              const output = new Uint8Array(await response.arrayBuffer());
              let binary = ''; for (const byte of output) binary += String.fromCharCode(byte);
              return {status: response.status, bodyBase64: btoa(binary)};
            }""",
            {
                "endpoint": endpoint,
                "bodyBase64": base64.b64encode(body).decode("ascii"),
            },
        )
        return int(result["status"]), base64.b64decode(result["bodyBase64"])

    async def fetch_text(self, url: str, timeout_ms: int = 15_000) -> dict[str, Any]:
        if not self.page or not url.startswith("https://www.douyin.com/"):
            raise DoudouError(
                "PROTOCOL_ENDPOINT_INVALID",
                "Only supported page-context protocol endpoints are allowed.",
            )
        return await self.page.evaluate(
            """async ({url, timeoutMs}) => {
              const controller = new AbortController();
              const timer = setTimeout(() => controller.abort(), timeoutMs);
              try {
                const response = await fetch(url, {credentials: 'include', signal: controller.signal});
                return {
                  http_status: response.status,
                  content_type: response.headers.get('content-type'),
                  retry_after: response.headers.get('retry-after'),
                  text: await response.text(),
                };
              } finally {
                clearTimeout(timer);
              }
            }""",
            {"url": url, "timeoutMs": timeout_ms},
        )

    async def status(self) -> dict[str, Any]:
        verification_kind = await self._verification_kind()
        return {
            "running": self.context is not None,
            "mode": self.mode,
            "visible": self.visible,
            "started_at": self.started_at,
            "page_url": self.page.url if self.page else None,
            "page_count": len(self.context.pages) if self.context else 0,
            "blocked_media_requests": self.blocked_media_requests,
            "ready": self.ready,
            "ready_at": self.ready_at,
            "initialization_count": self.initialization_count,
            "verification_visible": verification_kind is not None,
            "verification_kind": verification_kind,
            "takeover": {
                "active": self.takeover_active,
                "reason": self.takeover_reason,
                "verification_kind": self.takeover_kind,
                "started_at": self.takeover_started_at,
            },
            "init_request_bytes": len(self.init_request or b""),
            "init_response_bytes": len(self.init_response or b""),
            "camoufox_package_version": CAMOUFOX_PACKAGE_VERSION,
            "browser_version": CAMOUFOX_BROWSER_VERSION,
            "window_guard": {
                "active": self._window_guard is not None,
                "events_seen": self._window_guard.events_seen
                if self._window_guard
                else 0,
                "hidden_before_show": self._window_guard.windows_hidden_before_show
                if self._window_guard
                else 0,
                "windows": await asyncio.to_thread(
                    descendant_window_states, os.getpid()
                )
                if os.name == "nt" and not self.fake
                else [],
            },
        }

    async def close(self) -> None:
        try:
            with contextlib.suppress(Exception):
                if self.manager is not None:
                    await self.manager.__aexit__(None, None, None)
                elif self.context is not None:
                    await self.context.close()
        finally:
            if self._visibility_task is not None:
                self._visibility_task.cancel()
                with contextlib.suppress(asyncio.CancelledError):
                    await self._visibility_task
                self._visibility_task = None
            if self._window_guard is not None:
                await asyncio.to_thread(self._window_guard.stop)
                self._window_guard = None
        self.manager = None
        self.context = None
        self.page = None
        self.ready = False
        self.ready_at = None
        self.takeover_active = False
        self.takeover_reason = None
        self.takeover_kind = None
        self.takeover_started_at = None


class _FakePage:
    def __init__(self) -> None:
        self.url = "about:blank"

    async def goto(self, url: str, **_: Any) -> None:
        self.url = url

    async def title(self) -> str:
        return "Fake Camoufox"


class _FakeContext:
    def __init__(self) -> None:
        self.pages = [_FakePage()]

    async def close(self) -> None:
        self.pages.clear()
