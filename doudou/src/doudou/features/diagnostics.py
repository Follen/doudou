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
from ..ui.adapters import ADAPTERS, CHAT_ADAPTER, VIDEO_ADAPTER
from ..ui.common import NetworkCapture
from ..ui.components import (
    ChatComposerComponent,
    CommentItemComponent,
    ImagePreviewComponent,
    MessageItemComponent,
    VideoCommentComposerComponent,
    VideoCommentsPanelComponent,
)


async def _visible_count(adapter: Any, page: Any, key: str, limit: int = 100) -> int:
    locator = adapter.locator(page, key)
    visible = 0
    for index in range(min(await locator.count(), limit)):
        if await locator.nth(index).is_visible():
            visible += 1
    return visible


async def page_doctor(
    browser: BrowserSession, data_root: Path, profile: str
) -> dict[str, Any]:
    ready = await browser.ensure_ready()
    page = browser.page
    shape = await page.evaluate(
        """() => ({
          url_path: location.pathname,
          scripts: [...document.scripts].map(item => item.src).filter(Boolean).slice(0, 300).map(value => { const url = new URL(value); return {origin: url.origin, path: url.pathname}; }),
        })"""
    )
    scripts = sorted(
        {f"{item['origin']}{item['path']}" for item in shape.pop("scripts")}
    )
    script_set_sha256 = sha256("\n".join(scripts))
    report_root = profile_paths(data_root, profile).diagnostics / "page-doctor"
    previous_files = sorted(
        report_root.glob("*/report.json"),
        key=lambda item: item.stat().st_mtime_ns,
        reverse=True,
    )
    previous = read_json(previous_files[0], optional=True) if previous_files else None
    adapter_reports: list[dict[str, Any]] = []
    visible_counts: dict[str, dict[str, int]] = {}
    for adapter in ADAPTERS:
        counts = {
            key: await _visible_count(adapter, page, key) for key in adapter.selectors
        }
        visible_counts[adapter.name] = counts
        adapter_reports.append(
            {
                "name": adapter.name,
                "version": adapter.version,
                "route_prefix": adapter.route_prefix,
                "route_aliases": list(adapter.route_aliases),
                "control_counts": counts,
            }
        )
    required: dict[str, tuple[Any, str]] = {}
    not_exercised: list[dict[str, str]] = []
    if CHAT_ADAPTER.matches_route(shape["url_path"]):
        chat_counts = visible_counts[CHAT_ADAPTER.name]
        if chat_counts["composer"] or chat_counts["message_item"]:
            required["chat.compose"] = (CHAT_ADAPTER, "composer")
        else:
            not_exercised.append(
                {"capability": "chat.compose", "reason": "conversation_not_selected"}
            )
    if VIDEO_ADAPTER.matches_route(shape["url_path"]):
        required["video.comments.open"] = (VIDEO_ADAPTER, "comment_button")
    drift = [
        {
            "capability": capability,
            "adapter": adapter.name,
            "adapter_version": adapter.version,
            "missing_control": key,
        }
        for capability, (adapter, key) in required.items()
        if visible_counts[adapter.name][key] == 0
    ]
    component_reports: list[dict[str, Any]] = []
    if CHAT_ADAPTER.matches_route(shape["url_path"]):
        composer = ChatComposerComponent(page)
        selected = bool(await _visible_count(CHAT_ADAPTER, page, "composer"))
        component_reports.extend(
            [
                await composer.probe(selected),
                await MessageItemComponent(page).probe(selected),
                await ImagePreviewComponent(page).probe(selected),
            ]
        )
    if VIDEO_ADAPTER.matches_route(shape["url_path"]):
        panel = VideoCommentsPanelComponent()
        composer_visible = bool(
            await _visible_count(VIDEO_ADAPTER, page, "comment_container")
        )
        list_visible = bool(
            await page.locator(CommentItemComponent.list_selector).count()
        )
        component_reports.extend(
            [
                await panel.probe(page, True),
                await VideoCommentComposerComponent(page).probe(composer_visible),
                await CommentItemComponent(page).probe(list_visible),
            ]
        )
    component_drift = [
        item for item in component_reports if item["status"] == "drifted"
    ]
    record = {
        "schema": "doudou.page-doctor.v1",
        "completed_at": utc_now(),
        "profile_id": profile_paths(data_root, profile).profile_id,
        "ready": ready,
        "page": shape,
        "adapters": adapter_reports,
        "components": component_reports,
        "adapter_status": "drifted" if drift or component_drift else "compatible",
        "adapter_findings": [*drift, *component_drift],
        "not_exercised": not_exercised,
        "adapter_error_code": "PAGE_ADAPTER_DRIFTED"
        if drift or component_drift
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
    return {
        "status": "adapter_drifted" if drift or component_drift else "ok",
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
