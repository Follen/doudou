from __future__ import annotations

import asyncio
import contextlib
from pathlib import Path
from typing import Any
from urllib.parse import parse_qsl, quote, urlencode, urlsplit, urlunsplit

from ..common.errors import DoudouError
from ..protocol.contracts import WEB_PROTOCOL
from ..protocol.responses import ResponseCodes
from ..protocol.web import (
    identities_in,
    normalize_comment,
    normalize_video,
    normalize_video_detail,
    parse_render_data,
    resolve_comment_parents,
)
from ..runtime.browser.session import BrowserSession
from ..storage.content import (
    read_comment_checkpoint,
    write_comment_checkpoint,
    write_identity,
    write_video,
    write_video_index,
)
from ..ui.adapters import PROFILE_ADAPTER, VIDEO_ADAPTER
from ..ui.components import VideoCommentsPanelComponent
from .transport import fetch_json, parse_result


async def _first_visible(locator: Any) -> Any | None:
    for index in range(await locator.count()):
        candidate = locator.nth(index)
        if await candidate.is_visible():
            return candidate
    return None


async def _bounded_control_inventory(
    page: Any, limit: int = 80
) -> list[dict[str, Any]]:
    controls = VIDEO_ADAPTER.locator(page, "inventory_controls")
    output: list[dict[str, Any]] = []
    for index in range(min(await controls.count(), limit)):
        control = controls.nth(index)
        with contextlib.suppress(Exception):
            if not await control.is_visible():
                continue
            output.append(
                {
                    "data_e2e": await control.get_attribute("data-e2e"),
                    "aria_label": await control.get_attribute("aria-label"),
                    "title": await control.get_attribute("title"),
                    "text": (await control.inner_text())[:120],
                }
            )
    return output


COMMENT_CODES = ResponseCodes(
    empty="COMMENT_RESPONSE_EMPTY",
    invalid="COMMENT_RESPONSE_INVALID",
    failed="COMMENT_COLLECT_FAILED",
)

VIDEO_LIST_CODES = ResponseCodes(
    "VIDEO_RESPONSE_EMPTY", "VIDEO_RESPONSE_INVALID", "VIDEO_COLLECT_FAILED"
)

VOLATILE_QUERY_KEYS = {"msToken", "a_bogus"}
REPLY_INCOMPATIBLE_QUERY_KEYS = {
    "aweme_id",
    "pc_img_format",
    "insert_ids",
    "whale_cut_token",
    "rcFT",
}


def _url_with_query(
    url: str, *, remove: set[str] | None = None, **changes: Any
) -> str:
    parsed = urlsplit(url)
    query = dict(parse_qsl(parsed.query, keep_blank_values=True))
    for key in VOLATILE_QUERY_KEYS | (remove or set()):
        query.pop(key, None)
    query.update({key: str(value) for key, value in changes.items()})
    return urlunsplit(
        (parsed.scheme, parsed.netloc, parsed.path, urlencode(query), parsed.fragment)
    )


def _reply_url(root_template: str, aweme_id: str, root_cid: str, cursor: Any) -> str:
    parsed = urlsplit(
        _url_with_query(
            root_template,
            remove=REPLY_INCOMPATIBLE_QUERY_KEYS,
            cursor=cursor,
            item_id=aweme_id,
            comment_id=root_cid,
            count=20,
            item_type=0,
        )
    )
    return urlunsplit(
        (
            parsed.scheme,
            parsed.netloc,
            WEB_PROTOCOL.path("comment_reply"),
            parsed.query,
            parsed.fragment,
        )
    )


async def _collect_reply_thread(
    session: BrowserSession,
    reply_template: str,
    aweme_id: str,
    target: dict[str, Any],
    max_pages: int,
) -> dict[str, Any]:
    cursor: Any = 0
    pages: list[dict[str, Any]] = []
    comments: list[dict[str, Any]] = []
    for page_number in range(1, max_pages + 1):
        value = await fetch_json(
            session,
            _reply_url(reply_template, aweme_id, target["cid"], cursor),
            codes=COMMENT_CODES,
            operation="comment.reply.list",
        )
        batch = WEB_PROTOCOL.list_value(value, "comments", "comment.reply.list")
        next_cursor = WEB_PROTOCOL.value(value, "comment_cursor")
        has_more = bool(WEB_PROTOCOL.value(value, "has_more"))
        comments.extend(normalize_comment(item, aweme_id) for item in batch)
        pages.append(
            {
                "page": page_number,
                "cursor_in": cursor,
                "cursor_out": next_cursor,
                "received": len(batch),
                "has_more": has_more,
            }
        )
        if not has_more:
            return {"cid": target["cid"], "pages": pages, "comments": comments}
        if str(next_cursor) == str(cursor):
            raise DoudouError(
                "COMMENT_CURSOR_STALLED",
                "Reply comment cursor did not advance.",
                {"root_cid": target["cid"]},
            )
        cursor = next_cursor
    raise DoudouError(
        "COMMENT_PAGE_LIMIT",
        "Reply comment collection reached the configured page limit.",
        {"root_cid": target["cid"], "max_pages": max_pages},
    )


async def resolve_identity(
    session: BrowserSession, data_root: Path, profile: str, douyin_id: str
) -> dict[str, Any]:
    await session.ensure_ready()
    captured: list[Any] = []
    captured_results: list[dict[str, Any]] = []
    tasks: set[asyncio.Task[Any]] = set()

    async def consume(response: Any) -> None:
        path = response.url.split("?", 1)[0].lower()
        if not any(
            part in path
            for part in ("search", "user/profile", "user/detail", "aweme/post")
        ):
            return
        try:
            captured_results.append(
                {
                    "http_status": response.status,
                    "content_type": response.headers.get("content-type"),
                    "retry_after": response.headers.get("retry-after"),
                    "text": await response.text(),
                }
            )
        except Exception:  # noqa: BLE001 - detached Playwright response callback
            return

    def on_response(response: Any) -> None:
        task = asyncio.create_task(consume(response))
        tasks.add(task)
        task.add_done_callback(tasks.discard)

    session.page.on("response", on_response)
    try:
        await session.open(
            f"https://www.douyin.com/search/{quote(douyin_id)}?type=user"
        )
        await session.page.wait_for_timeout(10_000)
        if tasks:
            await asyncio.gather(*tasks, return_exceptions=True)
        render = VIDEO_ADAPTER.locator(session.page, "render_data")
        if await render.count():
            try:
                captured.append(
                    parse_render_data(await render.text_content(timeout=2000) or "")
                )
            except Exception:  # noqa: BLE001 - SSR is an optional identity source
                captured.append({})
    finally:
        session.page.remove_listener("response", on_response)
    protocol_errors: list[DoudouError] = []
    for result in captured_results:
        try:
            captured.append(
                await parse_result(
                    session,
                    result,
                    codes=ResponseCodes(
                        "IDENTITY_RESPONSE_EMPTY",
                        "IDENTITY_RESPONSE_INVALID",
                        "IDENTITY_LOOKUP_FAILED",
                    ),
                    operation="identity.lookup",
                )
            )
        except DoudouError as error:
            protocol_errors.append(error)
    candidates: dict[tuple[str, str, str], dict[str, Any]] = {}
    for value in captured:
        for item in identities_in(value):
            candidates[(item["unique_id"], item["sec_uid"], item["uid"])] = item
    values = list(candidates.values())
    exact = [
        item
        for item in values
        if item["unique_id"].casefold() == douyin_id.casefold() and item["sec_uid"]
    ]
    if len(exact) != 1:
        blocking = next(
            (
                error
                for error in protocol_errors
                if error.code
                in {"LOGIN_REQUIRED", "VERIFICATION_REQUIRED", "RATE_LIMITED"}
            ),
            None,
        )
        if blocking:
            raise blocking
        code = "IDENTITY_AMBIGUOUS" if len(exact) > 1 else "IDENTITY_NOT_FOUND"
        raise DoudouError(
            code,
            "The Douyin ID did not resolve to one exact structured identity.",
            {
                "douyin_id": douyin_id,
                "candidates": values,
                "protocol_error_codes": sorted(
                    {error.code for error in protocol_errors}
                ),
            },
        )
    files = write_identity(data_root, profile, exact[0], values)
    return {
        "status": "resolved",
        "identity": exact[0],
        "candidates": values,
        "files": files,
    }


async def collect_user_videos(
    session: BrowserSession,
    data_root: Path,
    profile: str,
    identity: dict[str, Any],
    max_pages: int = 1000,
) -> dict[str, Any]:
    await session.ensure_ready()
    queue: asyncio.Queue[dict[str, Any]] = asyncio.Queue()
    tasks: set[asyncio.Task[Any]] = set()
    ignored_non_json = 0

    async def consume(response: Any) -> None:
        if WEB_PROTOCOL.path("video_list") not in response.url:
            return
        try:
            result = {
                "http_status": response.status,
                "content_type": response.headers.get("content-type"),
                "retry_after": response.headers.get("retry-after"),
                "text": await response.text(),
            }
        except Exception:  # noqa: BLE001 - detached Playwright response callback
            return
        await queue.put({"url": response.url, "result": result})

    def on_response(response: Any) -> None:
        task = asyncio.create_task(consume(response))
        tasks.add(task)
        task.add_done_callback(tasks.discard)

    session.page.on("response", on_response)
    async def next_protocol_page(timeout: float) -> tuple[str, dict[str, Any]]:
        nonlocal ignored_non_json
        deadline = asyncio.get_running_loop().time() + timeout
        last_invalid: DoudouError | None = None
        while True:
            remaining = deadline - asyncio.get_running_loop().time()
            if remaining <= 0:
                if last_invalid is not None:
                    raise last_invalid
                raise DoudouError(
                    "PROTOCOL_TRANSPORT_TIMEOUT",
                    "The profile page did not emit a video-list protocol response.",
                    {"operation": "video.list", "timeout_seconds": timeout},
                )
            item = await asyncio.wait_for(queue.get(), timeout=remaining)
            try:
                value = await parse_result(
                    session,
                    item["result"],
                    codes=VIDEO_LIST_CODES,
                    operation="video.list",
                )
                return item["url"], value
            except DoudouError as error:
                content_type = str(item["result"].get("content_type") or "")
                if error.code == "VIDEO_RESPONSE_INVALID" and "html" in content_type:
                    ignored_non_json += 1
                    last_invalid = error
                    continue
                raise

    videos: dict[str, dict[str, Any]] = {}
    pages: list[dict[str, Any]] = []
    try:
        await session.open(
            f"https://www.douyin.com/user/{identity['sec_uid']}?from_tab_name=main"
        )
        _template, value = await next_protocol_page(30)
        scroll = PROFILE_ADAPTER.locator(session.page, "scroll_container").first
        if not await scroll.count() or not await scroll.is_visible():
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The profile page scroll component is unavailable.",
                {
                    "adapter": PROFILE_ADAPTER.name,
                    "adapter_version": PROFILE_ADAPTER.version,
                    "capability": "video.list.paginate",
                },
            )
        cursor_in: Any = 0
        seen_cursors: set[str] = set()
        for page_number in range(1, max_pages + 1):
            raw_items = WEB_PROTOCOL.list_value(value, "video_items", "video.list")
            added = 0
            for raw in raw_items:
                item = normalize_video(raw)
                if item["aweme_id"] and item["aweme_id"] not in videos:
                    videos[item["aweme_id"]] = item
                    added += 1
            cursor_out = WEB_PROTOCOL.value(value, "video_cursor")
            has_more = bool(WEB_PROTOCOL.value(value, "has_more"))
            pages.append(
                {
                    "page": page_number,
                    "cursor_in": cursor_in,
                    "cursor_out": cursor_out,
                    "received": len(raw_items),
                    "added": added,
                    "has_more": has_more,
                }
            )
            if not has_more:
                break
            if cursor_out is None or str(cursor_out) in seen_cursors:
                raise DoudouError(
                    "VIDEO_CURSOR_STALLED",
                    "Video pagination cursor did not advance.",
                    {"cursor": cursor_out},
                )
            seen_cursors.add(str(cursor_out))
            cursor_in = cursor_out
            for attempt in range(1, 6):
                await scroll.evaluate(
                    "element => { element.scrollTop = element.scrollHeight; }"
                )
                try:
                    _template, value = await next_protocol_page(4)
                    break
                except DoudouError as error:
                    if error.code != "PROTOCOL_TRANSPORT_TIMEOUT" or attempt == 5:
                        raise
                    await session.page.wait_for_timeout(300)
        else:
            raise DoudouError(
                "VIDEO_PAGE_LIMIT",
                "Video collection reached the configured page limit.",
                {"max_pages": max_pages},
            )
    finally:
        session.page.remove_listener("response", on_response)
    ordered = sorted(
        videos.values(),
        key=lambda item: (-(item["create_time"] or 0), item["aweme_id"]),
    )
    files = write_video_index(data_root, profile, identity, ordered, pages)
    return {
        "status": "collected",
        "identity": identity,
        "count": len(ordered),
        "pages": pages,
        "pagination_trigger": "profile_component_scroll",
        "ignored_non_json_responses": ignored_non_json,
        "files": files,
    }


async def collect_video(
    session: BrowserSession,
    data_root: Path,
    profile: str,
    sec_uid: str,
    aweme_id: str,
    max_pages: int = 1000,
) -> dict[str, Any]:
    await session.ensure_ready()
    root_urls: asyncio.Queue[str] = asyncio.Queue()
    reply_urls: asyncio.Queue[str] = asyncio.Queue()
    detail_values: asyncio.Queue[dict[str, Any]] = asyncio.Queue()
    response_tasks: set[asyncio.Task[Any]] = set()

    def on_request(request: Any) -> None:
        if WEB_PROTOCOL.path("comment_reply") in request.url:
            reply_urls.put_nowait(request.url)
        elif WEB_PROTOCOL.path("comment_root") in request.url:
            root_urls.put_nowait(request.url)

    async def consume_response(response: Any) -> None:
        if WEB_PROTOCOL.path("video_detail") not in response.url:
            return
        with contextlib.suppress(Exception):
            await detail_values.put(
                {
                    "http_status": response.status,
                    "content_type": response.headers.get("content-type"),
                    "retry_after": response.headers.get("retry-after"),
                    "text": await response.text(),
                }
            )

    def on_response(response: Any) -> None:
        task = asyncio.create_task(consume_response(response))
        response_tasks.add(task)
        task.add_done_callback(response_tasks.discard)

    session.page.on("request", on_request)
    session.page.on("response", on_response)
    try:
        source_url = f"https://www.douyin.com/video/{aweme_id}"
        await session.open(source_url)
        render_data: dict[str, Any] = {}
        render = VIDEO_ADAPTER.locator(session.page, "render_data")
        if await render.count():
            with contextlib.suppress(Exception):
                render_data = parse_render_data(
                    await render.text_content(timeout=3000) or ""
                )
        app_data = render_data.get("app") or {}
        detail = app_data.get("videoDetail") or {}
        if not detail:
            detail_response = await asyncio.wait_for(detail_values.get(), timeout=20)
            detail_value = await parse_result(
                session,
                detail_response,
                codes=ResponseCodes(
                    "VIDEO_DETAIL_RESPONSE_EMPTY",
                    "VIDEO_DETAIL_RESPONSE_INVALID",
                    "VIDEO_DETAIL_FAILED",
                ),
                operation="video.detail",
            )
            detail = WEB_PROTOCOL.value(detail_value, "video_detail", {}) or {}
        normalized = normalize_video_detail(detail)
        if normalized["aweme_id"] != aweme_id:
            raise DoudouError(
                "VIDEO_DETAIL_MISMATCH",
                "SSR detail does not match the requested video.",
                {
                    "expected": aweme_id,
                    "actual": normalized["aweme_id"],
                    "page_url": session.page.url.split("?", 1)[0],
                    "render_keys": sorted(render_data),
                    "app_keys": sorted(app_data) if isinstance(app_data, dict) else [],
                },
            )
        await session.page.wait_for_timeout(3000)
        tutorial = await _first_visible(VIDEO_ADAPTER.text(session.page, "tutorial"))
        if tutorial:
            await tutorial.click()
        try:
            root_template = root_urls.get_nowait()
        except asyncio.QueueEmpty:
            comment_button = await _first_visible(
                VIDEO_ADAPTER.locator(session.page, "comment_button")
            )
            if not comment_button:
                comment_count = str(
                    (detail.get("stats") or {}).get("commentCount") or ""
                )
                comment_button = await _first_visible(
                    session.page.get_by_text(comment_count, exact=True)
                )
            if not comment_button:
                raise DoudouError(
                    "COMMENT_PANEL_MISSING",
                    "The bounded comment panel control is unavailable.",
                    {
                        "visible_controls": await _bounded_control_inventory(
                            session.page
                        )
                    },
                )
            await comment_button.click(timeout=15_000)
            root_template = await asyncio.wait_for(root_urls.get(), timeout=20)
        checkpoint, comments = read_comment_checkpoint(data_root, profile, aweme_id)
        if not checkpoint or checkpoint.get("status") == "completed":
            checkpoint, comments = {}, []
        pages = list(checkpoint.get("pages") or [])
        embedded_reply_counts = {
            str(key): int(value)
            for key, value in (checkpoint.get("embedded_reply_counts") or {}).items()
        }
        completed_reply_root_ids = {
            str(value) for value in checkpoint.get("completed_reply_root_ids") or []
        }
        completed_root_pages = int(checkpoint.get("completed_root_pages") or 0)
        root_complete = bool(checkpoint.get("root_complete"))
        cursor: Any = checkpoint.get("root_cursor", 0)

        def persist(
            status: str,
            *,
            next_reply_root_id: str | None = None,
            error: DoudouError | None = None,
        ) -> None:
            record: dict[str, Any] = {
                "status": status,
                "root_cursor": cursor,
                "root_complete": root_complete,
                "completed_root_pages": completed_root_pages,
                "completed_reply_root_ids": sorted(completed_reply_root_ids),
                "next_reply_root_id": next_reply_root_id,
                "embedded_reply_counts": embedded_reply_counts,
                "pages": pages,
            }
            if error is not None:
                record["last_error"] = {"code": error.code, "details": error.details}
            write_comment_checkpoint(data_root, profile, aweme_id, record, comments)

        try:
            if not root_complete:
                for page_number in range(completed_root_pages + 1, max_pages + 1):
                    value = await fetch_json(
                        session,
                        _url_with_query(root_template, cursor=cursor),
                        codes=COMMENT_CODES,
                        operation="comment.root.list",
                    )
                    raw_roots = WEB_PROTOCOL.list_value(
                        value, "comments", "comment.root.list"
                    )
                    batch = [normalize_comment(item, aweme_id) for item in raw_roots]
                    comments.extend(batch)
                    for raw_root, root in zip(raw_roots, batch, strict=True):
                        embedded = (
                            WEB_PROTOCOL.value(raw_root, "embedded_replies", []) or []
                        )
                        embedded_reply_counts[root["cid"]] = len(embedded)
                        comments.extend(
                            normalize_comment(item, aweme_id) for item in embedded
                        )
                    next_cursor = WEB_PROTOCOL.value(value, "comment_cursor")
                    has_more = bool(WEB_PROTOCOL.value(value, "has_more"))
                    pages.append(
                        {
                            "kind": "root",
                            "page": page_number,
                            "cursor_in": cursor,
                            "cursor_out": next_cursor,
                            "received": len(batch),
                            "has_more": has_more,
                        }
                    )
                    completed_root_pages = page_number
                    root_complete = not has_more
                    if has_more and str(next_cursor) == str(cursor):
                        raise DoudouError(
                            "COMMENT_CURSOR_STALLED",
                            "Root comment cursor did not advance.",
                        )
                    cursor = next_cursor
                    persist("running")
                    if root_complete:
                        break
                else:
                    raise DoudouError(
                        "COMMENT_PAGE_LIMIT",
                        "Root comment collection reached the configured page limit.",
                        {"max_pages": max_pages},
                    )

            roots = [
                item for item in comments if item.get("cid") == item.get("reply_id")
            ]
            reply_targets = [
                {
                    "cid": root["cid"],
                    "reported": int(root.get("reported_reply_count") or 0),
                }
                for root in roots
                if int(root.get("reported_reply_count") or 0)
                > embedded_reply_counts.get(root["cid"], 0)
                and root["cid"] not in completed_reply_root_ids
            ]
            reply_template: str | None = None
            if reply_targets:
                panel = VideoCommentsPanelComponent()
                await panel.ensure_open(session.page)
                await panel.expand_any_replies(session.page)
                reply_template = await asyncio.wait_for(reply_urls.get(), timeout=20)
            semaphore = asyncio.Semaphore(4)

            async def collect_target(target: dict[str, Any]) -> dict[str, Any]:
                async with semaphore:
                    try:
                        return await _collect_reply_thread(
                            session,
                            reply_template or root_template,
                            aweme_id,
                            target,
                            max_pages,
                        )
                    except DoudouError as error:
                        error.details = {
                            "root_cid": target["cid"],
                            **(error.details or {}),
                        }
                        raise

            tasks = [
                asyncio.create_task(collect_target(target)) for target in reply_targets
            ]
            try:
                for completed in asyncio.as_completed(tasks):
                    thread = await completed
                    comments.extend(thread.get("comments") or [])
                    pages.extend(
                        {"kind": "reply", "root_cid": thread["cid"], **item}
                        for item in thread.get("pages") or []
                    )
                    completed_reply_root_ids.add(thread["cid"])
                    pending = next(
                        (
                            target["cid"]
                            for target in reply_targets
                            if target["cid"] not in completed_reply_root_ids
                        ),
                        None,
                    )
                    persist("running", next_reply_root_id=pending)
            except Exception:
                for task in tasks:
                    task.cancel()
                await asyncio.gather(*tasks, return_exceptions=True)
                raise
        except DoudouError as error:
            persist("failed", error=error)
            raise
        deduplicated = {item["cid"]: item for item in comments if item.get("cid")}
        comments = resolve_comment_parents(list(deduplicated.values()))
        cover = None
        if normalized.get("cover_url"):
            response = await session.context.request.get(
                normalized["cover_url"], headers={"referer": "https://www.douyin.com/"}
            )
            candidate = await response.body()
            if response.status == 200 and candidate.startswith(
                (b"\xff\xd8\xff", b"\x89PNG", b"RIFF")
            ):
                cover = candidate
        files = write_video(data_root, profile, normalized, cover, comments, pages)
        persist("completed")
        return {
            "status": "collected",
            "video": normalized,
            "comments": len(comments),
            "pages": pages,
            "files": files,
        }
    finally:
        session.page.remove_listener("request", on_request)
        session.page.remove_listener("response", on_response)
        if response_tasks:
            await asyncio.gather(*response_tasks, return_exceptions=True)
