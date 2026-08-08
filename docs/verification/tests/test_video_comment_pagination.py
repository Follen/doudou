from __future__ import annotations

import asyncio
import unittest
from types import SimpleNamespace
from unittest.mock import AsyncMock, patch

from doudou.common.errors import DoudouError
from doudou.reads.video import _collect_reply_thread, _resolve_reply_template
from doudou.ui.components.video import VideoCommentsPanelComponent


class ReplyPaginationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        self.session = SimpleNamespace(
            page=SimpleNamespace(wait_for_timeout=AsyncMock())
        )
        self.template = (
            "https://www.douyin.com/aweme/v1/web/comment/list/"
            "?aweme_id=video-1&cursor=0&count=10"
        )
        self.target = {"cid": "root-1", "reported": 17}

    async def test_collects_every_reply_page_until_has_more_is_false(self) -> None:
        responses = [
            {
                "comments": [
                    {"cid": "reply-1", "reply_id": "root-1", "text": "one"}
                ],
                "cursor": 10,
                "has_more": True,
            },
            {
                "comments": [
                    {"cid": "reply-2", "reply_id": "root-1", "text": "two"}
                ],
                "cursor": 20,
                "has_more": False,
            },
        ]
        with patch(
            "doudou.reads.video.fetch_json", new=AsyncMock(side_effect=responses)
        ) as fetch:
            result = await _collect_reply_thread(
                self.session, self.template, "video-1", self.target, max_pages=5
            )

        self.assertEqual([item["cid"] for item in result["comments"]], ["reply-1", "reply-2"])
        self.assertEqual(
            [(item["cursor_in"], item["cursor_out"], item["has_more"]) for item in result["pages"]],
            [(0, 10, True), (10, 20, False)],
        )
        self.assertEqual(fetch.await_count, 2)

    async def test_rejects_a_reply_cursor_that_does_not_advance(self) -> None:
        response = {"comments": [], "cursor": 0, "has_more": True}
        with patch(
            "doudou.reads.video.fetch_json", new=AsyncMock(return_value=response)
        ), self.assertRaises(DoudouError) as raised:
            await _collect_reply_thread(
                self.session, self.template, "video-1", self.target, max_pages=5
            )

        self.assertEqual(raised.exception.code, "COMMENT_CURSOR_STALLED")

    async def test_rejects_a_reply_thread_that_reaches_the_page_limit(self) -> None:
        response = {"comments": [], "cursor": 10, "has_more": True}
        with patch(
            "doudou.reads.video.fetch_json", new=AsyncMock(return_value=response)
        ), self.assertRaises(DoudouError) as raised:
            await _collect_reply_thread(
                self.session, self.template, "video-1", self.target, max_pages=1
            )

        self.assertEqual(raised.exception.code, "COMMENT_PAGE_LIMIT")

    async def test_reply_template_falls_back_after_bounded_network_timeouts(self) -> None:
        queue: asyncio.Queue[str] = asyncio.Queue()
        with patch.object(
            VideoCommentsPanelComponent,
            "trigger_reply_template",
            new=AsyncMock(return_value=1),
        ) as trigger:
            template = await _resolve_reply_template(
                self.session,
                self.template,
                queue,
                max_attempts=2,
                timeout_seconds=0,
            )

        self.assertEqual(template, self.template)
        self.assertEqual(trigger.await_count, 2)
        self.session.page.wait_for_timeout.assert_awaited_once_with(150)


if __name__ == "__main__":
    unittest.main()
