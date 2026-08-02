from __future__ import annotations

import argparse
import asyncio
from pathlib import Path
from typing import Any

from ..features.chat import service as chat
from ..features.diagnostics import page_doctor, protocol_capture
from ..features.video import service as video
from ..reads.chat import refresh_session_index
from ..runtime.broker.server import serve
from ..runtime.browser.session import BrowserSession


async def dispatch(
    command: str,
    params: dict[str, Any],
    browser: BrowserSession,
    data_root: Path,
    profile: str,
) -> dict[str, Any] | None:
    if command == "session.refresh":
        if browser.fake:
            return await browser.ensure_ready(force_refresh=True)
        return await refresh_session_index(browser, data_root, profile, force=True)
    if command == "conversations.refresh":
        return await chat.conversations(
            browser,
            data_root,
            profile,
            force=bool(params.get("force")),
            kind=str(params.get("kind") or "all"),
        )
    if command == "messages.collect":
        return await chat.collect(browser, data_root, profile, params)
    if command == "chat.send.preview":
        return await chat.preview_send(browser, data_root, profile, params)
    if command == "chat.send.execute":
        return await chat.execute_send(browser, data_root, profile, params)
    if command == "chat.send.batch.preview":
        return await chat.preview_batch(browser, data_root, profile, params)
    if command == "chat.send.batch.execute":
        return await chat.execute_batch(browser, data_root, profile, params)
    if command == "chat.like.preview":
        return await chat.preview_like(browser, data_root, profile, params)
    if command == "chat.like.execute":
        return await chat.execute_like(browser, data_root, profile, params)
    if command == "identity.resolve":
        return await video.identity(browser, data_root, profile, params["douyin_id"])
    if command == "videos.collect":
        return await video.videos(browser, data_root, profile, params)
    if command == "video.collect":
        return await video.video(browser, data_root, profile, params)
    if command == "video.interaction.preview":
        return await video.preview_interaction(
            data_root, profile, params["operation"], params["payload"]
        )
    if command == "video.interaction.execute":
        return await video.execute_interaction(
            browser, data_root, profile, params["operation"], params
        )
    if command == "page.doctor":
        return await page_doctor(browser, data_root, profile, params)
    if command == "protocol.capture":
        return await protocol_capture(
            browser, data_root, profile, float(params.get("timeout_seconds") or 600)
        )
    return None


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--data-root", required=True, type=Path)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--idle-timeout", type=float, default=1200.0)
    parser.add_argument("--fake-browser", action="store_true")
    args = parser.parse_args(argv)
    asyncio.run(
        serve(
            args.data_root,
            args.profile,
            args.idle_timeout,
            args.fake_browser,
            handler=dispatch,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
