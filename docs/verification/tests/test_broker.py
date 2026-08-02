from __future__ import annotations

import asyncio
import json
import os
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import AsyncMock, MagicMock, patch

from doudou.common.errors import DoudouError
from doudou.runtime.broker.client import BrokerClient, command_timeout_seconds
from doudou.runtime.broker.server import BrokerServer
from doudou.runtime.browser.session import BrowserSession
from doudou.runtime.profile import create_profile
from doudou.ui.components.auth import AuthChallengeComponent


class BrokerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.previous_timeout = os.environ.get("DOUDOU_BROKER_IDLE_SECONDS")
        os.environ["DOUDOU_BROKER_IDLE_SECONDS"] = "3"

    def tearDown(self) -> None:
        if self.previous_timeout is None:
            os.environ.pop("DOUDOU_BROKER_IDLE_SECONDS", None)
        else:
            os.environ["DOUDOU_BROKER_IDLE_SECONDS"] = self.previous_timeout

    def test_long_collection_commands_have_bounded_total_timeouts(self) -> None:
        self.assertEqual(command_timeout_seconds("videos.collect", {}), 600.0)
        self.assertEqual(command_timeout_seconds("video.collect", {}), 900.0)
        self.assertEqual(command_timeout_seconds("messages.collect", {}), 90.0)

    def test_commands_reuse_one_broker_and_idle_exit_cleans_runtime(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            paths = create_profile(root, "work")[1]
            client = BrokerClient(root, "work", fake_browser=True, start_timeout=10)
            first = client.request("status")["result"]
            second = client.request("ping")["result"]
            third = client.request("status")["result"]
            ready = client.request("session.refresh", semantics="control")["result"]
            reused = client.request("session.status")["result"]
            self.assertEqual(first["pid"], second["pid"])
            self.assertEqual(second["pid"], third["pid"])
            self.assertEqual(
                first["browser"]["started_at"], third["browser"]["started_at"]
            )
            self.assertEqual(ready["initialization_count"], 1)
            self.assertEqual(reused["browser"]["initialization_count"], 1)
            self.assertTrue(paths.lock.exists())
            deadline = time.monotonic() + 8
            while time.monotonic() < deadline and paths.broker_descriptor.exists():
                time.sleep(0.1)
            self.assertTrue(client.wait_stopped())
            self.assertFalse(paths.broker_descriptor.exists())
            self.assertFalse(paths.broker_token.exists())
            self.assertFalse(paths.lock.exists())

    def test_idle_timeout_starts_after_browser_startup(self) -> None:
        async def exercise() -> None:
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                create_profile(root, "work")
                server = BrokerServer(
                    root, "work", idle_timeout=0.05, fake_browser=True
                )

                async def delayed_start() -> None:
                    await asyncio.sleep(0.08)

                with patch.object(server.browser, "start", new=delayed_start):
                    await server.start()
                idle_task = asyncio.create_task(server._idle_watch())
                await asyncio.sleep(0.02)
                self.assertFalse(server.stop_event.is_set())
                await asyncio.sleep(0.06)
                self.assertTrue(server.stop_event.is_set())
                await idle_task
                await server.close()

        asyncio.run(exercise())

    def test_pre_dispatch_write_error_is_stable_for_same_request(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            create_profile(root, "work")
            client = BrokerClient(root, "work", fake_browser=True, start_timeout=10)
            request_id = "fixed-write-request"
            for _ in range(2):
                with self.assertRaises(DoudouError) as context:
                    client.request(
                        "unknown.write", semantics="write", request_id=request_id
                    )
                self.assertEqual(context.exception.code, "BROKER_COMMAND_UNKNOWN")
            client.request("shutdown", semantics="control", start=False)
            self.assertTrue(client.wait_stopped())

    def test_invalid_token_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            create_profile(root, "work")
            client = BrokerClient(root, "work", fake_browser=True, start_timeout=10)
            descriptor = client.ensure_started()

            async def request() -> dict:
                endpoint = descriptor["endpoint"]
                reader, writer = await asyncio.open_connection(
                    endpoint["host"], endpoint["port"]
                )
                writer.write(
                    json.dumps(
                        {
                            "token": "wrong",
                            "request_id": "bad",
                            "command": "ping",
                            "semantics": "read",
                        }
                    ).encode()
                    + b"\n"
                )
                await writer.drain()
                response = json.loads(await reader.readline())
                writer.close()
                await writer.wait_closed()
                return response

            response = asyncio.run(request())
            self.assertFalse(response["ok"])
            self.assertEqual(response["error"]["code"], "BROKER_AUTH_FAILED")
            client.request("shutdown", semantics="control", start=False)
            self.assertTrue(client.wait_stopped())

    def test_same_profile_name_in_two_data_roots_gets_two_brokers(self) -> None:
        os.environ["DOUDOU_BROKER_IDLE_SECONDS"] = "10"
        with (
            tempfile.TemporaryDirectory() as left,
            tempfile.TemporaryDirectory() as right,
        ):
            left_root, right_root = Path(left), Path(right)
            create_profile(left_root, "work")
            create_profile(right_root, "work")
            left_client = BrokerClient(
                left_root, "work", fake_browser=True, start_timeout=10
            )
            right_client = BrokerClient(
                right_root, "work", fake_browser=True, start_timeout=10
            )
            left_status = left_client.request("status")["result"]
            right_status = right_client.request("status")["result"]
            self.assertNotEqual(left_status["pid"], right_status["pid"])
            self.assertNotEqual(left_status["data_root"], right_status["data_root"])
            left_client.request("shutdown", semantics="control", start=False)
            right_client.request("shutdown", semantics="control", start=False)
            self.assertTrue(left_client.wait_stopped())
            self.assertTrue(right_client.wait_stopped())

    def test_takeover_reuses_existing_fake_session_and_restores_hidden_mode(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            _, paths = create_profile(Path(directory), "work")

            async def exercise() -> tuple[dict, dict, dict]:
                browser = BrowserSession(paths, visible=False, fake=True)
                await browser.start()
                shown = await browser.present_for_takeover(
                    "VERIFICATION_REQUIRED", "sms_code"
                )
                during = await browser.status()
                await browser.finish_takeover()
                after = await browser.status()
                await browser.close()
                return shown, during, after

            shown, during, after = asyncio.run(exercise())
            self.assertTrue(shown["active"])
            self.assertEqual(shown["verification_kind"], "sms_code")
            self.assertTrue(during["visible"])
            self.assertTrue(during["takeover"]["active"])
            self.assertFalse(after["visible"])
            self.assertFalse(after["takeover"]["active"])

    def test_verification_detector_distinguishes_all_takeover_kinds(self) -> None:
        class Candidate:
            def __init__(self, quick: bool = False) -> None:
                self.quick = quick

            async def is_visible(self) -> bool:
                return True

            async def is_enabled(self) -> bool:
                return True

            async def bounding_box(self) -> dict[str, int]:
                return {"x": 10, "y": 10, "width": 300, "height": 200}

            async def evaluate(self, _script: str) -> str:
                return "DIV"

            async def inner_text(self, timeout: int = 1000) -> str:
                return "安全验证"

            def locator(self, _selector: str) -> Locator:
                return Locator(True, self.quick)

            def get_by_text(self, text: str, exact: bool = False) -> Locator:
                return Locator(self.quick and text == "快捷登录", self.quick)

        class Locator:
            def __init__(self, visible: bool, quick: bool = False) -> None:
                self.visible = visible
                self.quick = quick

            async def count(self) -> int:
                return int(self.visible)

            def nth(self, _index: int) -> Candidate:
                return Candidate(self.quick)

            async def bounding_box(self) -> dict[str, int] | None:
                if not self.visible:
                    return None
                return await Candidate(self.quick).bounding_box()

        class Page:
            def locator(self, _selector: str) -> Locator:
                return Locator(True)

        class Adapter:
            def __init__(self, target: str, quick: bool = False) -> None:
                self.target = target
                self.quick = quick
                self.labels = {
                    "quick_login": "快捷登录",
                    "get_code": "获取验证码",
                    "send_code": "发送验证码",
                }

            def locator(self, _page: object, kind: str) -> Locator:
                return Locator(kind == self.target, self.quick)

            def text(self, _page: object, key: str, exact: bool = False) -> Locator:
                return Locator(key == "quick_login" and self.quick)

        async def detect(target: str, quick: bool = False) -> str | None:
            with tempfile.TemporaryDirectory() as directory:
                _, paths = create_profile(Path(directory), "work")
                browser = BrowserSession(paths, fake=False)
                browser.page = Page()
                with patch("doudou.ui.adapters.AUTH_ADAPTER", Adapter(target, quick)):
                    return await browser._verification_kind()

        fixtures = {
            ("sms_code", False): "sms_code",
            ("sms_code", True): "quick_login_sms",
            ("slider", False): "slider",
            ("qr_login", False): "qr_login",
            ("generic_verification", False): "generic_verification",
        }
        for (target, quick), expected in fixtures.items():
            with self.subTest(target=target, quick=quick):
                self.assertEqual(asyncio.run(detect(target, quick)), expected)

    def test_auth_challenge_disappearing_during_rebind_is_not_drift(self) -> None:
        class Candidate:
            async def is_visible(self) -> bool:
                return True

            async def bounding_box(self) -> dict[str, int]:
                return {"x": 0, "y": 0, "width": 10, "height": 10}

        class Locator:
            def __init__(self, visible: bool) -> None:
                self.visible = visible

            async def count(self) -> int:
                return int(self.visible)

            def nth(self, _index: int) -> Candidate:
                return Candidate()

        class Adapter:
            def __init__(self) -> None:
                self.labels: dict[str, str] = {}

            def locator(self, _page: object, kind: str) -> Locator:
                return Locator(kind == "slider")

        component = AuthChallengeComponent(MagicMock(), Adapter())
        component._challenge = AsyncMock(
            side_effect=DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "gone",
                {"candidate_count": 0, "visible_count": 0},
            )
        )
        self.assertIsNone(asyncio.run(component.detect()))

    def test_login_classification_accepts_loaded_chat_with_unknown_cookie(
        self,
    ) -> None:
        class Context:
            async def cookies(self, _urls: list[str]) -> list[dict[str, str]]:
                return [{"name": "future_auth_cookie", "value": "present"}]

        with tempfile.TemporaryDirectory() as directory:
            _, paths = create_profile(Path(directory), "work")

            async def exercise() -> dict:
                browser = BrowserSession(paths, fake=False)
                browser.context = Context()
                browser.page = MagicMock()
                browser.page.url = "https://www.douyin.com/chat"
                browser._verification_kind = AsyncMock(return_value=None)
                browser._chat_session_evidence = AsyncMock(
                    return_value={
                        "chat_route": True,
                        "loaded": True,
                        "candidate_count": 12,
                    }
                )
                login = await browser.classify_login()
                browser._record_login_state(login)
                return login

            login = asyncio.run(exercise())
            profile = json.loads(paths.metadata.read_text(encoding="utf-8"))
            self.assertTrue(login["valid"])
            self.assertEqual(login["authenticated_cookie_count"], 1)
            self.assertFalse(login["im_initialized"])
            self.assertEqual(profile["login_status"], "valid")
            self.assertIsNone(profile["last_error_code"])

    def test_logged_in_session_reports_im_capture_separately(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            _, paths = create_profile(Path(directory), "work")
            browser = BrowserSession(paths, fake=False)
            browser.page = MagicMock()
            browser.page.url = "https://www.douyin.com/chat"
            login = {"status": "valid", "valid": True}
            with self.assertRaises(DoudouError) as missing:
                browser._require_im_initialization(login)
            self.assertEqual(missing.exception.code, "IM_CAPTURE_MISSING")
            browser.init_request = b"request"
            browser.init_response = b'{"decision":"gateway"}'
            with self.assertRaises(DoudouError) as unusable:
                browser._require_im_initialization(login)
            self.assertEqual(unusable.exception.code, "IM_INITIALIZATION_MISSING")

    def test_async_auth_event_finishes_takeover_without_polling(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            _, paths = create_profile(Path(directory), "work")

            async def exercise() -> dict:
                browser = BrowserSession(paths, visible=False, fake=True)
                await browser.start()
                await browser.present_for_takeover("LOGIN_REQUIRED", "sms_code")
                browser._signal_auth_state_change()
                await asyncio.sleep(0.25)
                status = await browser.status()
                await browser.close()
                return status

            status = asyncio.run(exercise())
            self.assertFalse(status["visible"])
            self.assertFalse(status["takeover"]["active"])

    def test_wait_for_login_does_not_lose_event_during_clear_window(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            _, paths = create_profile(Path(directory), "work")

            async def exercise() -> tuple[dict, int]:
                browser = BrowserSession(paths, fake=False)
                browser.page = MagicMock()
                browser.page.url = "https://www.douyin.com/"
                browser.present_for_takeover = AsyncMock()
                browser.finalize_takeover_for_current_page = AsyncMock(
                    return_value=True
                )
                browser.ensure_ready = AsyncMock(return_value={"status": "ready"})
                calls = 0

                async def classify() -> dict:
                    nonlocal calls
                    calls += 1
                    if calls == 1:
                        asyncio.get_running_loop().call_soon(
                            browser._signal_auth_state_change
                        )
                        return {"status": "login_required", "valid": False}
                    return {"status": "valid", "valid": True}

                browser.classify_login = classify
                result = await browser.wait_for_login(0.5)
                return result, calls

            result, calls = asyncio.run(exercise())
            self.assertEqual(result["status"], "ready")
            self.assertEqual(calls, 2)

    def test_reconcile_consumes_events_arriving_during_classification(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            _, paths = create_profile(Path(directory), "work")

            async def exercise() -> tuple[int, int, int]:
                browser = BrowserSession(paths, fake=False)
                browser.takeover_active = True
                calls = 0

                async def classify() -> dict:
                    nonlocal calls
                    calls += 1
                    if calls == 1:
                        browser._signal_auth_state_change()
                        return {"status": "verification_required", "valid": False}
                    return {"status": "valid", "valid": True}

                async def finish() -> bool:
                    browser.takeover_active = False
                    return True

                browser.classify_login = classify
                browser._record_login_state = MagicMock()
                browser.finalize_takeover_for_current_page = AsyncMock(
                    side_effect=finish
                )
                browser._signal_auth_state_change()
                task = browser._auth_reconcile_task
                self.assertIsNotNone(task)
                await asyncio.wait_for(task, timeout=2)
                return (
                    calls,
                    browser._auth_event_generation,
                    browser._auth_reconciled_generation,
                )

            calls, generated, reconciled = asyncio.run(exercise())
            self.assertEqual(calls, 2)
            self.assertEqual(generated, 2)
            self.assertEqual(reconciled, generated)

    def test_navigation_restores_observer_for_active_takeover(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            _, paths = create_profile(Path(directory), "work")

            async def exercise() -> tuple[int, AsyncMock]:
                browser = BrowserSession(paths, fake=False)
                browser.takeover_active = True
                browser._set_page_auth_observer_active = AsyncMock()
                browser._schedule_auth_reconciliation = MagicMock()
                await browser._restore_auth_observer_after_navigation()
                return browser._auth_event_generation, (
                    browser._set_page_auth_observer_active
                )

            generation, restore = asyncio.run(exercise())
            self.assertEqual(generation, 1)
            restore.assert_awaited_once_with(True)

    def test_takeover_stays_visible_only_while_verification_is_present(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            _, paths = create_profile(Path(directory), "work")

            async def exercise() -> tuple[bool, dict]:
                browser = BrowserSession(paths, visible=False, fake=True)
                await browser.start()
                await browser.present_for_takeover("VERIFICATION_REQUIRED", "sms_code")
                browser._verification_kind = AsyncMock(return_value="sms_code")
                finished = await browser.finalize_takeover_for_current_page()
                status = await browser.status()
                await browser.close()
                return finished, status

            finished, status = asyncio.run(exercise())
            self.assertFalse(finished)
            self.assertTrue(status["visible"])
            self.assertTrue(status["takeover"]["active"])

    def test_business_failure_after_login_still_finishes_takeover(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            create_profile(root, "work")

            async def exercise() -> tuple[dict, dict]:
                async def handler(command, params, browser, data_root, profile):
                    await browser.present_for_takeover("LOGIN_REQUIRED", "sms_code")
                    raise DoudouError("UPSTREAM_UNAVAILABLE", "temporary")

                server = BrokerServer(root, "work", fake_browser=True, handler=handler)
                await server.start()
                try:
                    response = await server.execute(
                        {
                            "request_id": "failure-after-login",
                            "command": "messages.collect",
                            "semantics": "read",
                            "params": {},
                        }
                    )
                    return response, await server.browser.status()
                finally:
                    await server.close()

            response, status = asyncio.run(exercise())
            self.assertFalse(response["ok"])
            self.assertEqual(response["error"]["code"], "UPSTREAM_UNAVAILABLE")
            self.assertFalse(status["visible"])
            self.assertFalse(status["takeover"]["active"])

    def test_browser_session_replaces_restored_pages_with_fresh_business_page(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            _, paths = create_profile(Path(directory), "work")

            async def exercise() -> tuple[object, object, AsyncMock, AsyncMock]:
                first = MagicMock()
                first.is_closed.return_value = False
                first.close = AsyncMock()
                extra = MagicMock()
                extra.is_closed.return_value = False
                extra.close = AsyncMock()
                fresh = MagicMock()
                browser = BrowserSession(paths, visible=False, fake=True)
                browser.context = MagicMock()
                browser.context.pages = [first, extra]
                browser.context.new_page = AsyncMock(return_value=fresh)
                await browser._adopt_single_business_page()
                return browser.page, fresh, first.close, extra.close

            page, fresh, first_close, extra_close = asyncio.run(exercise())
            self.assertIs(page, fresh)
            first_close.assert_awaited_once()
            extra_close.assert_awaited_once()

    def test_ready_session_survives_navigation_between_douyin_routes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            _, paths = create_profile(Path(directory), "work")

            async def exercise() -> tuple[dict, dict]:
                browser = BrowserSession(paths, visible=False, fake=True)
                await browser.start()
                initialized = await browser.ensure_ready()
                await browser.open("https://www.douyin.com/user/example")
                reused = await browser.ensure_ready()
                await browser.close()
                return initialized, reused

            initialized, reused = asyncio.run(exercise())
            self.assertEqual(initialized["initialization_count"], 1)
            self.assertEqual(reused["source"], "existing_session")
            self.assertEqual(reused["initialization_count"], 1)
            self.assertTrue(reused["page_url"].endswith("/user/example"))

    def test_page_adapter_drift_is_not_hidden_as_unknown_write_result(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            create_profile(root, "work")

            async def exercise() -> dict:
                async def handler(command, params, browser, data_root, profile):
                    raise DoudouError(
                        "PAGE_ADAPTER_DRIFTED",
                        "adapter changed",
                        {"adapter_version": "v1"},
                    )

                server = BrokerServer(root, "work", fake_browser=True, handler=handler)
                await server.start()
                try:
                    return await server.execute(
                        {
                            "request_id": "drift",
                            "command": "chat.send",
                            "semantics": "write",
                            "params": {},
                        }
                    )
                finally:
                    await server.close()

            response = asyncio.run(exercise())
            self.assertFalse(response["ok"])
            self.assertEqual(response["error"]["code"], "PAGE_ADAPTER_DRIFTED")

    def test_pre_click_write_error_is_preserved(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            create_profile(root, "work")

            async def handler(command, params, browser, data_root, profile):
                raise DoudouError("COMMENT_COMPOSER_EMPTY", "empty before click")

            async def exercise():
                server = BrokerServer(root, "work", fake_browser=True, handler=handler)
                await server.start()
                try:
                    return await server.execute(
                        {
                            "request_id": "pre-click",
                            "command": "video.comment",
                            "semantics": "write",
                            "params": {},
                        }
                    )
                finally:
                    await server.close()

            response = asyncio.run(exercise())
            self.assertFalse(response["ok"])
            self.assertEqual(response["error"]["code"], "COMMENT_COMPOSER_EMPTY")

    def test_cancelled_write_returns_stable_unknown_result(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            create_profile(root, "work")

            async def exercise() -> dict:
                async def handler(command, params, browser, data_root, profile):
                    raise asyncio.CancelledError

                server = BrokerServer(root, "work", fake_browser=True, handler=handler)
                await server.start()
                try:
                    return await server.execute(
                        {
                            "request_id": "cancelled",
                            "command": "chat.like",
                            "semantics": "write",
                            "params": {},
                        }
                    )
                finally:
                    await server.close()

            response = asyncio.run(exercise())
            self.assertFalse(response["ok"])
            self.assertEqual(response["error"]["code"], "WRITE_RESULT_UNKNOWN")
            self.assertTrue(response["error"]["details"]["write_started"])

    def test_timed_out_write_releases_command_lock_and_is_stable(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            create_profile(root, "work")

            async def exercise() -> tuple[dict, dict, bool]:
                async def handler(command, params, browser, data_root, profile):
                    if command == "slow.write":
                        await asyncio.sleep(10)
                    return {"status": "ok"}

                server = BrokerServer(root, "work", fake_browser=True, handler=handler)
                await server.start()
                request = {
                    "request_id": "timed-out-write",
                    "command": "slow.write",
                    "semantics": "write",
                    "command_timeout_seconds": 0.05,
                    "params": {},
                }
                try:
                    first = await server.execute(request)
                    second = await server.execute(request)
                    return first, second, server.command_lock.locked()
                finally:
                    await server.close()

            first, second, locked = asyncio.run(exercise())
            self.assertEqual(first, second)
            self.assertFalse(first["ok"])
            self.assertEqual(first["error"]["code"], "WRITE_RESULT_UNKNOWN")
            self.assertEqual(first["error"]["details"]["timeout_seconds"], 0.1)
            self.assertFalse(locked)

    def test_read_verification_waits_and_retries_original_command_once(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            create_profile(root, "work")

            async def exercise() -> tuple[dict, int, int]:
                calls = 0
                waits = 0

                async def handler(command, params, browser, data_root, profile):
                    nonlocal calls
                    calls += 1
                    if calls == 1:
                        raise DoudouError("LOGIN_REQUIRED", "login")
                    return {"status": "resumed"}

                server = BrokerServer(root, "work", fake_browser=True, handler=handler)
                await server.start()

                async def verification_kind():
                    return "sms_code"

                async def wait_for_takeover_completion(timeout_seconds):
                    nonlocal waits
                    waits += 1
                    await server.browser.finish_takeover()
                    return {"status": "ready"}

                server.browser._verification_kind = verification_kind
                server.browser.wait_for_takeover_completion = (
                    wait_for_takeover_completion
                )
                try:
                    response = await server.execute(
                        {
                            "request_id": "recover",
                            "command": "messages.collect",
                            "semantics": "read",
                            "params": {},
                        }
                    )
                    return response, calls, waits
                finally:
                    await server.close()

            response, calls, waits = asyncio.run(exercise())
            self.assertTrue(response["ok"])
            self.assertEqual(response["result"]["status"], "resumed")
            self.assertEqual(calls, 2)
            self.assertEqual(waits, 1)

    def test_kick_without_visible_challenge_does_not_show_or_wait(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            create_profile(root, "work")

            async def exercise() -> tuple[dict, int, int]:
                calls = 0
                waits = 0

                async def handler(command, params, browser, data_root, profile):
                    nonlocal calls
                    calls += 1
                    raise DoudouError(
                        "VERIFICATION_REQUIRED",
                        "verification gateway",
                        {"gateway_decision": "KICK"},
                    )

                server = BrokerServer(root, "work", fake_browser=True, handler=handler)
                await server.start()

                async def wait_for_takeover_completion(timeout_seconds):
                    nonlocal waits
                    waits += 1
                    return {"status": "ready"}

                server.browser.wait_for_takeover_completion = (
                    wait_for_takeover_completion
                )
                try:
                    response = await server.execute(
                        {
                            "request_id": "kick",
                            "command": "comments.collect",
                            "semantics": "read",
                            "params": {},
                        }
                    )
                    return response, calls, waits
                finally:
                    await server.close()

            response, calls, waits = asyncio.run(exercise())
            self.assertFalse(response["ok"])
            self.assertEqual(response["error"]["code"], "VERIFICATION_REQUIRED")
            self.assertEqual(response["error"]["details"]["gateway_decision"], "KICK")
            self.assertFalse(response["error"]["details"]["takeover"]["active"])
            self.assertEqual(calls, 1)
            self.assertEqual(waits, 0)


if __name__ == "__main__":
    unittest.main()
