from __future__ import annotations

import asyncio
import json
import unittest

from doudou.common.errors import DoudouError
from doudou.protocol.responses import ResponseCodes, parse_json_response
from doudou.reads.transport import fetch_json

CODES = ResponseCodes(
    "COMMENT_RESPONSE_EMPTY", "COMMENT_RESPONSE_INVALID", "COMMENT_COLLECT_FAILED"
)


class ResponseClassificationTests(unittest.TestCase):
    def classify(
        self,
        text: str,
        status: int = 200,
        content_type: str = "application/json",
        authenticated: bool = True,
        visible: bool = False,
    ):
        return parse_json_response(
            {
                "http_status": status,
                "content_type": content_type,
                "retry_after": None,
                "text": text,
            },
            codes=CODES,
            authenticated=authenticated,
            verification_visible=visible,
            operation="comment.root.list",
        )

    def error_code(self, *args, **kwargs) -> DoudouError:
        with self.assertRaises(DoudouError) as context:
            self.classify(*args, **kwargs)
        return context.exception

    def test_stable_error_taxonomy_and_redacted_metadata(self) -> None:
        self.assertEqual(
            self.error_code("{}", authenticated=False).code, "LOGIN_REQUIRED"
        )
        self.assertEqual(
            self.error_code("", content_type="text/plain").code,
            "COMMENT_RESPONSE_EMPTY",
        )
        invalid = self.error_code("private gateway body", content_type="text/plain")
        self.assertEqual(invalid.code, "COMMENT_RESPONSE_INVALID")
        self.assertNotIn("private gateway body", json.dumps(invalid.details))
        self.assertEqual(self.error_code("{}", status=429).code, "RATE_LIMITED")
        self.assertEqual(self.error_code("{}", status=503).code, "UPSTREAM_UNAVAILABLE")
        self.assertEqual(
            self.error_code('{"status_code": 7}').code, "COMMENT_COLLECT_FAILED"
        )

    def test_html_verification_and_nested_kick_are_classified(self) -> None:
        self.assertEqual(
            self.error_code(
                "<html>captcha verify</html>", content_type="text/html"
            ).code,
            "VERIFICATION_REQUIRED",
        )
        kick = self.error_code('{"data":{"decision":"KICK"}}')
        self.assertEqual(kick.code, "VERIFICATION_REQUIRED")
        self.assertEqual(kick.details["gateway_decision"], "KICK")
        self.assertNotIn('{"data":{"decision":"KICK"}}', json.dumps(kick.as_event()))

    def test_valid_response_is_returned(self) -> None:
        self.assertEqual(
            self.classify('{"status_code":0,"comments":[]}')["comments"], []
        )


class _Page:
    def __init__(self) -> None:
        self.waits = 0

    async def wait_for_timeout(self, milliseconds: int) -> None:
        self.waits += 1


class _Session:
    def __init__(self, responses: list[dict]) -> None:
        self.responses = responses
        self.calls = 0
        self.page = _Page()

    async def fetch_text(self, url: str, timeout_ms: int = 15_000) -> dict:
        value = self.responses[self.calls]
        self.calls += 1
        return value

    async def classify_login(self) -> dict:
        return {
            "valid": True,
            "authenticated_cookie_count": 1,
            "verification_visible": False,
            "verification_kind": None,
        }


class _HangingSession(_Session):
    async def fetch_text(self, url: str, timeout_ms: int = 15_000) -> dict:
        await asyncio.Event().wait()
        raise AssertionError("unreachable")


class ResponseRetryTests(unittest.IsolatedAsyncioTestCase):
    async def test_browser_evaluate_is_bounded_by_python_timeout(self) -> None:
        session = _HangingSession([])
        with self.assertRaises(DoudouError) as context:
            await fetch_json(
                session,
                "https://www.douyin.com/test",
                codes=CODES,
                operation="test",
                timeout_ms=10,
            )
        self.assertEqual(context.exception.code, "PROTOCOL_TRANSPORT_TIMEOUT")

    async def test_only_upstream_5xx_is_retried_once(self) -> None:
        session = _Session(
            [
                {
                    "http_status": 503,
                    "content_type": "application/json",
                    "retry_after": None,
                    "text": "{}",
                },
                {
                    "http_status": 200,
                    "content_type": "application/json",
                    "retry_after": None,
                    "text": '{"status_code":0}',
                },
            ]
        )
        value = await fetch_json(
            session, "https://www.douyin.com/test", codes=CODES, operation="test"
        )
        self.assertEqual(value["status_code"], 0)
        self.assertEqual(session.calls, 2)
        self.assertEqual(session.page.waits, 1)

    async def test_invalid_json_is_not_retried(self) -> None:
        session = _Session(
            [
                {
                    "http_status": 200,
                    "content_type": "text/plain",
                    "retry_after": None,
                    "text": "invalid",
                },
                {
                    "http_status": 200,
                    "content_type": "application/json",
                    "retry_after": None,
                    "text": '{"status_code":0}',
                },
            ]
        )
        with self.assertRaises(DoudouError) as context:
            await fetch_json(
                session, "https://www.douyin.com/test", codes=CODES, operation="test"
            )
        self.assertEqual(context.exception.code, "COMMENT_RESPONSE_INVALID")
        self.assertEqual(session.calls, 1)


if __name__ == "__main__":
    unittest.main()
