from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from ..common.core import sha256
from ..common.errors import DoudouError
from ..protocol.im import parse_send_request, parse_send_response
from ..runtime.browser.session import BrowserSession
from .common import NetworkCapture
from .components import (
    ChatComposerComponent,
    ConversationListComponent,
    ImagePreviewComponent,
    MessageItemComponent,
)

SEND_PATHS = (
    "/v1/message/send",
    "/aweme/v1/im/consistency/action/report",
    "/passport/safe/get_identity_security_token/",
    "/upload/",
    "batch_build_image",
)


async def open_conversation(session: BrowserSession, display_name: str) -> int:
    await session.ensure_ready()
    if not str(session.page.url).startswith("https://www.douyin.com/chat"):
        await session.open("https://www.douyin.com/chat")
    operations = await ConversationListComponent(session.page).open(display_name)
    await session.page.wait_for_timeout(1200)
    return operations


async def _select_reply(page: Any, target_text: str) -> int:
    component = MessageItemComponent(page)
    target = await component.target(target_text, "chat.reply")
    return await component.menu_action(target, "回复", "chat.reply")


def _send_verification(capture: NetworkCapture, plan: dict[str, Any]) -> dict[str, Any]:
    requests = [
        body for path, body in capture.request_bodies if "/v1/message/send" in path
    ]
    responses = [
        body for path, body in capture.response_bodies if "/v1/message/send" in path
    ]
    payload = plan["payload"]
    request: dict[str, Any] = {}
    response: dict[str, Any] = {}
    parse_error = None
    try:
        if len(requests) == 1:
            request = parse_send_request(requests[0])
        if len(responses) == 1:
            response = parse_send_response(responses[0])
    except (TypeError, ValueError, UnicodeDecodeError, json.JSONDecodeError) as error:
        parse_error = type(error).__name__
    content = request.get("content") if isinstance(request.get("content"), dict) else {}
    actual_text = content.get("text") if isinstance(content, dict) else None
    expected_text = None if payload["mode"] == "image" else payload.get("text")
    conversation = payload["conversation"]
    expected_type = 27 if payload["mode"] == "image" else 7
    image_shape_match = payload["mode"] != "image" or (
        content.get("aweType") == 2702
        and isinstance(content.get("resource_url"), dict)
        and bool(content["resource_url"].get("oid"))
        and bool(content["resource_url"].get("skey"))
    )
    text_match = (
        expected_text is None
        or actual_text == expected_text
    )
    request_matches = (
        request.get("conversation_id") == str(conversation["conversation_id"])
        and request.get("conversation_short_id")
        == str(conversation["conversation_short_id"])
        and request.get("conversation_type") == int(conversation["conversation_type"])
        and request.get("message_type") == expected_type
        and bool(request.get("client_message_id"))
        and text_match
        and image_shape_match
        and (
            not payload.get("reply")
            or request.get("referenced_message_id")
            == str(payload["reply"]["message_id"])
        )
    )
    client_id_match = bool(request.get("client_message_id")) and response.get(
        "client_message_id"
    ) == request.get("client_message_id")
    gateway = response.get("gateway")
    passed = (
        len(requests) == 1
        and len(responses) == 1
        and parse_error is None
        and request_matches
        and response.get("protobuf_status") == 0
        and response.get("business_status") == 0
        and bool(response.get("server_message_id"))
        and client_id_match
        and gateway is None
    )
    return {
        "passed": passed,
        "request_count": len(requests),
        "response_count": len(responses),
        "protobuf_status": response.get("protobuf_status"),
        "status_text": response.get("status_text"),
        "business_status": response.get("business_status"),
        "gateway_decision": gateway.get("decision")
        if isinstance(gateway, dict)
        else None,
        "parse_error": parse_error,
        "conversation_match": request.get("conversation_id")
        == str(conversation["conversation_id"]),
        "message_type_match": request.get("message_type") == expected_type,
        "text_match": text_match,
        "reference_match": not payload.get("reply")
        or request.get("referenced_message_id") == str(payload["reply"]["message_id"]),
        "image_shape_match": image_shape_match,
        "server_message_id_present": bool(response.get("server_message_id")),
        "server_message_id": response.get("server_message_id"),
        "client_message_id": response.get("client_message_id"),
        "client_message_id_match": client_id_match,
        "request_sha256": sha256(requests[0]) if len(requests) == 1 else None,
        "response_sha256": sha256(responses[0]) if len(responses) == 1 else None,
    }


async def send_message(session: BrowserSession, plan: dict[str, Any]) -> dict[str, Any]:
    payload = plan["payload"]
    page = session.page
    dom_operations = await open_conversation(
        session, payload["conversation"]["display_name"]
    )
    capture = NetworkCapture(page, SEND_PATHS)
    capture.start()
    try:
        mode = payload["mode"]
        planned_text: str | None = payload.get("text")
        if mode == "image":
            source = Path(payload["image_path"])
            if not source.is_file():
                raise DoudouError(
                    "IMAGE_NOT_FOUND",
                    "The confirmed image file no longer exists.",
                    {"path": str(source)},
                )
            image_preview = ImagePreviewComponent(page)
            dom_operations += await image_preview.choose(source)
            await page.wait_for_timeout(1000)
            dom_operations += await image_preview.send()
            planned_text = None
        else:
            if payload.get("reply"):
                dom_operations += await _select_reply(page, payload["reply"]["text"])
            composer = ChatComposerComponent(page)
            await composer.clear()
            await composer.insert_text(payload["text"])
            dom_operations += 2
            composed = await composer.text()
            if composed != planned_text:
                await composer.clear()
                raise DoudouError(
                    "COMPOSER_PLAN_MISMATCH",
                    "The page composer does not exactly match the confirmed plan.",
                    {
                        "planned_sha256": sha256(planned_text),
                        "actual_sha256": sha256(composed),
                    },
                )
            try:
                dom_operations += await composer.send()
            except Exception as error:
                raise DoudouError(
                    "WRITE_RESULT_UNKNOWN",
                    "The chat send control was invoked but its result is uncertain.",
                    {"plan_id": plan["plan_id"], "write_started": True},
                ) from error
        for _ in range(150):
            if any("/v1/message/send" in path for path, _ in capture.response_bodies):
                break
            await page.wait_for_timeout(100)
    finally:
        await capture.stop()
    verification = _send_verification(capture, plan)
    if not verification["passed"]:
        if (
            verification.get("gateway_decision") == "KICK"
            or await session._verification_visible()
        ):
            kind = await session._verification_kind()
            raise DoudouError(
                "WRITE_VERIFICATION_REQUIRED",
                "The page write triggered account verification and was not replayed.",
                {
                    "plan_id": plan["plan_id"],
                    "verification": verification,
                    "verification_kind": kind,
                    "write_started": True,
                    "automatic_replay_allowed": False,
                },
            )
        raise DoudouError(
            "WRITE_RESULT_UNKNOWN",
            "The page send did not produce one verified protocol receipt.",
            {
                "plan_id": plan["plan_id"],
                "verification": verification,
                "write_started": True,
            },
        )
    return {
        "status": "sent",
        "plan_id": plan["plan_id"],
        "dom_operations": dom_operations,
        "react_fiber_traversal": False,
        "network": {"requests": capture.requests, "responses": capture.responses},
        "verification": verification,
    }


async def like_message(session: BrowserSession, plan: dict[str, Any]) -> dict[str, Any]:
    payload = plan["payload"]
    if payload.get("already_liked"):
        return {
            "status": "already_liked",
            "plan_id": plan["plan_id"],
            "dom_operations": 0,
            "network": {"requests": [], "responses": []},
            "verification": {
                "passed": True,
                "already_liked": True,
                "source": "protocol_plan",
            },
        }
    page = session.page
    dom_operations = await open_conversation(
        session, payload["conversation"]["display_name"]
    )
    component = MessageItemComponent(page)
    target = await component.target(payload["message"]["text"], "chat.message.like")
    capture = NetworkCapture(page, ("/v1/message/modify_property",))
    capture.start()
    try:
        try:
            dom_operations += await component.menu_action(
                target, "点赞", "chat.message.like"
            )
        except DoudouError:
            raise
        except Exception as error:
            raise DoudouError(
                "WRITE_RESULT_UNKNOWN",
                "The message-like control was invoked but its result is uncertain.",
                {
                    "plan_id": plan["plan_id"],
                    "write_started": True,
                    "cause_type": type(error).__name__,
                },
            ) from error
        await page.wait_for_timeout(3000)
    finally:
        await capture.stop()
    modify_requests = [
        item
        for item in capture.requests
        if item["endpoint"]["path"].rstrip("/") == "/v1/message/modify_property"
    ]
    modify_responses = [
        item
        for item in capture.responses
        if item["endpoint"]["path"].rstrip("/") == "/v1/message/modify_property"
    ]
    transport_observed = bool(modify_requests or modify_responses)
    transport_consistent = (
        not transport_observed
        or (
            len(modify_requests) == 1
            and len(modify_responses) == 1
            and modify_responses[0]["http_status"] == 200
        )
    )
    if not transport_consistent:
        raise DoudouError(
            "WRITE_RESULT_UNKNOWN",
            "The message-like transport observation was incomplete.",
            {
                "plan_id": plan["plan_id"],
                "write_started": True,
                "request_count": len(modify_requests),
                "response_count": len(modify_responses),
            },
        )
    return {
        "status": "liked",
        "plan_id": plan["plan_id"],
        "dom_operations": dom_operations,
        "network": {"requests": capture.requests, "responses": capture.responses},
        "verification": {
            "passed": False,
            "pending_protocol_recheck": True,
            "transport_observed": transport_observed,
            "request_count": len(modify_requests),
            "response_count": len(modify_responses),
        },
    }
