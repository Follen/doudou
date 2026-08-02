from __future__ import annotations

import asyncio
from pathlib import Path
from typing import Any

from ...common.errors import DoudouError
from ...reads.video import collect_user_videos, collect_video, resolve_identity
from ...runtime.browser.session import BrowserSession
from ...storage.content import read_video_comments
from ...ui.video import comment_write, like_comment
from ...writes.plans import complete_plan, confirm_plan, create_plan, load_plan
from ...writes.receipts import write_receipt


def verify_interaction_readback(
    operation: str,
    payload: dict[str, Any],
    result: dict[str, Any],
    comments: list[dict[str, Any]],
) -> dict[str, Any]:
    if operation == "video.comment.like":
        expected_cid = str(payload["comment"]["cid"])
        matches = [item for item in comments if str(item.get("cid")) == expected_cid]
        passed = len(matches) == 1 and bool(matches[0].get("user_digged"))
        return {
            "passed": passed,
            "operation": operation,
            "comment_id": expected_cid,
            "matches": len(matches),
            "user_digged": bool(matches[0].get("user_digged")) if matches else None,
        }

    created = result.get("created_comment") or {}
    expected_cid = str(created.get("cid") or "")
    matches = [item for item in comments if str(item.get("cid")) == expected_cid]
    expected_root = (
        str(payload["comment"]["cid"])
        if operation == "video.comment.reply"
        else expected_cid
    )
    passed = (
        bool(expected_cid)
        and len(matches) == 1
        and str(matches[0].get("text") or "") == str(payload.get("text") or "")
        and str(matches[0].get("reply_id") or "") == expected_root
    )
    return {
        "passed": passed,
        "operation": operation,
        "comment_id": expected_cid,
        "root_comment_id": expected_root,
        "matches": len(matches),
        "text_match": bool(matches)
        and str(matches[0].get("text") or "") == str(payload.get("text") or ""),
        "root_match": bool(matches)
        and str(matches[0].get("reply_id") or "") == expected_root,
    }


async def _protocol_recheck(
    session: BrowserSession,
    data_root: Path,
    profile: str,
    operation: str,
    plan: dict[str, Any],
    result: dict[str, Any],
) -> dict[str, Any]:
    payload = plan["payload"]
    collected = await collect_video(
        session,
        data_root,
        profile,
        str(payload["sec_uid"]),
        str(payload["aweme_id"]),
        1000,
    )
    verification = verify_interaction_readback(
        operation,
        payload,
        result,
        read_video_comments(data_root, profile, str(payload["aweme_id"])),
    )
    verification["collected_comments"] = int(collected.get("comments") or 0)
    return verification


async def identity(
    session: BrowserSession, data_root: Path, profile: str, douyin_id: str
) -> dict[str, Any]:
    return await resolve_identity(session, data_root, profile, douyin_id)


async def videos(
    session: BrowserSession, data_root: Path, profile: str, params: dict[str, Any]
) -> dict[str, Any]:
    resolved = await resolve_identity(session, data_root, profile, params["douyin_id"])
    return await collect_user_videos(
        session,
        data_root,
        profile,
        resolved["identity"],
        int(params.get("max_pages") or 1000),
    )


async def video(
    session: BrowserSession, data_root: Path, profile: str, params: dict[str, Any]
) -> dict[str, Any]:
    return await collect_video(
        session,
        data_root,
        profile,
        params["sec_uid"],
        params["aweme_id"],
        int(params.get("max_pages") or 1000),
    )


async def preview_interaction(
    data_root: Path, profile: str, operation: str, payload: dict[str, Any]
) -> dict[str, Any]:
    if not payload.get("aweme_id") or not payload.get("sec_uid"):
        raise DoudouError(
            "VIDEO_TARGET_REQUIRED", "A resolved sec_uid and aweme_id are required."
        )
    if operation in {"video.comment.create", "video.comment.reply"} and not str(
        payload.get("text") or ""
    ):
        raise DoudouError(
            "COMMENT_TEXT_REQUIRED", "A non-empty confirmed comment body is required."
        )
    if operation in {"video.comment.reply", "video.comment.like"}:
        comment_id = str((payload.get("comment") or {}).get("cid") or "")
        matches = [
            item
            for item in read_video_comments(
                data_root, profile, str(payload["aweme_id"])
            )
            if str(item.get("cid")) == comment_id
        ]
        if len(matches) != 1:
            raise DoudouError(
                "COMMENT_TARGET_NOT_FOUND",
                "The comment ID is not unique in the collected protocol records.",
                {"comment_id": comment_id, "matches": len(matches)},
            )
        supplied_text = str((payload.get("comment") or {}).get("text") or "")
        if supplied_text and supplied_text != str(matches[0].get("text") or ""):
            raise DoudouError(
                "COMMENT_TARGET_DRIFTED",
                "The supplied comment text does not match the collected protocol record.",
                {"comment_id": comment_id},
            )
        payload = {**payload, "comment": matches[0]}
    return {
        "status": "confirmation_required",
        "plan": create_plan(data_root, profile, operation, payload),
    }


async def execute_interaction(
    session: BrowserSession,
    data_root: Path,
    profile: str,
    operation: str,
    params: dict[str, Any],
) -> dict[str, Any]:
    stored = load_plan(data_root, profile, params["plan_id"], operation)
    plan = confirm_plan(
        data_root, profile, params["plan_id"], operation, stored["payload"]
    )
    receipt_written = False
    try:
        result = await (
            like_comment(session, plan)
            if operation == "video.comment.like"
            else comment_write(session, plan)
        )
        readback = await _protocol_recheck(
            session, data_root, profile, operation, plan, result
        )
        result.setdefault("verification", {})["protocol_recheck"] = readback
        if not readback["passed"]:
            files = write_receipt(
                data_root,
                profile,
                "video",
                operation,
                {"plan_id": plan["plan_id"], **result, "status": "unknown"},
            )
            receipt_written = True
            complete_plan(
                data_root, profile, plan["plan_id"], "unknown", files["receipt"]
            )
            raise DoudouError(
                "WRITE_RESULT_UNKNOWN",
                "The page write succeeded but protocol read-back did not confirm it.",
                {
                    "write_started": True,
                    "plan_id": plan["plan_id"],
                    "receipt": files["receipt"],
                    "protocol_recheck": readback,
                },
            )
        files = write_receipt(
            data_root,
            profile,
            "video",
            operation,
            {"plan_id": plan["plan_id"], **result},
        )
        complete_plan(
            data_root, profile, plan["plan_id"], result["status"], files["receipt"]
        )
        return {**result, "files": files}
    except asyncio.CancelledError:
        if not receipt_written:
            complete_plan(data_root, profile, plan["plan_id"], "unknown")
        raise
    except Exception as error:
        if receipt_written:
            raise
        details = (
            error.details
            if isinstance(error, DoudouError) and isinstance(error.details, dict)
            else {}
        )
        status = (
            "failed"
            if details.get("result_known") or not details.get("write_started")
            else "unknown"
        )
        complete_plan(data_root, profile, plan["plan_id"], status)
        raise
