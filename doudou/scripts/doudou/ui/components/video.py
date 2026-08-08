from __future__ import annotations

import asyncio
import re
from typing import Any

from ...common.errors import DoudouError
from ...runtime.browser.session import BrowserSession
from ..adapters import PROFILE_ADAPTER, VIDEO_ADAPTER
from .base import (
    click_verified_target,
    component_probe,
    not_exercised_probe,
    require_target,
    resolution_probe,
    visible_candidates,
)

EMOJI_TOKEN = re.compile(r"\[[^\[\]\r\n]{1,24}\]")


def visible_comment_text(protocol_text: str) -> str:
    return EMOJI_TOKEN.sub("", protocol_text).strip()


class TutorialDialogComponent:
    name = "video-tutorial-dialog"
    version = "2026-08-02.1"

    async def dismiss(self, page: Any) -> int:
        labels = page.get_by_text(VIDEO_ADAPTER.labels["tutorial"], exact=True)
        visible = await visible_candidates(labels, limit=10)
        if not visible:
            return 0
        action = await self.action(page)
        await action.click(timeout=10_000)
        return 1

    async def action(self, page: Any) -> Any:
        labels = page.get_by_text(VIDEO_ADAPTER.labels["tutorial"], exact=True)
        action = await require_target(
            labels,
            component=self.name,
            version=self.version,
            capability="video.tutorial.dismiss",
            strategy="stable tutorial action label",
            require_enabled=True,
        )
        dialog = await require_target(
            action.locator(
                "xpath=ancestor::*[@role='dialog' or contains(@class,'modal') or contains(@class,'guide')][1]"
            ),
            component=self.name,
            version=self.version,
            capability="video.tutorial.dismiss",
            strategy="tutorial action -> unique local dialog",
            scope=page.locator("body"),
        )
        action = await require_target(
            dialog.get_by_text(VIDEO_ADAPTER.labels["tutorial"], exact=True),
            component=self.name,
            version=self.version,
            capability="video.tutorial.dismiss",
            strategy="tutorial dialog -> local semantic action",
            require_enabled=True,
            scope=dialog,
        )
        return action

    async def probe(self, page: Any) -> dict[str, Any]:
        visible = bool(
            await visible_candidates(
                page.get_by_text(VIDEO_ADAPTER.labels["tutorial"], exact=True),
                limit=10,
            )
        )
        return await resolution_probe(
            lambda: self.action(page),
            component=self.name,
            version=self.version,
            capability="video.tutorial.dismiss",
            strategy="semantic tutorial action -> local dialog -> local action",
            exercised=visible,
            not_exercised_reason="tutorial_dialog_not_present",
        )


class ProfileVideoListComponent:
    name = "profile-video-list"
    version = "2026-08-02.1"

    def __init__(self, page: Any) -> None:
        self.page = page

    async def scroll_container(self) -> Any:
        return await require_target(
            PROFILE_ADAPTER.locator(self.page, "scroll_container"),
            component=self.name,
            version=self.version,
            capability="video.list.paginate",
            strategy="profile route -> versioned scroll container",
            scope=self.page.locator("body"),
        )

    async def scroll_to_end(self) -> bool:
        container = await self.scroll_container()
        return bool(
            await container.evaluate(
                "element => { const before=element.scrollTop; element.scrollTop=element.scrollHeight; return element.scrollTop !== before; }"
            )
        )


class VideoDetailComponent:
    name = "video-detail"
    version = "2026-08-02.1"

    def __init__(self, page: Any) -> None:
        self.page = page

    async def render_data(self) -> str | None:
        locator = VIDEO_ADAPTER.locator(self.page, "render_data")
        count = await locator.count()
        if count == 0:
            return None
        if count != 1:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The structured video detail node is not unique.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "video.detail.read",
                    "strategy": "video route -> unique structured render node",
                    "candidate_count": count,
                },
            )
        return await locator.text_content(timeout=3000) or ""


class VideoCommentsPanelComponent:
    name = "video-comments-panel"
    version = "2026-08-03.1"
    comment_button_selector = '[data-e2e="feed-comment-icon"], [data-e2e*="comment" i], button[aria-label*="评论"], [title*="评论"]'
    reply_expand_selector = "button.comment-reply-expand-btn"

    def candidates(self, page: Any) -> Any:
        return page.locator(self.comment_button_selector)

    async def _panel(self, page: Any, *, required: bool) -> Any | None:
        lists = page.locator('[data-e2e="comment-list"]')
        visible = await visible_candidates(lists)
        if not visible and not required:
            return None
        comment_list = await require_target(
            lists,
            component=self.name,
            version=self.version,
            capability="video.comments.panel",
            strategy="versioned comment list -> visible local panel",
        )
        panel = comment_list.locator(
            "xpath=ancestor::*[.//*[contains(@class,'comment-input-inner-container')]][1]"
        )
        if await panel.count() == 0:
            return comment_list
        resolved = await require_target(
            panel,
            component=self.name,
            version=self.version,
            capability="video.comments.panel",
            strategy="comment list -> local panel containing composer",
        )
        await require_target(
            comment_list,
            component=self.name,
            version=self.version,
            capability="video.comments.panel",
            strategy="resolved panel -> contained comment list",
            scope=resolved,
        )
        await require_target(
            VIDEO_ADAPTER.locator(page, "comment_container"),
            component=self.name,
            version=self.version,
            capability="video.comments.panel",
            strategy="resolved panel -> contained comment composer",
            scope=resolved,
        )
        return resolved

    async def _action_bar_and_button(self, page: Any) -> tuple[Any, Any]:
        candidates = page.locator(self.comment_button_selector)
        deadline = asyncio.get_running_loop().time() + 20
        button = None
        while asyncio.get_running_loop().time() < deadline:
            visible = await visible_candidates(candidates)
            if len(visible) == 1:
                button = await require_target(
                    visible[0],
                    component=self.name,
                    version=self.version,
                    capability="video.comments.open",
                    strategy="bounded render wait -> unique semantic comment action",
                    require_enabled=True,
                )
                break
            if len(visible) > 1:
                await require_target(
                    candidates,
                    component=self.name,
                    version=self.version,
                    capability="video.comments.open",
                    strategy="bounded render wait -> unique semantic comment action",
                    require_enabled=True,
                )
            await page.wait_for_timeout(250)
        if button is None:
            await require_target(
                candidates,
                component=self.name,
                version=self.version,
                capability="video.comments.open",
                strategy="bounded render wait -> unique semantic comment action",
                require_enabled=True,
            )
            raise AssertionError("unreachable")
        bar = await require_target(
            button.locator(
                "xpath=ancestor::*[@role='toolbar' or contains(@class,'action') or contains(@class,'right')][1]"
            ),
            component=self.name,
            version=self.version,
            capability="video.comments.open",
            strategy="comment action -> local video action bar",
            scope=page.locator("body"),
        )
        local = await require_target(
            bar.locator(self.comment_button_selector),
            component=self.name,
            version=self.version,
            capability="video.comments.open",
            strategy="video action bar -> local comment action",
            require_enabled=True,
            scope=bar,
        )
        return bar, local

    async def probe(self, page: Any, exercised: bool) -> dict[str, Any]:
        if not exercised:
            return not_exercised_probe(
                component=self.name,
                version=self.version,
                capability="video.comments.open",
                strategy="video action bar -> local comment action",
                reason="video_route_not_open",
            )
        panel = await self._panel(page, required=False)

        async def resolve() -> Any:
            if panel is not None:
                return await self._panel(page, required=True)
            _bar, button = await self._action_bar_and_button(page)
            return button

        return await resolution_probe(
            resolve,
            component=self.name,
            version=self.version,
            capability="video.comments.open",
            strategy="video route -> local action bar/comment panel -> geometry",
            exercised=True,
            not_exercised_reason="video_route_not_open",
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
        operations += await TutorialDialogComponent().dismiss(session.page)
        operations += await self.ensure_open(session.page)
        return session.page, operations

    async def ensure_open(self, page: Any) -> int:
        if await self._panel(page, required=False) is not None:
            return 0
        _bar, button = await self._action_bar_and_button(page)
        try:
            await button.click(timeout=15_000)
            for _ in range(200):
                if await self._panel(page, required=False) is not None:
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

    async def trigger_reply_template(self, page: Any, max_attempts: int = 2) -> int:
        panel = await self._panel(page, required=True)
        visible = await visible_candidates(panel.locator(self.reply_expand_selector))
        if not visible:
            return 0
        positioned = [(item, await item.bounding_box()) for item in visible]
        positioned = [(item, box) for item, box in positioned if box]
        top_y = min((box["y"] for _, box in positioned), default=None)
        topmost = [
            item
            for item, box in positioned
            if top_y is not None and abs(box["y"] - top_y) < 1
        ]
        if len(topmost) != 1:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The reply expansion trigger is not geometrically unique.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "video.comments.reply-template",
                    "strategy": "comment panel -> local reply buttons -> unique topmost",
                    "candidate_count": len(visible),
                    "visible_count": len(visible),
                    "enabled_count": 0,
                    "geometry_reason": "topmost_not_unique",
                },
            )
        scroll_result = topmost[0].scroll_into_view_if_needed(timeout=10_000)
        if hasattr(scroll_result, "__await__"):
            await scroll_result
        trigger = await require_target(
            topmost[0],
            component=self.name,
            version=self.version,
            capability="video.comments.reply-template",
            strategy="comment panel -> local reply buttons -> unique topmost",
            require_enabled=True,
            scope=panel,
            scope_tolerance=4,
        )
        label = (await trigger.inner_text()).strip()
        if not re.search(r"(展开|查看).*回复", label):
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The reply expansion trigger lacks local semantic evidence.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "video.comments.reply-template",
                    "strategy": "reply button -> local expand/view reply label",
                    "geometry_reason": "state_mismatch",
                },
            )
        attempts = max(1, min(int(max_attempts), 3))
        last_error: Exception | None = None
        for attempt in range(1, attempts + 1):
            try:
                await trigger.click(timeout=10_000)
                return 1
            except Exception as error:  # noqa: BLE001 - browser drivers vary by backend
                last_error = error
                if attempt < attempts:
                    await page.wait_for_timeout(150 * attempt)
        raise DoudouError(
            "PAGE_ADAPTER_DRIFTED",
            "The reply expansion trigger did not complete after bounded attempts.",
            {
                "component": self.name,
                "component_version": self.version,
                "capability": "video.comments.reply-template",
                "strategy": "comment panel -> unique topmost reply trigger -> bounded retry",
                "geometry_reason": "reply_trigger_action_failed",
                "attempts": attempts,
                "cause_type": type(last_error).__name__ if last_error else None,
            },
        ) from last_error


class VideoCommentComposerComponent:
    name = "video-comment-composer"
    version = "2026-08-03.2"
    container_selector = ".comment-input-inner-container"
    editor_selector = '[contenteditable="true"][role="combobox"], textarea, input'
    controls_selector = ".commentInput-right-ct span"

    def __init__(self, page: Any, scope: Any | None = None) -> None:
        self.page = page
        self.scope = scope

    def candidates(self) -> Any:
        root = self.scope if self.scope is not None else self.page
        return root.locator(self.container_selector)

    def _scope(self) -> Any:
        return self.scope if self.scope is not None else self.page.locator("body")

    def _capability(self) -> str:
        return "video.comment.reply" if self.scope is not None else "video.comment.compose"

    async def container(self) -> Any:
        return await require_target(
            self.candidates(),
            component=self.name,
            version=self.version,
            capability=self._capability(),
            strategy=(
                "confirmed comment item -> local reply input container"
                if self.scope is not None
                else "visible comment input container"
            ),
            scope=self._scope(),
        )

    async def editor(self) -> Any:
        container = await self.container()
        return await require_target(
            container.locator(self.editor_selector),
            component=self.name,
            version=self.version,
            capability=self._capability(),
            strategy=(
                "confirmed comment item -> local reply container -> Draft combobox"
                if self.scope is not None
                else "composer-local Draft combobox"
            ),
            scope=container,
        )

    async def insert_text(self, text: str) -> Any:
        editor = await self.editor()
        current = await editor.inner_text()
        if current.strip("\u200b\u200c\u200d\ufeff\r\n "):
            raise DoudouError(
                "COMMENT_EDITOR_NOT_EMPTY",
                "The comment editor contains unplanned content.",
            )
        await self._focus_editor(editor)
        await self.page.keyboard.insert_text(text)
        await self.page.wait_for_timeout(250)
        return editor

    async def activate(self) -> int:
        container = await self.container()
        editors = container.locator(self.editor_selector)
        visible = await visible_candidates(editors)
        if not visible:
            await click_verified_target(
                self.page,
                container,
                component=self.name,
                version=self.version,
                capability="video.comment.compose",
                strategy=(
                    "visible comment container -> stable hit target -> "
                    "local editor render"
                ),
                scroll_into_view=False,
            )
            for _ in range(40):
                visible = await visible_candidates(editors)
                if visible:
                    break
                await self.page.wait_for_timeout(50)
        editor = await require_target(
            editors,
            component=self.name,
            version=self.version,
            capability="video.comment.compose",
            strategy="activated comment container -> local Draft combobox",
            scope=container,
        )
        await self._focus_editor(editor)
        return 1

    async def _focus_editor(self, editor: Any) -> None:
        focused = await editor.evaluate(
            "element => element === document.activeElement || "
            "element.contains(document.activeElement)"
        )
        if not focused:
            await click_verified_target(
                self.page,
                editor,
                component=self.name,
                version=self.version,
                capability="video.comment.compose",
                strategy=(
                    "local comment editor -> stable hit target -> dynamic mouse focus"
                ),
                scroll_into_view=False,
            )
            focused = await editor.evaluate(
                "element => element === document.activeElement || "
                "element.contains(document.activeElement)"
            )
        if not focused:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The verified comment editor focus action did not focus the editor.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "video.comment.compose",
                    "strategy": (
                        "local comment editor -> dynamic mouse focus -> "
                        "activeElement confirmation"
                    ),
                    "geometry_reason": "comment_editor_not_focused_after_action",
                },
            )

    async def composed_text(self, editor: Any) -> str:
        return await editor.evaluate(
            "element => 'value' in element ? element.value : (element.innerText || element.textContent || '')"
        )

    async def send_control(self) -> Any:
        container = await self.container()
        controls = container.locator(self.controls_selector)
        visible = await visible_candidates(controls)
        boxes = [(item, await item.bounding_box()) for item in visible]
        boxes = [(item, box) for item, box in boxes if box]
        rightmost_x = max((box["x"] for _, box in boxes), default=None)
        rightmost = [item for item, box in boxes if box["x"] == rightmost_x]
        if len(rightmost) != 1:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The video comment send action is not unique.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "video.comment.send",
                    "strategy": "rightmost composer-local action",
                    "candidate_count": len(rightmost),
                    "visible_count": len(visible),
                    "enabled_count": 0,
                    "geometry_reason": "rightmost_not_unique",
                },
            )
        return await require_target(
            rightmost[0],
            component=self.name,
            version=self.version,
            capability="video.comment.send",
            strategy="rightmost composer-local action",
            require_enabled=True,
            scope=container,
            peers=controls,
            position="rightmost",
            minimum_peer_count=2,
        )

    async def publish(self) -> int:
        await self.send_control()
        editor = await self.editor()
        await editor.press("Enter", timeout=5_000)
        return 1

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
    version = "2026-08-03.2"
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

    async def _scroll_toward_end(self, root: Any) -> dict[str, Any]:
        return await root.evaluate(
            """element => {
              const scrollable = node => {
                const style = getComputedStyle(node);
                return node.scrollHeight > node.clientHeight + 20
                  && ['auto', 'scroll'].includes(style.overflowY);
              };
              const preferredNodes = [
                ...(element.matches('.comment-mainContent') ? [element] : []),
                ...element.querySelectorAll('.comment-mainContent')
              ];
              for (let node = element.parentElement; node; node = node.parentElement) {
                if (node.matches('.comment-mainContent')) preferredNodes.push(node);
              }
              const preferred = preferredNodes.filter(scrollable);
              if (preferredNodes.length && preferred.length !== 1) {
                return {
                  resolved: false,
                  reason: 'comment_main_content_not_unique_or_scrollable',
                  preferred_count: preferredNodes.length,
                  preferred_scrollable_count: preferred.length,
                };
              }
              const local = preferred.length ? preferred : [element, ...element.querySelectorAll('div')]
                .filter(scrollable);
              const ancestors = [];
              for (let node = element.parentElement; node; node = node.parentElement) {
                if (scrollable(node)) ancestors.push(node);
              }
              const candidates = local.length ? local : ancestors;
              if (candidates.length !== 1) {
                return {
                  resolved: false,
                  reason: 'comment_scroll_root_not_unique',
                  local_count: local.length,
                  ancestor_count: ancestors.length,
                };
              }
              const scroller = candidates[0];
              const before = scroller.scrollTop;
              scroller.scrollTop = Math.min(
                scroller.scrollHeight,
                before + Math.max(500, scroller.clientHeight * 0.8)
              );
              return {
                resolved: true,
                source: preferred.length ? 'comment-mainContent' : (local.length ? 'local' : 'ancestor'),
                before,
                after: scroller.scrollTop,
                moved: scroller.scrollTop !== before,
                client_height: scroller.clientHeight,
                scroll_height: scroller.scrollHeight,
              };
            }"""
        )

    async def target(
        self, text: str, author_name: str | None = None, attempts: int = 30
    ) -> Any:
        root = await self._list()
        visible_text = visible_comment_text(text)
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
                return await require_target(
                    items[0],
                    component=self.name,
                    version=self.version,
                    capability="video.comment.target",
                    strategy="comment-list exact text plus author -> local item",
                    scope=root,
                )
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
            scroll = await self._scroll_toward_end(root)
            if not scroll.get("resolved"):
                raise DoudouError(
                    "PAGE_ADAPTER_DRIFTED",
                    "The video comment scroll root is not uniquely resolved.",
                    {
                        "component": self.name,
                        "component_version": self.version,
                        "capability": "video.comment.target",
                        "strategy": (
                            "comment-list -> unique local or containing scroll root"
                        ),
                        "geometry_reason": scroll.get("reason"),
                        "local_count": scroll.get("local_count"),
                        "ancestor_count": scroll.get("ancestor_count"),
                        "preferred_count": scroll.get("preferred_count"),
                        "preferred_scrollable_count": scroll.get(
                            "preferred_scrollable_count"
                        ),
                    },
                )
            if not scroll.get("moved"):
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
        action = await require_target(
            item.get_by_text("回复", exact=True),
            component=self.name,
            version=self.version,
            capability="video.comment.reply",
            strategy="comment-item-local semantic reply",
            require_enabled=True,
            scope=item,
        )
        await action.click()
        containers = item.locator(VideoCommentComposerComponent.container_selector)
        for _ in range(50):
            visible = await visible_candidates(containers)
            if len(visible) == 1:
                await require_target(
                    containers,
                    component=self.name,
                    version=self.version,
                    capability="video.comment.reply",
                    strategy=(
                        "confirmed comment item -> local reply action -> "
                        "local reply composer"
                    ),
                    scope=item,
                )
                return 1
            if len(visible) > 1:
                break
            await self.page.wait_for_timeout(100)
        raise DoudouError(
            "PAGE_ADAPTER_DRIFTED",
            "The confirmed comment did not expose one local reply composer.",
            {
                "component": self.name,
                "component_version": self.version,
                "capability": "video.comment.reply",
                "strategy": (
                    "confirmed comment item -> local reply action -> "
                    "local reply composer"
                ),
                "candidate_count": min(await containers.count(), 40),
                "visible_count": len(await visible_candidates(containers)),
                "geometry_reason": "reply_composer_not_unique",
            },
        )

    async def like_control(self, item: Any, count: int) -> Any:
        stats = await require_target(
            item.locator(self.stats_selector),
            component=self.name,
            version=self.version,
            capability="video.comment.like",
            strategy="comment-item-local stats container",
            scope=item,
            scope_tolerance=4,
        )
        count_label = await require_target(
            stats.get_by_text(str(count), exact=True),
            component=self.name,
            version=self.version,
            capability="video.comment.like",
            strategy="stats-local exact protocol count label",
            scope=stats,
        )
        control = await require_target(
            count_label.locator(
                "xpath=ancestor-or-self::*[self::button or @role='button' or .//*[name()='svg']][1]"
            ),
            component=self.name,
            version=self.version,
            capability="video.comment.like",
            strategy="protocol count label -> local icon/button action",
            require_enabled=True,
            scope=stats,
        )
        await require_target(
            control.locator("svg, [aria-label*='赞'], [data-e2e*='digg' i]"),
            component=self.name,
            version=self.version,
            capability="video.comment.like",
            strategy="local like action -> semantic/icon evidence",
            scope=control,
        )
        pressed = await control.get_attribute("aria-pressed")
        if pressed == "true":
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The page like state contradicts the unliked protocol plan.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "video.comment.like",
                    "geometry_reason": "state_mismatch",
                },
            )
        return control

    async def like(self, item: Any, count: int) -> int:
        await (await self.like_control(item, count)).click()
        return 1
