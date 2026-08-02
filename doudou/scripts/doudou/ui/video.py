from __future__ import annotations

from typing import Any

from ..common.errors import DoudouError
from ..protocol.contracts import WEB_PROTOCOL
from ..protocol.write_responses import (
    parse_page_write_response,
    request_fields,
    response_comment,
    validate_comment_request,
)
from ..runtime.browser.session import BrowserSession
from .common import NetworkCapture
from .components import (
    CommentItemComponent,
    VideoCommentComposerComponent,
    VideoCommentsPanelComponent,
)

SENTINELS = "\u200b\u200c\u200d\ufeff"


def _captured_write(
    capture: NetworkCapture, path: str, operation: str
) -> tuple[dict[str, str], dict[str, Any]]:
    expected = path.rstrip("/")
    requests = [
        request_fields(url, body)
        for (request_path, url), (body_path, body) in zip(
            capture.request_urls, capture.request_bodies, strict=True
        )
        if request_path.rstrip("/") == expected and body_path.rstrip("/") == expected
    ]
    responses = [
        (metadata, body)
        for metadata, (response_path, body) in zip(
            capture.responses, capture.response_bodies, strict=True
        )
        if response_path.rstrip("/") == expected
    ]
    if len(requests) != 1 or len(responses) != 1:
        raise DoudouError(
            "WRITE_RESULT_UNKNOWN",
            "The page write did not produce one request and one response.",
            {
                "write_started": True,
                "operation": operation,
                "request_count": len(requests),
                "response_count": len(responses),
                "captured_request_paths": sorted(
                    {item[0] for item in capture.request_urls}
                ),
                "captured_response_paths": sorted(
                    {item[0] for item in capture.response_bodies}
                ),
            },
        )
    metadata, body = responses[0]
    return requests[0], parse_page_write_response(
        body, http_status=int(metadata["http_status"]), operation=operation
    )


async def comment_write(
    session: BrowserSession, plan: dict[str, Any]
) -> dict[str, Any]:
    payload = plan["payload"]
    page, operations = await VideoCommentsPanelComponent().open(
        session, payload["aweme_id"]
    )
    operation = plan["operation"]
    item_component = CommentItemComponent(page)
    if operation == "video.comment.reply":
        item = await item_component.target(
            payload["comment"]["text"],
            (payload["comment"].get("user") or {}).get("nickname"),
        )
        operations += await item_component.reply(item)
        await page.wait_for_timeout(500)
    composer = VideoCommentComposerComponent(page, focused=operation == "video.comment.reply")
    if operation == "video.comment.create":
        operations += await composer.activate()
    editor = await composer.insert_text(payload["text"])
    composed = await composer.composed_text(editor)
    if composed.translate({ord(char): None for char in SENTINELS}) != payload["text"]:
        raise DoudouError(
            "COMPOSER_PLAN_MISMATCH",
            "The comment editor does not match the confirmed plan.",
        )
    publish_path = WEB_PROTOCOL.path("comment_publish")
    capture = NetworkCapture(page, ("/aweme/v1/web/comment/",))
    capture.start()
    try:
        operations += await composer.publish()
        operations += 2
        for _ in range(250):
            if any(
                response_path.rstrip("/") == publish_path.rstrip("/")
                for response_path, _body in capture.response_bodies
            ):
                break
            await page.wait_for_timeout(100)
        verification_visible = await session._verification_visible()
    finally:
        await capture.stop()
    if verification_visible:
        raise DoudouError(
            "WRITE_VERIFICATION_REQUIRED",
            "The page showed account verification after the comment write.",
            {"write_started": True, "plan_id": plan["plan_id"]},
        )
    fields, value = _captured_write(capture, publish_path, operation)
    target_comment_id = (
        payload.get("comment", {}).get("cid")
        if operation == "video.comment.reply"
        else None
    )
    verified_fields = validate_comment_request(
        fields,
        aweme_id=payload["aweme_id"],
        text=payload["text"],
        comment_id=target_comment_id,
    )
    created = response_comment(value)
    if (
        not created
        or str(created.get("text") or "") != payload["text"]
        or not str(created.get("cid") or "")
    ):
        raise DoudouError(
            "WRITE_RESULT_UNKNOWN",
            "The successful page response did not contain the confirmed comment.",
            {
                "write_started": True,
                "plan_id": plan["plan_id"],
                "response_comment_present": bool(created),
            },
        )
    return {
        "status": "created" if operation.endswith("create") else "replied",
        "plan_id": plan["plan_id"],
        "dom_operations": operations,
        "react_fiber_traversal": False,
        "target": verified_fields,
        "created_comment": {"cid": str(created["cid"]), "text": str(created["text"])},
        "network": {"requests": capture.requests, "responses": capture.responses},
        "verification": {"passed": True, "network_business_status": 0},
    }


async def like_comment(session: BrowserSession, plan: dict[str, Any]) -> dict[str, Any]:
    payload = plan["payload"]
    page, operations = await VideoCommentsPanelComponent().open(
        session, payload["aweme_id"]
    )
    component = CommentItemComponent(page)
    item = await component.target(
        payload["comment"]["text"],
        (payload["comment"].get("user") or {}).get("nickname"),
    )
    if payload["comment"].get("user_digged"):
        return {
            "status": "already_liked",
            "plan_id": plan["plan_id"],
            "dom_operations": operations,
            "verification": {"passed": True, "already_liked": True},
        }
    digg_path = WEB_PROTOCOL.path("comment_digg")
    capture = NetworkCapture(page, (digg_path,), exact_paths=True)
    capture.start()
    try:
        operations += await component.like(
            item, int(payload["comment"].get("digg_count") or 0)
        )
        for _ in range(200):
            if capture.responses:
                break
            await page.wait_for_timeout(100)
    finally:
        await capture.stop()
    fields, _value = _captured_write(capture, digg_path, "video.comment.like")
    verified_fields = validate_comment_request(
        fields, aweme_id=payload["aweme_id"], comment_id=payload["comment"]["cid"]
    )
    return {
        "status": "liked",
        "plan_id": plan["plan_id"],
        "dom_operations": operations,
        "target": verified_fields,
        "network": {"requests": capture.requests, "responses": capture.responses},
        "verification": {"passed": True, "network_business_status": 0},
    }
