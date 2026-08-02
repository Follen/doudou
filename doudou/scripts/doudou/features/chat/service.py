from __future__ import annotations

import asyncio
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

from ...common.core import sha256
from ...common.errors import DoudouError
from ...models.resolver import (
    default_message_window,
    infer_self_uid,
    resolve_conversation,
    resolve_message,
)
from ...protocol.im import property_has_user
from ...reads.chat import collect_messages, refresh_session_index
from ...runtime.browser.session import BrowserSession
from ...storage.repository import read_messages
from ...ui.chat import like_message, send_message
from ...writes.plans import complete_plan, confirm_plan, create_plan, load_plan
from ...writes.receipts import write_receipt


def verify_send_readback(
    payload: dict[str, Any], result: dict[str, Any], messages: list[dict[str, Any]]
) -> dict[str, Any]:
    server_id = str((result.get("verification") or {}).get("server_message_id") or "")
    matches = [item for item in messages if str(item.get("message_id")) == server_id]
    message = matches[0] if len(matches) == 1 else None
    expected_type = 27 if payload["mode"] == "image" else 7
    text_match = bool(message) and (
        payload["mode"] == "image"
        or str(payload.get("text") or "") in str(message.get("text") or "")
    )
    reference_match = bool(message) and (
        not payload.get("reply")
        or str(message.get("reply_to_message_id") or "")
        == str(payload["reply"]["message_id"])
    )
    passed = (
        bool(server_id)
        and len(matches) == 1
        and str(message.get("conversation_id"))
        == str(payload["conversation"]["conversation_id"])
        and int(message.get("message_type") or 0) == expected_type
        and text_match
        and reference_match
    )
    return {
        "passed": passed,
        "server_message_id": server_id,
        "matches": len(matches),
        "message_type_match": bool(message)
        and int(message.get("message_type") or 0) == expected_type,
        "text_match": text_match,
        "reference_match": reference_match,
    }


def verify_like_readback(
    payload: dict[str, Any], messages: list[dict[str, Any]]
) -> dict[str, Any]:
    message_id = str(payload["message"]["message_id"])
    matches = [item for item in messages if str(item.get("message_id")) == message_id]
    liked = len(matches) == 1 and property_has_user(
        matches[0].get("property_list"), str(payload["self_uid"])
    )
    return {
        "passed": liked,
        "message_id": message_id,
        "matches": len(matches),
        "self_uid": str(payload["self_uid"]),
        "user_liked": liked,
    }


async def _collect_readback(
    session: BrowserSession,
    data_root: Path,
    profile: str,
    conversation: dict[str, Any],
    since_ms: int | None = None,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    now = datetime.now(timezone.utc)
    lower = (
        datetime.fromtimestamp(since_ms / 1000, timezone.utc) - timedelta(minutes=1)
        if since_ms
        else now - timedelta(minutes=10)
    )
    run = await collect_messages(
        session,
        data_root,
        profile,
        str(conversation["conversation_id"]),
        lower.isoformat(),
        (now + timedelta(minutes=2)).isoformat(),
        10,
        False,
    )
    return run, read_messages(data_root, profile, conversation)


async def conversations(
    session: BrowserSession,
    data_root: Path,
    profile: str,
    force: bool = False,
    kind: str = "all",
) -> dict[str, Any]:
    refreshed = await refresh_session_index(session, data_root, profile, force=force)
    values = refreshed["index"]["conversations"]
    if kind != "all":
        values = [item for item in values if item["kind"] == kind]
    return {
        "status": "ok",
        "source": refreshed["source"],
        "refreshed_at": refreshed["index"]["refreshed_at"],
        "count": len(values),
        "conversations": values,
    }


async def collect(
    session: BrowserSession, data_root: Path, profile: str, params: dict[str, Any]
) -> dict[str, Any]:
    from_time, to_time = params.get("from"), params.get("to")
    if not from_time and not to_time:
        from_time, to_time = default_message_window(datetime.now().astimezone())
    result = await collect_messages(
        session,
        data_root,
        profile,
        params["target"],
        from_time,
        to_time,
        int(params.get("max_pages") or 100),
        not bool(params.get("skip_images")),
    )
    result["requested_window"] = {"from": from_time, "to": to_time}
    return result


async def preview_send(
    session: BrowserSession, data_root: Path, profile: str, params: dict[str, Any]
) -> dict[str, Any]:
    if params.get("at"):
        raise DoudouError(
            "MENTION_UNSUPPORTED",
            "Group mentions are not supported.",
        )
    refreshed = await refresh_session_index(session, data_root, profile)
    conversation = resolve_conversation(
        refreshed["index"]["conversations"], params["target"]
    )
    text, image = params.get("text"), params.get("image")
    if bool(text) == bool(image):
        raise DoudouError(
            "SEND_PAYLOAD_INVALID", "Exactly one of text or image is required."
        )
    payload: dict[str, Any] = {
        "conversation": conversation,
        "mode": "image" if image else "text",
    }
    if image:
        source = Path(image).expanduser().resolve()
        if not source.is_file():
            raise DoudouError(
                "IMAGE_NOT_FOUND",
                "The image file does not exist.",
                {"path": str(source)},
            )
        value = source.read_bytes()
        payload.update(
            {
                "image_path": str(source),
                "image": {
                    "name": source.name,
                    "bytes": len(value),
                    "sha256": sha256(value),
                },
            }
        )
    else:
        payload["text"] = str(text)
        if params.get("reply"):
            messages = read_messages(data_root, profile, conversation)
            target = resolve_message(messages, params["reply"])
            if not target.get("text"):
                raise DoudouError(
                    "REPLY_TARGET_TEXT_MISSING",
                    "The page writer needs the collected target text to select the reply action.",
                )
            payload["reply"] = {
                "message_id": target["message_id"],
                "text": target["text"],
                "sender": target.get("sender"),
            }
            payload["mode"] = "reply"
    plan = create_plan(data_root, profile, "chat.send", payload)
    return {"status": "confirmation_required", "plan": plan}


async def execute_send(
    session: BrowserSession, data_root: Path, profile: str, params: dict[str, Any]
) -> dict[str, Any]:
    stored = load_plan(data_root, profile, params["plan_id"], "chat.send")
    if stored["payload"].get("mention"):
        raise DoudouError(
            "MENTION_UNSUPPORTED",
            "This stored plan uses a group mention and can no longer be executed.",
        )
    plan = confirm_plan(
        data_root, profile, params["plan_id"], "chat.send", stored["payload"]
    )
    try:
        result = await send_message(session, plan)
        read_run, messages = await _collect_readback(
            session, data_root, profile, plan["payload"]["conversation"]
        )
        readback = verify_send_readback(plan["payload"], result, messages)
        readback["collect_run_id"] = read_run["run_id"]
        result.setdefault("verification", {})["protocol_recheck"] = readback
        if not readback["passed"]:
            raise DoudouError(
                "WRITE_RESULT_UNKNOWN",
                "The page send succeeded but protocol read-back did not confirm it.",
                {
                    "plan_id": plan["plan_id"],
                    "write_started": True,
                    "protocol_recheck": readback,
                },
            )
        files = write_receipt(
            data_root,
            profile,
            "chat",
            "chat.send",
            {"plan_id": plan["plan_id"], **result},
            send=True,
        )
        complete_plan(data_root, profile, plan["plan_id"], "sent", files["receipt"])
        return {**result, "files": files}
    except asyncio.CancelledError:
        complete_plan(data_root, profile, plan["plan_id"], "unknown")
        raise
    except Exception as error:
        details = (
            error.details
            if isinstance(error, DoudouError) and isinstance(error.details, dict)
            else {}
        )
        complete_plan(
            data_root,
            profile,
            plan["plan_id"],
            "unknown" if details.get("write_started") else "failed",
        )
        raise


async def preview_batch(
    session: BrowserSession, data_root: Path, profile: str, params: dict[str, Any]
) -> dict[str, Any]:
    refreshed = await refresh_session_index(session, data_root, profile)
    targets = list(params.get("targets") or [])
    text = str(params.get("text") or "")
    if not targets or not text:
        raise DoudouError(
            "BATCH_PLAN_INVALID",
            "Batch send requires at least one target and non-empty text.",
        )
    tasks = [
        {
            "conversation": resolve_conversation(
                refreshed["index"]["conversations"], target
            ),
            "mode": "text",
            "text": text,
        }
        for target in targets
    ]
    plan = create_plan(
        data_root, profile, "chat.send.batch", {"tasks": tasks, "interval_seconds": 5.0}
    )
    return {"status": "confirmation_required", "plan": plan}


async def execute_batch(
    session: BrowserSession, data_root: Path, profile: str, params: dict[str, Any]
) -> dict[str, Any]:
    stored = load_plan(data_root, profile, params["plan_id"], "chat.send.batch")
    plan = confirm_plan(
        data_root, profile, params["plan_id"], "chat.send.batch", stored["payload"]
    )
    tasks = list(plan["payload"]["tasks"])
    interval_seconds = float(plan["payload"]["interval_seconds"])
    if interval_seconds < 5:
        raise DoudouError(
            "INVALID_SEND_INTERVAL",
            "Batch send interval must be at least five seconds.",
        )
    results: list[dict[str, Any]] = []
    failed = False
    for index, payload in enumerate(tasks):
        if failed:
            results.append({"index": index, "status": "skipped"})
            continue
        try:
            subplan = {
                "schema": "doudou.write-plan.v1",
                "plan_id": f"{plan['plan_id']}:{index}",
                "operation": "chat.send",
                "payload": payload,
            }
            result = await send_message(session, subplan)
            read_run, messages = await _collect_readback(
                session, data_root, profile, payload["conversation"]
            )
            readback = verify_send_readback(payload, result, messages)
            readback["collect_run_id"] = read_run["run_id"]
            result.setdefault("verification", {})["protocol_recheck"] = readback
            if not readback["passed"]:
                raise DoudouError(
                    "WRITE_RESULT_UNKNOWN",
                    "The page send succeeded but protocol read-back did not confirm it.",
                    {"write_started": True, "protocol_recheck": readback},
                )
            files = write_receipt(
                data_root,
                profile,
                "chat",
                "chat.send",
                {"batch_plan_id": plan["plan_id"], "task_index": index, **result},
                send=True,
            )
            results.append({"index": index, **result, "files": files})
        except asyncio.CancelledError:
            complete_plan(data_root, profile, plan["plan_id"], "unknown")
            raise
        # A batch records one stable failed item and skips the remaining writes.
        except Exception as error:  # noqa: BLE001
            results.append(
                {
                    "index": index,
                    "status": "failed",
                    "error_code": getattr(error, "code", type(error).__name__),
                }
            )
            failed = True
            continue
        if index < len(tasks) - 1:
            try:
                await asyncio.sleep(interval_seconds)
            except asyncio.CancelledError:
                complete_plan(data_root, profile, plan["plan_id"], "unknown")
                raise
    status = "failed" if failed else "sent"
    files = write_receipt(
        data_root,
        profile,
        "chat",
        "chat.send.batch",
        {
            "plan_id": plan["plan_id"],
            "status": status,
            "results": results,
            "interval_seconds": interval_seconds,
        },
        send=True,
    )
    complete_plan(data_root, profile, plan["plan_id"], status, files["receipt"])
    return {
        "status": status,
        "plan_id": plan["plan_id"],
        "results": results,
        "interval_seconds": interval_seconds,
        "files": files,
    }


async def preview_like(
    session: BrowserSession, data_root: Path, profile: str, params: dict[str, Any]
) -> dict[str, Any]:
    refreshed = await refresh_session_index(session, data_root, profile)
    conversation = resolve_conversation(
        refreshed["index"]["conversations"], params["target"]
    )
    message = resolve_message(
        read_messages(data_root, profile, conversation), params["message"]
    )
    self_uid = infer_self_uid(refreshed["index"]["conversations"])
    if not self_uid:
        raise DoudouError(
            "SELF_IDENTITY_UNRESOLVED",
            "The current account UID could not be derived from the IM protocol index.",
        )
    plan = create_plan(
        data_root,
        profile,
        "chat.message.like",
        {
            "conversation": conversation,
            "message": {
                "message_id": message["message_id"],
                "text": message.get("text"),
                "property_list": message.get("property_list"),
                "create_time_ms": message.get("create_time_ms"),
            },
            "self_uid": self_uid,
            "already_liked": property_has_user(
                message.get("property_list"), self_uid
            ),
        },
    )
    return {"status": "confirmation_required", "plan": plan}


async def execute_like(
    session: BrowserSession, data_root: Path, profile: str, params: dict[str, Any]
) -> dict[str, Any]:
    stored = load_plan(data_root, profile, params["plan_id"], "chat.message.like")
    plan = confirm_plan(
        data_root, profile, params["plan_id"], "chat.message.like", stored["payload"]
    )
    try:
        result = await like_message(session, plan)
        read_run, messages = await _collect_readback(
            session,
            data_root,
            profile,
            plan["payload"]["conversation"],
            int(plan["payload"]["message"].get("create_time_ms") or 0) or None,
        )
        readback = verify_like_readback(plan["payload"], messages)
        readback["collect_run_id"] = read_run["run_id"]
        verification = result.setdefault("verification", {})
        verification["protocol_recheck"] = readback
        verification["passed"] = readback["passed"]
        verification["pending_protocol_recheck"] = False
        if not readback["passed"]:
            raise DoudouError(
                "WRITE_RESULT_UNKNOWN",
                "The message-like action was not confirmed by protocol read-back.",
                {
                    "plan_id": plan["plan_id"],
                    "write_started": result.get("status") != "already_liked",
                    "protocol_recheck": readback,
                },
            )
        files = write_receipt(
            data_root,
            profile,
            "chat",
            "chat.message.like",
            {"plan_id": plan["plan_id"], **result},
        )
        complete_plan(
            data_root, profile, plan["plan_id"], result["status"], files["receipt"]
        )
        return {**result, "files": files}
    except asyncio.CancelledError:
        complete_plan(data_root, profile, plan["plan_id"], "unknown")
        raise
    except Exception as error:
        details = (
            error.details
            if isinstance(error, DoudouError) and isinstance(error.details, dict)
            else {}
        )
        complete_plan(
            data_root,
            profile,
            plan["plan_id"],
            "unknown" if details.get("write_started") else "failed",
        )
        raise
