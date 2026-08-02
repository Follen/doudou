from __future__ import annotations

import json
from typing import Any
from urllib.parse import parse_qsl, urlsplit

from ..common.core import sha256
from ..common.errors import DoudouError


def request_fields(url: str, body: bytes) -> dict[str, str]:
    """Decode the public form fields needed to verify a page-generated write."""
    fields = dict(parse_qsl(urlsplit(url).query, keep_blank_values=True))
    if body:
        try:
            decoded = body.decode("utf-8")
            body_value = json.loads(decoded)
            if isinstance(body_value, dict):
                fields.update(
                    {
                        str(key): str(value)
                        for key, value in body_value.items()
                        if isinstance(value, (str, int, float, bool))
                    }
                )
        except (UnicodeDecodeError, json.JSONDecodeError):
            fields.update(
                dict(
                    parse_qsl(
                        body.decode("utf-8", errors="replace"), keep_blank_values=True
                    )
                )
            )
    return fields


def validate_comment_request(
    fields: dict[str, str],
    *,
    aweme_id: str,
    text: str | None = None,
    comment_id: str | None = None,
) -> dict[str, str]:
    actual_aweme = fields.get("aweme_id") or fields.get("item_id")
    actual_text = fields.get("text")
    actual_comment = (
        fields.get("comment_id") or fields.get("cid") or fields.get("reply_id")
    )
    mismatches: list[str] = []
    if actual_aweme != aweme_id:
        mismatches.append("aweme_id")
    if text is not None and actual_text != text:
        mismatches.append("text")
    if comment_id is not None and actual_comment != comment_id:
        mismatches.append("comment_id")
    if mismatches:
        raise DoudouError(
            "PAGE_WRITE_REQUEST_DRIFTED",
            "The page-generated write request did not match the confirmed plan.",
            {"write_started": True, "mismatched_fields": mismatches},
        )
    return {
        "aweme_id": actual_aweme,
        **({"text": actual_text} if text is not None else {}),
        **({"comment_id": actual_comment} if comment_id is not None else {}),
    }


def parse_page_write_response(
    body: bytes, *, http_status: int, operation: str
) -> dict[str, Any]:
    metadata = {
        "write_started": True,
        "operation": operation,
        "http_status": http_status,
        "response_bytes": len(body),
        "response_sha256": sha256(body),
    }
    if not body:
        raise DoudouError(
            "WRITE_RESULT_UNKNOWN",
            "The page write returned an empty response.",
            metadata,
        )
    try:
        value = json.loads(body)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise DoudouError(
            "WRITE_RESULT_UNKNOWN",
            "The page write returned a non-JSON response.",
            metadata,
        ) from error
    if not isinstance(value, dict):
        raise DoudouError(
            "WRITE_RESULT_UNKNOWN",
            "The page write response was not an object.",
            metadata,
        )
    decision = str(
        value.get("decision") or (value.get("data") or {}).get("decision") or ""
    ).upper()
    if decision == "KICK":
        raise DoudouError(
            "WRITE_VERIFICATION_REQUIRED",
            "The page write reached account verification.",
            {**metadata, "result_known": True, "gateway_decision": "KICK"},
        )
    status_code = value.get("status_code", value.get("statusCode"))
    if not 200 <= http_status < 300 or status_code not in (None, 0):
        raise DoudouError(
            "VIDEO_WRITE_FAILED",
            "The page write returned a business error.",
            {**metadata, "result_known": True, "status_code": status_code},
        )
    return value


def response_comment(value: dict[str, Any]) -> dict[str, Any] | None:
    candidates = [
        value.get("comment"),
        (value.get("data") or {}).get("comment")
        if isinstance(value.get("data"), dict)
        else None,
    ]
    return next((item for item in candidates if isinstance(item, dict)), None)
