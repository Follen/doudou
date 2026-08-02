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
from doudou.runtime.broker.client import BrokerClient
from doudou.runtime.broker.server import BrokerServer
from doudou.runtime.browser.session import BrowserSession
from doudou.runtime.profile import create_profile


class BrokerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.previous_timeout = os.environ.get("DOUDOU_BROKER_IDLE_SECONDS")
        os.environ["DOUDOU_BROKER_IDLE_SECONDS"] = "3"

    def tearDown(self) -> None:
        if self.previous_timeout is None:
            os.environ.pop("DOUDOU_BROKER_IDLE_SECONDS", None)
        else:
            os.environ["DOUDOU_BROKER_IDLE_SECONDS"] = self.previous_timeout

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
            async def is_visible(self) -> bool:
                return True

            async def evaluate(self, _script: str) -> str:
                return "DIV"

            async def inner_text(self, timeout: int = 1000) -> str:
                return "安全验证"

        class Locator:
            def __init__(self, visible: bool) -> None:
                self.visible = visible

            async def count(self) -> int:
                return int(self.visible)

            def nth(self, _index: int) -> Candidate:
                return Candidate()

        class Adapter:
            def __init__(self, target: str, quick: bool = False) -> None:
                self.target = target
                self.quick = quick

            def locator(self, _page: object, kind: str) -> Locator:
                return Locator(kind == self.target)

            def text(self, _page: object, key: str, exact: bool = False) -> Locator:
                return Locator(key == "quick_login" and self.quick)

        async def detect(target: str, quick: bool = False) -> str | None:
            with tempfile.TemporaryDirectory() as directory:
                _, paths = create_profile(Path(directory), "work")
                browser = BrowserSession(paths, fake=False)
                browser.page = object()
                with patch(
                    "doudou.ui.adapters.AUTH_ADAPTER", Adapter(target, quick)
                ):
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

    def test_browser_session_keeps_only_one_restored_page(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            _, paths = create_profile(Path(directory), "work")

            async def exercise() -> tuple[object, AsyncMock]:
                first = MagicMock()
                first.is_closed.return_value = False
                extra = MagicMock()
                extra.is_closed.return_value = False
                extra.close = AsyncMock()
                browser = BrowserSession(paths, visible=False, fake=True)
                browser.context = MagicMock()
                browser.context.pages = [first, extra]
                await browser._adopt_single_business_page()
                return browser.page, extra.close

            page, close = asyncio.run(exercise())
            self.assertIsNotNone(page)
            close.assert_awaited_once()

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
