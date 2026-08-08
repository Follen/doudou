from __future__ import annotations

import asyncio
import base64
import contextlib
import os
from typing import Any

from ...common.core import atomic_write_json, read_json, utc_now
from ...common.errors import DoudouError
from ..paths import ProfilePaths
from ..platform.windows import suppress_asyncio_console_windows
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

_PROTOBUF_NETWORK_RETRY_DELAYS = (0.25, 0.5)


class BrowserSession:
    def __init__(
        self, paths: ProfilePaths, visible: bool = False, fake: bool = False
    ) -> None:
        self.paths = paths
        self.visible = visible
        self.fake = fake
        requested_mode = os.environ.get("DOUDOU_BROWSER_MODE", "").strip().lower()
        self.background_mode = (
            "fake"
            if fake
            else "headless"
            if requested_mode == "headless"
            else platform_background_mode()
        )
        self.mode = "headed_takeover" if visible and not fake else self.background_mode
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
        self.user_info_responses: list[bytes] = []
        self.user_info_endpoint: str | None = None
        self._init_request_future: asyncio.Future[bytes] | None = None
        self._init_response_future: asyncio.Future[bytes] | None = None
        self._user_info_response_event = asyncio.Event()
        self._user_info_endpoint_event = asyncio.Event()
        self._network_tasks: set[asyncio.Task[Any]] = set()
        self._auth_state_event = asyncio.Event()
        self._auth_event_generation = 0
        self._auth_reconciled_generation = 0
        self._auth_reconcile_task: asyncio.Task[None] | None = None
        self.takeover_active = False
        self.takeover_reason: str | None = None
        self.takeover_kind: str | None = None
        self.takeover_started_at: str | None = None
        self.takeover_sms_requested = False
        self.context_generation = 0
        self.mode_transition_count = 0
        self.last_mode_transition: dict[str, Any] | None = None
        self._mode_transition_lock = asyncio.Lock()
        self._mode_transitioning = False

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
            self.context_generation += 1
            return
        await self._launch_context(self.visible)

    def _headless_setting(self, visible: bool) -> bool | str:
        if visible:
            return False
        if self.background_mode in {"headless", "windows_headless"}:
            return True
        if self.background_mode == "linux_virtual_headed":
            return "virtual"
        return False

    async def _launch_context(self, visible: bool) -> None:
        try:
            from ..camoufox_asset import configure_cache

            configure_cache()
            from camoufox.async_api import AsyncCamoufox
        except ImportError as error:
            raise DoudouError(
                "CAMOUFOX_MISSING",
                "Camoufox is not installed in the active doudou environment.",
            ) from error
        config_record = read_json(self.paths.camoufox_config)
        config = self._fingerprint_config()
        headless = self._headless_setting(visible)
        self.visible = visible
        self.mode = "headed_takeover" if visible else self.background_mode
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
        await self._attach_auth_state_observer()
        self._attach_im_capture()
        self.started_at = utc_now()
        self.context_generation += 1

    async def _adopt_single_business_page(self) -> None:
        restored_pages = [
            page for page in self.context.pages if not page.is_closed()
        ]
        self.page = await self.context.new_page()
        for extra in restored_pages:
            with contextlib.suppress(Exception):
                await extra.close()

    async def set_visible(self, visible: bool) -> dict[str, Any]:
        previous_mode = self.mode
        if self.fake:
            self.visible = visible
            self.mode = "fake"
        elif visible != self.visible or self.context is None:
            await self._restart_context(visible, preserve_url=True)
        return {
            "visible": visible,
            "mode": self.mode,
            "previous_mode": previous_mode,
            "context_generation": self.context_generation,
            "mode_transition_count": self.mode_transition_count,
        }

    async def _restart_context(self, visible: bool, preserve_url: bool) -> None:
        async with self._mode_transition_lock:
            previous_mode = self.mode
            previous_url = (
                str(self.page.url)
                if preserve_url and self.page and not self.page.is_closed()
                else None
            )
            self._mode_transitioning = True
            await self._close_context(cancel_background_tasks=True)
            try:
                await self._launch_context(visible)
                if previous_url and previous_url.startswith("https://www.douyin.com"):
                    await self.open(previous_url)
            except BaseException:
                self.last_mode_transition = {
                    "at": utc_now(),
                    "from": previous_mode,
                    "to": "headed_takeover" if visible else self.background_mode,
                    "status": "failed",
                }
                raise
            finally:
                self._mode_transitioning = False
            self.mode_transition_count += 1
            self.last_mode_transition = {
                "at": utc_now(),
                "from": previous_mode,
                "to": self.mode,
                "status": "completed",
            }
            if (
                self.takeover_active
                and self._auth_reconciled_generation < self._auth_event_generation
            ):
                self._schedule_auth_reconciliation()

    async def present_for_takeover(
        self, reason: str, kind: str | None
    ) -> dict[str, Any]:
        self.takeover_active = True
        self.takeover_reason = reason
        self.takeover_kind = kind
        self.takeover_started_at = self.takeover_started_at or utc_now()
        presentation = await self.set_visible(True)
        await self._set_page_auth_observer_active(True)
        return {
            "active": True,
            "reason": reason,
            "verification_kind": kind,
            "started_at": self.takeover_started_at,
            "presentation": presentation,
            "resume_command": f"profile login --profile {self.paths.profile.name}",
        }

    async def finish_takeover(self) -> None:
        await self._set_page_auth_observer_active(False)
        await self.set_visible(False)
        self.takeover_active = False
        self.takeover_reason = None
        self.takeover_kind = None
        self.takeover_started_at = None
        self.takeover_sms_requested = False

    async def finalize_takeover_for_current_page(self) -> bool:
        if not self.takeover_active:
            return False
        if await self._verification_kind() is not None:
            return False
        login = await self.classify_login()
        self._record_login_state(login)
        if not login["valid"]:
            return False
        await self.finish_takeover()
        return True

    async def request_sms_code_if_available(self) -> bool:
        if self.fake or not self.page or self.takeover_sms_requested:
            return False
        from ...ui.components import AuthChallengeComponent

        requested = await AuthChallengeComponent(self.page).request_sms_code()
        self.takeover_sms_requested = requested
        return requested

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
                        if self.takeover_active:
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
        await self._set_page_auth_observer_active(self.takeover_active)
        return {"url": self.page.url, "title": await self.page.title()}

    def _signal_auth_state_change(self) -> None:
        self._auth_event_generation += 1
        self._auth_state_event.set()
        self._schedule_auth_reconciliation()

    async def _attach_auth_state_observer(self) -> None:
        if self.fake or not self.context or not self.page:
            return

        def on_auth_state_changed(_source: Any = None) -> None:
            self._signal_auth_state_change()

        await self.context.expose_binding(
            "__doudouAuthStateChanged", on_auth_state_changed
        )
        script = """() => {
          if (globalThis.__doudouAuthObserverInstalled || !document.documentElement) return;
          globalThis.__doudouAuthObserverInstalled = true;
          globalThis.__doudouAuthObserverActive = false;
          let timer = null;
          const observer = new MutationObserver(() => {
            if (!globalThis.__doudouAuthObserverActive) return;
            clearTimeout(timer);
            timer = setTimeout(() => {
              const callback = globalThis.__doudouAuthStateChanged;
              if (typeof callback === 'function') callback().catch(() => {});
            }, 150);
          });
          observer.observe(document.documentElement, {
            childList: true,
            subtree: true,
            attributes: true,
            attributeFilter: ['class', 'style', 'hidden', 'aria-hidden', 'disabled']
          });
        }"""
        await self.context.add_init_script(f"({script})()")
        await self.page.evaluate(script)

        def on_frame_navigated(frame: Any) -> None:
            if frame != self.page.main_frame:
                return
            task = asyncio.create_task(self._restore_auth_observer_after_navigation())
            self._network_tasks.add(task)
            task.add_done_callback(self._network_tasks.discard)

        self.page.on("framenavigated", on_frame_navigated)

    async def _restore_auth_observer_after_navigation(self) -> None:
        await self._set_page_auth_observer_active(self.takeover_active)
        self._signal_auth_state_change()

    async def _set_page_auth_observer_active(self, active: bool) -> None:
        if self.fake or not self.page:
            return
        with contextlib.suppress(Exception):
            await self.page.evaluate(
                "active => { globalThis.__doudouAuthObserverActive = active; }",
                active,
            )

    def _attach_im_capture(self) -> None:
        if self.fake or not self.page:
            return
        loop = asyncio.get_running_loop()
        self._init_request_future = loop.create_future()
        self._init_response_future = loop.create_future()
        self._user_info_response_event.clear()
        self._user_info_endpoint_event.clear()

        def on_request(request: Any) -> None:
            if "/aweme/v1/web/im/user/info/" in request.url:
                self.user_info_endpoint = request.url
                self._user_info_endpoint_event.set()
            if "/v1/message/get_message_by_init" not in request.url:
                return
            value = request.post_data_buffer
            if value:
                self.init_request = bytes(value)
                self.init_endpoint = request.url.split("?", 1)[0]
                if self._init_request_future and not self._init_request_future.done():
                    self._init_request_future.set_result(self.init_request)

        async def consume_response(response: Any) -> None:
            if response.url.startswith(
                ("https://www.douyin.com/", "https://imapi.douyin.com/")
            ):
                self._signal_auth_state_change()
            is_init = "/v1/message/get_message_by_init" in response.url
            is_user_info = "/aweme/v1/web/im/user/info/" in response.url
            if response.status != 200 or not (is_init or is_user_info):
                return
            try:
                value = await response.body()
            except Exception:  # noqa: BLE001 - detached Playwright response callback
                return
            if is_user_info:
                response_body = bytes(value)
                if response_body not in self.user_info_responses:
                    self.user_info_responses.append(response_body)
                self._user_info_response_event.set()
            if is_init:
                self.init_response = bytes(value)
                if self._init_response_future and not self._init_response_future.done():
                    self._init_response_future.set_result(self.init_response)

        def on_response(response: Any) -> None:
            task = asyncio.create_task(consume_response(response))
            self._network_tasks.add(task)
            task.add_done_callback(self._network_tasks.discard)

        self.page.on("request", on_request)
        self.page.on("response", on_response)

    async def captured_user_info_responses(
        self, timeout_seconds: float = 10, quiet_seconds: float = 0.75
    ) -> list[bytes]:
        loop = asyncio.get_running_loop()
        deadline = loop.time() + timeout_seconds
        observed_count = -1
        quiet_since: float | None = None
        while loop.time() < deadline:
            count = len(self.user_info_responses)
            if count != observed_count:
                observed_count = count
                quiet_since = loop.time() if count else None
            if quiet_since is not None and loop.time() - quiet_since >= quiet_seconds:
                return list(self.user_info_responses)
            self._user_info_response_event.clear()
            remaining = min(0.25, max(0.0, deadline - loop.time()))
            if not remaining:
                break
            try:
                await asyncio.wait_for(
                    self._user_info_response_event.wait(), timeout=remaining
                )
            except TimeoutError:
                pass
        return list(self.user_info_responses)

    async def captured_user_info_endpoint(
        self, timeout_seconds: float = 5
    ) -> str | None:
        if self.user_info_endpoint:
            return self.user_info_endpoint
        try:
            await asyncio.wait_for(
                self._user_info_endpoint_event.wait(), timeout=timeout_seconds
            )
        except TimeoutError:
            return None
        return self.user_info_endpoint

    def _schedule_auth_reconciliation(self) -> None:
        if (
            not self.takeover_active
            or self._mode_transitioning
            or self._auth_reconcile_task is not None
        ):
            return
        self._auth_reconcile_task = asyncio.create_task(
            self._reconcile_auth_state_after_event()
        )
        self._network_tasks.add(self._auth_reconcile_task)
        self._auth_reconcile_task.add_done_callback(self._network_tasks.discard)

    async def _reconcile_auth_state_after_event(self) -> None:
        try:
            while (
                self.takeover_active
                and self._auth_reconciled_generation < self._auth_event_generation
            ):
                target_generation = self._auth_event_generation
                await asyncio.sleep(0.15)
                login = await self.classify_login()
                self._record_login_state(login)
                self._auth_reconciled_generation = target_generation
                if login["valid"]:
                    await self.finalize_takeover_for_current_page()
                    break
        except Exception:  # noqa: BLE001 - asynchronous observer is best effort
            return
        finally:
            self._auth_reconcile_task = None
            if (
                self.takeover_active
                and self._auth_reconciled_generation < self._auth_event_generation
            ):
                self._schedule_auth_reconciliation()
            self._auth_state_event.set()

    async def _verification_kind(self) -> str | None:
        if self.fake or not self.page:
            return None
        from ...ui.components import AuthChallengeComponent

        return await AuthChallengeComponent(self.page).detect()

    async def _verification_visible(self) -> bool:
        return await self._verification_kind() is not None

    async def _chat_session_evidence(self) -> dict[str, Any]:
        if self.fake:
            return {"chat_route": True, "loaded": True, "candidate_count": 1}
        if not self.page or not str(self.page.url).startswith(
            "https://www.douyin.com/chat"
        ):
            return {"chat_route": False, "loaded": False, "candidate_count": 0}
        try:
            from ...ui.components import ConversationListComponent

            return await ConversationListComponent(self.page).evidence()
        except Exception:  # noqa: BLE001 - login classification remains bounded
            return {"chat_route": True, "loaded": False, "candidate_count": 0}

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
        nonempty_cookie_count = sum(bool(item.get("value")) for item in cookies)
        verification_kind = await self._verification_kind()
        verification_visible = verification_kind is not None
        im_initialized = bool(
            self.init_request
            and self.init_response
            and not self.init_response.lstrip().startswith((b"{", b"["))
        )
        chat_session = await self._chat_session_evidence()
        account_session_valid = im_initialized or bool(chat_session.get("loaded"))
        if verification_kind == "qr_login":
            status, code = "login_required", "LOGIN_REQUIRED"
        elif verification_visible:
            status, code = "verification_required", "VERIFICATION_REQUIRED"
        elif account_session_valid:
            status, code = "valid", None
        else:
            status, code = "login_required", "LOGIN_REQUIRED"
        return {
            "status": status,
            "valid": status == "valid",
            "reason_code": code,
            "authenticated_cookie_count": nonempty_cookie_count
            if status == "valid"
            else 0,
            "cookie_count": nonempty_cookie_count,
            "account_session_valid": account_session_valid,
            "chat_session": chat_session,
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
        observed_challenge = False
        try:
            while asyncio.get_running_loop().time() < deadline:
                observed_generation = self._auth_event_generation
                login = await self.classify_login()
                self._record_login_state(login)
                observed_challenge = observed_challenge or bool(
                    login.get("verification_visible")
                )
                if login["valid"]:
                    return await self.ensure_ready(force_refresh=True)
                if observed_challenge and not login.get("verification_visible"):
                    try:
                        return await self.ensure_ready(force_refresh=True)
                    except DoudouError as error:
                        if error.code not in {
                            "LOGIN_REQUIRED",
                            "VERIFICATION_REQUIRED",
                        }:
                            raise
                if self._auth_event_generation != observed_generation:
                    continue
                self._auth_state_event.clear()
                if self._auth_event_generation != observed_generation:
                    self._auth_state_event.set()
                    continue
                remaining = deadline - asyncio.get_running_loop().time()
                try:
                    await asyncio.wait_for(
                        self._auth_state_event.wait(), timeout=min(remaining, 5)
                    )
                except TimeoutError:
                    continue
            login = await self.classify_login()
            kind = login.get("verification_kind")
            code = (
                "LOGIN_TIMEOUT"
                if kind in {None, "qr_login"}
                else "VERIFICATION_REQUIRED"
            )
            raise DoudouError(
                code,
                "Headed login or verification did not complete before the timeout.",
                login,
            )
        finally:
            await self.finalize_takeover_for_current_page()

    def _require_im_initialization(self, login: dict[str, Any]) -> None:
        if self.fake:
            return
        details = {
            "page_url": self.page.url if self.page else None,
            "login": login,
            "init_request_bytes": len(self.init_request or b""),
            "init_response_bytes": len(self.init_response or b""),
        }
        if not self.init_request:
            raise DoudouError(
                "IM_CAPTURE_MISSING",
                "The account is logged in, but this broker did not capture the IM initialization request.",
                details,
            )
        if not self.init_response or self.init_response.lstrip().startswith((b"{", b"[")):
            raise DoudouError(
                "IM_INITIALIZATION_MISSING",
                "The account is logged in, but IM initialization did not produce a usable Protobuf response.",
                details,
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
            self._require_im_initialization(login)
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
            self.user_info_responses = []
            self.user_info_endpoint = None
            self._user_info_response_event.clear()
            self._user_info_endpoint_event.clear()
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
        try:
            self._require_im_initialization(login)
        except DoudouError:
            if self.takeover_active:
                await self.finish_takeover()
            raise
        if self.takeover_active:
            await self.finish_takeover()
            return await self.ensure_ready(force_refresh=True)
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
        await self._restart_context(self.visible, preserve_url=False)

    async def post_protobuf(self, endpoint: str, body: bytes) -> tuple[int, bytes]:
        if not self.page or not endpoint.startswith("https://imapi.douyin.com/"):
            raise DoudouError(
                "PROTOCOL_ENDPOINT_INVALID",
                "Only the supported IM protocol endpoint is allowed.",
            )
        attempts = len(_PROTOBUF_NETWORK_RETRY_DELAYS) + 1
        for attempt in range(1, attempts + 1):
            try:
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
            except Exception as error:
                if "NetworkError" not in f"{type(error).__name__}: {error}":
                    raise
                if attempt == attempts:
                    raise DoudouError(
                        "PROTOCOL_NETWORK_ERROR",
                        "The page-context IM protocol request failed after bounded retries.",
                        {
                            "endpoint": endpoint,
                            "attempts": attempts,
                            "cause": type(error).__name__,
                        },
                    ) from error
                await asyncio.sleep(_PROTOBUF_NETWORK_RETRY_DELAYS[attempt - 1])
                continue
            return int(result["status"]), base64.b64decode(result["bodyBase64"])
        raise AssertionError("unreachable")

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
            "context_generation": self.context_generation,
            "mode_transition_count": self.mode_transition_count,
            "last_mode_transition": self.last_mode_transition,
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
        }

    async def _close_context(self, *, cancel_background_tasks: bool) -> None:
        current_task = asyncio.current_task()
        if cancel_background_tasks:
            cancellable = [
                task
                for task in self._network_tasks
                if task is not current_task and not task.done()
            ]
            for task in cancellable:
                task.cancel()
            for task in cancellable:
                with contextlib.suppress(asyncio.CancelledError, Exception):
                    await task
            self._network_tasks = {
                task
                for task in self._network_tasks
                if task is current_task and not task.done()
            }
        try:
            with contextlib.suppress(Exception):
                if self.manager is not None:
                    await self.manager.__aexit__(None, None, None)
                elif self.context is not None:
                    await self.context.close()
        finally:
            self.manager = None
            self.context = None
            self.page = None
            self.ready = False
            self.ready_at = None
            self.init_request = None
            self.init_response = None
            self.init_endpoint = None
            self.user_info_responses = []
            self.user_info_endpoint = None
            self._user_info_response_event.clear()
            self._user_info_endpoint_event.clear()
            self._init_request_future = None
            self._init_response_future = None

    async def close(self) -> None:
        await self._close_context(cancel_background_tasks=True)
        if self._auth_reconcile_task is not None:
            if self._auth_reconcile_task is not asyncio.current_task():
                self._auth_reconcile_task.cancel()
                with contextlib.suppress(asyncio.CancelledError):
                    await self._auth_reconcile_task
            self._auth_reconcile_task = None
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
