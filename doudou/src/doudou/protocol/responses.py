from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any

from ..common.core import sha256
from ..common.errors import DoudouError


@dataclass(frozen=True)
class ResponseCodes:
    empty: str
    invalid: str
    failed: str


def response_metadata(result: dict[str, Any]) -> dict[str, Any]:
    text = str(result.get("text") or "")
    return {
        "http_status": result.get("http_status"),
        "content_type": result.get("content_type"),
        "retry_after": result.get("retry_after"),
        "response_bytes": len(text.encode("utf-8")),
        "response_sha256": sha256(text),
    }


def _verification_payload(value: Any) -> bool:
    if isinstance(value, list):
        return any(_verification_payload(item) for item in value)
    if not isinstance(value, dict):
        return False
    decision = str(value.get("decision") or "").upper()
    if decision == "KICK":
        return True
    return any(_verification_payload(item) for item in value.values())


def _verification_document(text: str, content_type: str) -> bool:
    if "html" not in content_type.casefold() and not text.lstrip().startswith("<"):
        return False
    sample = text[:64_000].casefold()
    markers = (
        "captcha",
        "verify",
        "verification",
        "second-verify",
        "验证码",
        "安全验证",
        "滑块",
    )
    return any(marker in sample for marker in markers)


def parse_json_response(
    result: dict[str, Any],
    *,
    codes: ResponseCodes,
    authenticated: bool,
    verification_visible: bool,
    operation: str,
) -> dict[str, Any]:
    """Classify a browser fetch without leaking its response body into diagnostics."""
    metadata = {"operation": operation, **response_metadata(result)}
    text = str(result.get("text") or "")
    status = int(result.get("http_status") or 0)
    content_type = str(result.get("content_type") or "")

    if not authenticated:
        raise DoudouError(
            "LOGIN_REQUIRED", "The Profile login session is missing.", metadata
        )
    if status == 429:
        raise DoudouError(
            "RATE_LIMITED", "The upstream request was rate limited.", metadata
        )
    if 500 <= status <= 599:
        raise DoudouError(
            "UPSTREAM_UNAVAILABLE",
            "The upstream service is temporarily unavailable.",
            metadata,
        )
    if not text:
        raise DoudouError(codes.empty, "The protocol response was empty.", metadata)

    try:
        value = json.loads(text)
    except json.JSONDecodeError as error:
        if verification_visible or _verification_document(text, content_type):
            raise DoudouError(
                "VERIFICATION_REQUIRED",
                "The protocol request reached account verification.",
                metadata,
            ) from error
        raise DoudouError(
            codes.invalid, "The protocol response was not valid JSON.", metadata
        ) from error

    gateway_kick = _verification_payload(value)
    if verification_visible or gateway_kick:
        if gateway_kick:
            metadata["gateway_decision"] = "KICK"
        raise DoudouError(
            "VERIFICATION_REQUIRED",
            "The protocol request reached account verification.",
            metadata,
        )
    if not isinstance(value, dict):
        raise DoudouError(
            codes.invalid, "The protocol response JSON was not an object.", metadata
        )
    status_code = value.get("status_code", value.get("statusCode"))
    if status < 200 or status >= 300 or status_code not in (None, 0):
        raise DoudouError(
            codes.failed,
            "The protocol request returned a business error.",
            {**metadata, "status_code": status_code},
        )
    return value
