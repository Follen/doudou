from __future__ import annotations

import asyncio
from pathlib import Path
from typing import Any

from ..common.core import (
    atomic_write_bytes,
    atomic_write_json,
    random_id,
    read_json,
    sha256,
    utc_now,
)
from ..common.errors import DoudouError
from ..runtime.browser.session import BrowserSession
from ..runtime.paths import profile_paths
from ..ui.common import NetworkCapture
from ..ui.components import (
    AuthChallengeComponent,
    ChatComposerComponent,
    CommentItemComponent,
    ConversationListComponent,
    ImagePreviewComponent,
    MessageItemComponent,
    TutorialDialogComponent,
    VideoCommentComposerComponent,
    VideoCommentsPanelComponent,
)
from ..ui.components.base import not_exercised_probe, visible_candidates


async def _doctor_stage(name: str, awaitable: Any, timeout_seconds: float) -> Any:
    try:
        async with asyncio.timeout(timeout_seconds):
            return await awaitable
    except TimeoutError as error:
        raise DoudouError(
            "PAGE_DOCTOR_STAGE_TIMEOUT",
            "A bounded Page Doctor stage exceeded its time limit.",
            {"stage": name, "timeout_seconds": timeout_seconds},
        ) from error


async def page_doctor(
    browser: BrowserSession,
    data_root: Path,
    profile: str,
    params: dict[str, Any] | None = None,
) -> dict[str, Any]:
    params = params or {}
    report_root = profile_paths(data_root, profile).diagnostics / "page-doctor"
    stage_path = report_root / "active-stage.json"
    atomic_write_json(stage_path, {"stage": "ensure_ready", "updated_at": utc_now()})
    ready = await _doctor_stage("ensure_ready", browser.ensure_ready(), 60)
    page = browser.page
    del params
    exercise = None
    exercise_result: dict[str, Any] | None = None
    atomic_write_json(stage_path, {"stage": "page_shape", "updated_at": utc_now()})
    shape = await _doctor_stage(
        "page_shape",
        page.evaluate(
        """() => ({
          url_path: location.pathname,
          scripts: [...document.scripts].map(item => item.src).filter(Boolean).slice(0, 300).map(value => { const url = new URL(value); return {origin: url.origin, path: url.pathname}; }),
        })"""
        ),
        10,
    )
    scripts = sorted(
        {f"{item['origin']}{item['path']}" for item in shape.pop("scripts")}
    )
    script_set_sha256 = sha256("\n".join(scripts))
    previous_files = sorted(
        report_root.glob("*/report.json"),
        key=lambda item: item.stat().st_mtime_ns,
        reverse=True,
    )
    previous = read_json(previous_files[0], optional=True) if previous_files else None
    component_reports: list[dict[str, Any]] = []
    atomic_write_json(stage_path, {"stage": "auth_probe", "updated_at": utc_now()})
    component_reports.extend(
        await _doctor_stage("auth_probe", AuthChallengeComponent(page).probe(), 10)
    )
    if shape["url_path"].startswith("/chat"):
        conversation_list = ConversationListComponent(page)
        atomic_write_json(
            stage_path, {"stage": "conversation_list_probe", "updated_at": utc_now()}
        )
        component_reports.append(
            await _doctor_stage(
                "conversation_list_probe", conversation_list.probe(True), 20
            )
        )
        composer = ChatComposerComponent(page)
        selected = bool(await visible_candidates(composer.candidates()))
        atomic_write_json(
            stage_path, {"stage": "chat_composer_probes", "updated_at": utc_now()}
        )
        component_reports.extend(
            await _doctor_stage(
                "chat_composer_probes",
                composer.operation_probes(selected),
                15,
            )
        )
        message_items = MessageItemComponent(page)
        component_reports.extend(
            [
                not_exercised_probe(
                    component=message_items.name,
                    version=message_items.version,
                    capability="chat.message.target",
                    strategy="protocol message -> local message item",
                    reason="protocol_target_not_bound",
                ),
                await message_items.context_menu_probe(),
                await ImagePreviewComponent(page).probe(selected),
                await ImagePreviewComponent(page).preview_probe(),
            ]
        )
    elif shape["url_path"].startswith(("/video/", "/note/")):
        panel = VideoCommentsPanelComponent()
        component_reports.extend(
            [
                await TutorialDialogComponent().probe(page),
                await panel.probe(page, True),
            ]
        )
        composer_component = VideoCommentComposerComponent(page)
        composer_visible = bool(
            await visible_candidates(composer_component.candidates())
        )
        comment_items = CommentItemComponent(page)
        list_visible = bool(await visible_candidates(comment_items.candidates()))
        component_reports.extend(
            [
                await composer_component.probe(composer_visible),
                await comment_items.probe(list_visible),
                not_exercised_probe(
                    component=comment_items.name,
                    version=comment_items.version,
                    capability="video.comment.reply",
                    strategy="protocol comment -> local reply action",
                    reason="protocol_target_not_bound",
                ),
                not_exercised_probe(
                    component=comment_items.name,
                    version=comment_items.version,
                    capability="video.comment.like",
                    strategy="protocol comment -> local stats/like action",
                    reason="protocol_target_not_bound",
                ),
            ]
        )
    component_drift = [
        item for item in component_reports if item["status"] == "drifted"
    ]
    adapter_reports = [
        {
            "name": item["component"],
            "version": item["component_version"],
            "capability": item["capability"],
            "status": item["status"],
        }
        for item in component_reports
    ]
    record = {
        "schema": "doudou.page-doctor.v1",
        "completed_at": utc_now(),
        "profile_id": profile_paths(data_root, profile).profile_id,
        "ready": ready,
        "page": shape,
        "exercise": exercise or None,
        "exercise_result": exercise_result,
        "adapters": adapter_reports,
        "components": component_reports,
        "adapter_status": "drifted" if component_drift else "compatible",
        "adapter_findings": component_drift,
        "not_exercised": [
            item for item in component_reports if item["status"] == "not_exercised"
        ],
        "adapter_error_code": "PAGE_ADAPTER_DRIFTED"
        if component_drift
        else None,
        "script_count": len(scripts),
        "script_set_sha256": script_set_sha256,
        "previous_script_set_sha256": previous.get("script_set_sha256")
        if isinstance(previous, dict)
        else None,
        "bundle_changed": bool(
            previous and previous.get("script_set_sha256") != script_set_sha256
        ),
        "im": {
            "request_bytes": len(browser.init_request or b""),
            "response_bytes": len(browser.init_response or b""),
        },
        "react_fiber_traversal": False,
    }
    directory = report_root / random_id("doctor")
    atomic_write_json(directory / "report.json", record)
    stage_path.unlink(missing_ok=True)
    return {
        "status": "adapter_drifted" if component_drift else "ok",
        "report": str((directory / "report.json").relative_to(data_root)),
        **record,
    }


async def protocol_capture(
    browser: BrowserSession, data_root: Path, profile: str, timeout_seconds: float
) -> dict[str, Any]:
    await browser.set_visible(True)
    await browser.ensure_ready()
    capture = NetworkCapture(browser.page, ("/v1/message/send",))
    capture.start()
    try:
        deadline = asyncio.get_running_loop().time() + timeout_seconds
        while (
            asyncio.get_running_loop().time() < deadline and not capture.response_bodies
        ):
            await browser.page.wait_for_timeout(250)
    finally:
        await capture.stop()
        await browser.set_visible(False)
    send_requests = [
        body for path, body in capture.request_bodies if "/v1/message/send" in path
    ]
    send_responses = [
        body for path, body in capture.response_bodies if "/v1/message/send" in path
    ]
    if len(send_requests) != 1 or len(send_responses) != 1:
        raise DoudouError(
            "CAPTURE_INCOMPLETE",
            "Capture did not observe exactly one complete normal page send.",
            {
                "request_count": len(send_requests),
                "response_count": len(send_responses),
            },
        )
    directory = (
        profile_paths(data_root, profile).diagnostics
        / "captures"
        / random_id("capture")
    )
    atomic_write_bytes(directory / "request.bin", send_requests[0])
    atomic_write_bytes(directory / "response.bin", send_responses[0])
    manifest = {
        "schema": "doudou.protocol-capture.v1",
        "completed_at": utc_now(),
        "complete": True,
        "request": {
            "file": "request.bin",
            "bytes": len(send_requests[0]),
            "sha256": sha256(send_requests[0]),
        },
        "response": {
            "file": "response.bin",
            "bytes": len(send_responses[0]),
            "sha256": sha256(send_responses[0]),
        },
        "network": {"requests": capture.requests, "responses": capture.responses},
    }
    atomic_write_json(directory / "manifest.json", manifest)
    return {
        "status": "captured",
        "directory": str(directory.relative_to(data_root)),
        "manifest": manifest,
    }
