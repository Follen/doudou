from __future__ import annotations

import asyncio
from typing import Any

from ..common.errors import DoudouError
from ..protocol.responses import ResponseCodes, parse_json_response
from ..runtime.browser.session import BrowserSession


async def parse_result(
    session: BrowserSession,
    result: dict[str, Any],
    *,
    codes: ResponseCodes,
    operation: str,
) -> dict[str, Any]:
    login = await session.classify_login()
    return parse_json_response(
        result,
        codes=codes,
        authenticated=bool(login.get("authenticated_cookie_count")),
        verification_visible=bool(login.get("verification_visible")),
        operation=operation,
    )


async def fetch_json(
    session: BrowserSession,
    url: str,
    *,
    codes: ResponseCodes,
    operation: str,
    timeout_ms: int = 15_000,
    retry_upstream_once: bool = True,
) -> dict[str, Any]:
    attempts = 2 if retry_upstream_once else 1
    for attempt in range(1, attempts + 1):
        try:
            result = await asyncio.wait_for(
                session.fetch_text(url, timeout_ms=timeout_ms),
                timeout=(timeout_ms + 1_000) / 1_000,
            )
        except TimeoutError as error:
            raise DoudouError(
                "PROTOCOL_TRANSPORT_TIMEOUT",
                "The page-context protocol request exceeded its bounded timeout.",
                {"operation": operation, "timeout_ms": timeout_ms},
            ) from error
        except Exception as error:
            if isinstance(error, DoudouError):
                raise
            raise DoudouError(
                "PROTOCOL_TRANSPORT_FAILED",
                "The page-context protocol request failed before a response was received.",
                {"operation": operation, "error_type": type(error).__name__},
            ) from error
        try:
            return await parse_result(session, result, codes=codes, operation=operation)
        except DoudouError as error:
            if error.code != "UPSTREAM_UNAVAILABLE" or attempt == attempts:
                raise
            await session.page.wait_for_timeout(500)
    raise AssertionError("unreachable")
