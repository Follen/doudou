from __future__ import annotations

import contextlib
import re
from typing import Any

from ...common.errors import DoudouError
from ...runtime.browser.session import BrowserSession
from ..common import first_visible
from .base import component_probe, visible_candidates

EMOJI_TOKEN = re.compile(r"\[[^\[\]\r\n]{1,24}\]")


def visible_comment_text(protocol_text: str) -> str:
    return EMOJI_TOKEN.sub("", protocol_text).strip()


class VideoCommentsPanelComponent:
    name = "video-comments-panel"
    version = "2026-08-02.1"
    comment_button_selector = '[data-e2e="feed-comment-icon"], [data-e2e*="comment" i], button[aria-label*="评论"], [title*="评论"]'

    def candidates(self, page: Any) -> Any:
        return page.locator(self.comment_button_selector)

    async def probe(self, page: Any, exercised: bool) -> dict[str, Any]:
        return await component_probe(
            self.candidates(page),
            component=self.name,
            version=self.version,
            capability="video.comments.open",
            strategy="semantic comment data-e2e/aria/title",
            exercised=exercised,
        )

    async def open(self, session: BrowserSession, aweme_id: str) -> tuple[Any, int]:
        stage = "ensure_ready"
        try:
            await session.ensure_ready()
            stage = "open_content"
            await session.open(f"https://www.douyin.com/video/{aweme_id}")
        except DoudouError:
            raise
        except Exception as error:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The video page did not become ready.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "video.comments.open",
                    "stage": stage,
                    "cause_type": type(error).__name__,
                },
            ) from error
        path = session.page.url.split("?", 1)[0].rstrip("/")
        if not path.endswith(f"/{aweme_id}") or not any(
            marker in path for marker in ("/video/", "/note/")
        ):
            raise DoudouError(
                "VIDEO_DETAIL_MISMATCH",
                "The page content does not match the confirmed plan.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "video.comments.open",
                },
            )
        await session.page.wait_for_timeout(3000)
        operations = 0
        tutorial = await first_visible(session.page.get_by_text("我知道了", exact=True))
        if tutorial:
            await tutorial.click()
            operations += 1
        operations += await self.ensure_open(session.page)
        return session.page, operations

    async def ensure_open(self, page: Any) -> int:
        panel_candidates = page.locator(
            '[data-e2e="comment-list"], .comment-input-container'
        )
        if await first_visible(panel_candidates):
            return 0
        button = await first_visible(page.locator(self.comment_button_selector))
        if not button or not await button.is_enabled():
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The video comment panel control is unavailable.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "video.comments.open",
                    "strategy": "semantic comment data-e2e/aria/title",
                },
            )
        try:
            await button.click(timeout=15_000)
            for _ in range(200):
                if await first_visible(panel_candidates):
                    break
                await page.wait_for_timeout(100)
            else:
                raise TimeoutError("no visible comment panel component")
        except Exception as error:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The video comments panel did not become ready.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "video.comments.open",
                    "stage": "open_comment_panel",
                    "cause_type": type(error).__name__,
                },
            ) from error
        return 1

    async def expand_any_replies(self, page: Any) -> int:
        try:
            for attempt in range(30):
                lists = await visible_candidates(
                    page.locator('[data-e2e="comment-list"]')
                )
                if len(lists) != 1:
                    raise DoudouError(
                        "PAGE_ADAPTER_DRIFTED",
                        "The video comment list is not uniquely visible.",
                        {
                            "component": self.name,
                            "component_version": self.version,
                            "capability": "video.comment.replies.read",
                            "candidate_count": len(lists),
                        },
                    )
                controls = await visible_candidates(
                    lists[0].get_by_text(
                        re.compile(r"(?:展开|查看|共).{0,12}条回复")
                    )
                )
                if controls:
                    await controls[0].click(timeout=15_000)
                    return attempt + 1
                await lists[0].hover()
                await page.mouse.wheel(0, 700)
                await page.wait_for_timeout(300)
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "No visible reply expansion control was found by bounded scrolling.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "video.comment.replies.read",
                    "attempts": 30,
                },
            )
        except DoudouError:
            raise
        except Exception as error:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The reply protocol discovery control became unavailable.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "video.comment.replies.read",
                    "cause_type": type(error).__name__,
                },
            ) from error


class VideoCommentComposerComponent:
    name = "video-comment-composer"
    version = "2026-08-02.1"
    container_selector = ".comment-input-container"
    editor_selector = '[contenteditable="true"][role="combobox"], textarea, input'
    controls_selector = ".commentInput-right-ct span"

    focused_editor_selector = '[contenteditable="true"][role="combobox"]:focus'
    focused_container_xpath = (
        "xpath=ancestor::div[contains(@class,'comment-input-container')][1]"
    )

    def __init__(self, page: Any, focused: bool = False) -> None:
        self.page = page
        self.focused = focused

    def candidates(self) -> Any:
        if self.focused:
            return self.page.locator(self.focused_editor_selector).locator(
                self.focused_container_xpath
            )
        return self.page.locator(self.container_selector)

    async def container(self) -> Any:
        containers = await visible_candidates(self.candidates())
        if len(containers) != 1:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The video comment composer is not unique.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "video.comment.compose",
                    "strategy": "visible comment input container",
                    "candidate_count": len(containers),
                },
            )
        return containers[0]

    async def editor(self) -> Any:
        if self.focused:
            with contextlib.suppress(Exception):
                await self.page.locator(self.focused_editor_selector).first.wait_for(
                    state="visible", timeout=3000
                )
            editors = await visible_candidates(
                self.page.locator(self.focused_editor_selector)
            )
            if len(editors) != 1:
                raise DoudouError(
                    "PAGE_ADAPTER_DRIFTED",
                    "The focused video comment editor is not unique.",
                    {
                        "component": self.name,
                        "component_version": self.version,
                        "capability": "video.comment.reply",
                        "strategy": "focused Draft combobox",
                        "candidate_count": len(editors),
                    },
                )
            return editors[0]
        container = await self.container()
        editors = await visible_candidates(container.locator(self.editor_selector))
        if len(editors) != 1:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The video comment editor is not unique.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "video.comment.compose",
                    "strategy": "composer-local Draft combobox",
                    "candidate_count": len(editors),
                },
            )
        return editors[0]

    async def insert_text(self, text: str) -> Any:
        editor = await self.editor()
        current = await editor.inner_text()
        if current.strip("\u200b\u200c\u200d\ufeff\r\n "):
            raise DoudouError(
                "COMMENT_EDITOR_NOT_EMPTY",
                "The comment editor contains unplanned content.",
            )
        await editor.click()
        await self.page.keyboard.insert_text(text)
        await self.page.wait_for_timeout(250)
        return editor

    async def send_control(self) -> Any:
        container = await self.container()
        controls = await visible_candidates(container.locator(self.controls_selector))
        boxes = [(item, await item.bounding_box()) for item in controls]
        boxes = [(item, box) for item, box in boxes if box]
        if len(boxes) < 2:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The video comment actions are unavailable.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "video.comment.send",
                    "strategy": "composer-local direct action spans",
                    "candidate_count": len(boxes),
                },
            )
        rightmost_x = max(box["x"] for _, box in boxes)
        rightmost = [item for item, box in boxes if box["x"] == rightmost_x]
        if len(rightmost) != 1 or not await rightmost[0].is_enabled():
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The video comment send action is not uniquely enabled.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "video.comment.send",
                    "strategy": "rightmost composer-local action",
                    "candidate_count": len(rightmost),
                },
            )
        return rightmost[0]

    async def probe(self, exercised: bool) -> dict[str, Any]:
        return await component_probe(
            self.candidates(),
            component=self.name,
            version=self.version,
            capability="video.comment.compose",
            strategy="visible local comment input container",
            exercised=exercised,
        )


class CommentItemComponent:
    name = "video-comment-item"
    version = "2026-08-02.1"
    list_selector = '[data-e2e="comment-list"]'
    item_xpath = "xpath=ancestor::div[.//div[contains(@class,'comment-item-stats-container')] and .//span[normalize-space()='回复']][1]"
    stats_selector = ".comment-item-stats-container"

    def __init__(self, page: Any) -> None:
        self.page = page

    def candidates(self) -> Any:
        return self.page.locator(self.list_selector)

    async def probe(self, exercised: bool) -> dict[str, Any]:
        return await component_probe(
            self.candidates(),
            component=self.name,
            version=self.version,
            capability="video.comment.target",
            strategy="data-e2e comment-list plus local item structure",
            exercised=exercised,
        )

    async def _list(self) -> Any:
        lists = await visible_candidates(self.page.locator(self.list_selector))
        if len(lists) != 1:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The video comment list is not uniquely visible.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "video.comment.target",
                    "strategy": "data-e2e comment-list",
                    "candidate_count": len(lists),
                },
            )
        return lists[0]

    async def target(
        self, text: str, author_name: str | None = None, attempts: int = 30
    ) -> Any:
        root = await self._list()
        visible_text = visible_comment_text(text)
        with contextlib.suppress(Exception):
            await root.get_by_text(visible_text, exact=True).first.wait_for(
                state="visible", timeout=15_000
            )
        for _ in range(attempts):
            matches = root.get_by_text(visible_text, exact=True)
            items: list[Any] = []
            for candidate in await visible_candidates(matches):
                item = candidate.locator(self.item_xpath)
                if await item.count() == 1 and await item.is_visible():
                    if (
                        author_name
                        and not await item.get_by_text(author_name, exact=True).count()
                    ):
                        continue
                    items.append(item)
            if len(items) == 1:
                return items[0]
            if len(items) > 1:
                raise DoudouError(
                    "PAGE_ADAPTER_DRIFTED",
                    "The protocol-selected comment is ambiguous.",
                    {
                        "component": self.name,
                        "component_version": self.version,
                        "capability": "video.comment.target",
                        "strategy": "comment-list text plus author",
                        "candidate_count": len(items),
                    },
                )
            moved = await root.evaluate(
                "element => { const scroller = [element, ...element.querySelectorAll('div')].find(node => node.scrollHeight > node.clientHeight + 20); if (!scroller) return false; const before=scroller.scrollTop; scroller.scrollBy(0, Math.max(500, scroller.clientHeight * 0.8)); return scroller.scrollTop !== before; }"
            )
            if not moved:
                break
            await self.page.wait_for_timeout(350)
        raise DoudouError(
            "COMMENT_TARGET_NOT_VISIBLE",
            "The protocol-selected comment was not found by bounded scrolling.",
            {
                "component": self.name,
                "component_version": self.version,
                "capability": "video.comment.target",
                "strategy": "comment-list bounded scroll plus exact normalized text",
                "attempts": attempts,
            },
        )

    async def reply(self, item: Any) -> int:
        action = await first_visible(item.get_by_text("回复", exact=True))
        if not action or not await action.is_enabled():
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The comment reply action is unavailable.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "video.comment.reply",
                    "strategy": "comment-item-local semantic reply",
                },
            )
        await action.click()
        return 1

    async def like_control(self, item: Any, count: int) -> Any:
        stats = await first_visible(item.locator(self.stats_selector))
        if not stats:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The comment stats component is unavailable.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "video.comment.like",
                    "strategy": "comment-item-local stats container",
                },
            )
        counters = await visible_candidates(stats.get_by_text(str(count), exact=True))
        if len(counters) != 1:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The comment like control is not unique.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "video.comment.like",
                    "strategy": "stats-local exact count",
                    "candidate_count": len(counters),
                },
            )
        return counters[0]
