from __future__ import annotations

import asyncio
import unittest
from typing import Any
from unittest.mock import AsyncMock, MagicMock, patch

from doudou.common.errors import DoudouError
from doudou.ui.components.base import (
    click_verified_target,
    component_probe,
    require_target,
)
from doudou.ui.components.chat import (
    ChatComposerComponent,
    ConversationListComponent,
    message_semantics_match,
)
from doudou.ui.components.video import (
    CommentItemComponent,
    VideoCommentComposerComponent,
    VideoCommentsPanelComponent,
)


class FakeElement:
    def __init__(
        self,
        box: dict[str, float] | None,
        *,
        visible: bool = True,
        enabled: bool = True,
    ) -> None:
        self.box = box
        self.visible = visible
        self.enabled = enabled

    async def is_visible(self) -> bool:
        return self.visible

    async def is_enabled(self) -> bool:
        return self.enabled

    async def bounding_box(self) -> dict[str, float] | None:
        return self.box


class FakeClickableElement(FakeElement):
    def __init__(
        self,
        boxes: list[dict[str, float] | None],
        *,
        hit: dict[str, Any],
    ) -> None:
        super().__init__(boxes[-1] if boxes else None)
        self.boxes = iter(boxes)
        self.hit = hit
        self.scrolled = False

    async def scroll_into_view_if_needed(self, timeout: int) -> None:
        self.scrolled = timeout > 0

    async def bounding_box(self) -> dict[str, float] | None:
        return next(self.boxes)

    async def evaluate(self, expression: str) -> dict[str, Any]:
        self.expression = expression
        return self.hit


class FakeReplyTrigger(FakeElement):
    def __init__(
        self,
        boxes: list[dict[str, float] | None],
        *,
        events: list[str],
        label: str = "展开 2 条回复",
    ) -> None:
        super().__init__(boxes[-1] if boxes else None)
        self.boxes = iter(boxes)
        self.events = events
        self.label = label
        self.clicks = 0

    async def count(self) -> int:
        return 1

    def nth(self, index: int) -> FakeReplyTrigger:
        if index != 0:
            raise IndexError(index)
        return self

    async def scroll_into_view_if_needed(self, timeout: int) -> None:
        self.events.append("scroll")

    async def bounding_box(self) -> dict[str, float] | None:
        self.events.append("box")
        return next(self.boxes)

    async def inner_text(self) -> str:
        return self.label

    async def click(self, timeout: int) -> None:
        self.clicks += 1


class FakeMouse:
    def __init__(self) -> None:
        self.clicks: list[tuple[float, float]] = []

    async def click(self, x: float, y: float) -> None:
        self.clicks.append((x, y))


class FakePage:
    def __init__(self) -> None:
        self.mouse = FakeMouse()
        self.waits: list[int] = []

    async def wait_for_timeout(self, timeout: int) -> None:
        self.waits.append(timeout)


class FakeLocator:
    def __init__(self, *items: FakeElement) -> None:
        self.items = items

    async def count(self) -> int:
        return len(self.items)

    def nth(self, index: int) -> FakeElement:
        return self.items[index]


def box(
    x: float, y: float, width: float = 20, height: float = 20
) -> dict[str, float]:
    return {"x": x, "y": y, "width": width, "height": height}


class ComponentGeometryTests(unittest.IsolatedAsyncioTestCase):
    async def test_comment_scroll_prefers_unique_main_content(self) -> None:
        page = MagicMock()
        root = MagicMock()
        root.evaluate = AsyncMock(
            return_value={
                "resolved": True,
                "source": "comment-mainContent",
                "moved": True,
            }
        )

        result = await CommentItemComponent(page)._scroll_toward_end(root)

        self.assertEqual(result["source"], "comment-mainContent")
        script = root.evaluate.await_args.args[0]
        self.assertIn(".comment-mainContent", script)
        self.assertIn("node.matches('.comment-mainContent')", script)
        self.assertIn("preferred.length !== 1", script)

    async def test_reply_template_retries_once_after_click_failure(self) -> None:
        page = MagicMock()
        page.wait_for_timeout = AsyncMock()
        panel = MagicMock()
        trigger = MagicMock()
        trigger.bounding_box = AsyncMock(return_value=box(10, 20))
        trigger.inner_text = AsyncMock(return_value="展开 2 条回复")
        trigger.click = AsyncMock(side_effect=[TimeoutError("first"), None])
        component = VideoCommentsPanelComponent()
        component._panel = AsyncMock(return_value=panel)

        with (
            patch(
                "doudou.ui.components.video.visible_candidates",
                new=AsyncMock(return_value=[trigger]),
            ),
            patch(
                "doudou.ui.components.video.require_target",
                new=AsyncMock(return_value=trigger),
            ),
        ):
            result = await component.trigger_reply_template(page)

        self.assertEqual(result, 1)
        self.assertEqual(trigger.click.await_count, 2)
        page.wait_for_timeout.assert_awaited_once_with(150)

    async def test_reply_template_stops_at_retry_limit(self) -> None:
        page = MagicMock()
        page.wait_for_timeout = AsyncMock()
        panel = MagicMock()
        trigger = MagicMock()
        trigger.bounding_box = AsyncMock(return_value=box(10, 20))
        trigger.inner_text = AsyncMock(return_value="查看 2 条回复")
        trigger.click = AsyncMock(side_effect=TimeoutError("still blocked"))
        component = VideoCommentsPanelComponent()
        component._panel = AsyncMock(return_value=panel)

        with (
            patch(
                "doudou.ui.components.video.visible_candidates",
                new=AsyncMock(return_value=[trigger]),
            ),
            patch(
                "doudou.ui.components.video.require_target",
                new=AsyncMock(return_value=trigger),
            ),
            self.assertRaises(DoudouError) as context,
        ):
            await component.trigger_reply_template(page, max_attempts=2)

        self.assertEqual(trigger.click.await_count, 2)
        self.assertEqual(context.exception.details["attempts"], 2)
        self.assertEqual(
            context.exception.details["geometry_reason"],
            "reply_trigger_action_failed",
        )

    async def test_reply_template_scrolls_before_tolerant_scope_validation(self) -> None:
        page = MagicMock()
        panel = MagicMock()
        panel.bounding_box = AsyncMock(return_value=box(100, 100, 100, 40))
        events: list[str] = []
        # The pre-scroll geometry is 2.1px beyond the panel; after scrolling the
        # browser reports the trigger fully inside the confirmed local scope.
        trigger = FakeReplyTrigger(
            [box(182.1, 100), box(180, 100)], events=events
        )
        component = VideoCommentsPanelComponent()
        component._panel = AsyncMock(return_value=panel)

        with patch(
            "doudou.ui.components.video.visible_candidates",
            new=AsyncMock(side_effect=[[trigger], [trigger]]),
        ):
            result = await component.trigger_reply_template(page)

        self.assertEqual(result, 1)
        self.assertEqual(trigger.clicks, 1)
        self.assertLess(events.index("scroll"), events.index("box", 1))

    async def test_reply_template_rejects_target_more_than_tolerance_outside_scope(
        self,
    ) -> None:
        page = MagicMock()
        panel = MagicMock()
        panel.bounding_box = AsyncMock(return_value=box(100, 100, 100, 40))
        events: list[str] = []
        trigger = FakeReplyTrigger(
            [box(185, 100), box(185, 100)], events=events
        )
        component = VideoCommentsPanelComponent()
        component._panel = AsyncMock(return_value=panel)

        with (
            patch(
                "doudou.ui.components.video.visible_candidates",
                new=AsyncMock(side_effect=[[trigger], [trigger]]),
            ),
            self.assertRaises(DoudouError) as context,
        ):
            await component.trigger_reply_template(page)

        self.assertEqual(context.exception.code, "PAGE_ADAPTER_DRIFTED")
        self.assertEqual(
            context.exception.details["geometry_reason"], "outside_local_scope"
        )
        self.assertEqual(trigger.clicks, 0)

    async def test_message_semantics_matches_rendered_emoji_titles(self) -> None:
        self.assertTrue(
            message_semantics_match(
                "正文[尬笑]",
                {"text": "正文", "media_labels": ["[尬笑]"]},
            )
        )
        self.assertFalse(
            message_semantics_match(
                "正文[微笑]",
                {"text": "正文", "media_labels": ["[尬笑]"]},
            )
        )
        self.assertFalse(
            message_semantics_match(
                "正文",
                {"text": "正文", "media_labels": ["[尬笑]"]},
            )
        )

    async def assert_drift(
        self, locator: FakeLocator, reason: str, **options: Any
    ) -> DoudouError:
        with self.assertRaises(DoudouError) as context:
            await require_target(
                locator,
                component="fixture-component",
                version="test.v1",
                capability="fixture.write",
                strategy="fixture semantic -> local -> geometry",
                **options,
            )
        error = context.exception
        self.assertEqual(error.code, "PAGE_ADAPTER_DRIFTED")
        self.assertEqual(error.details["geometry_reason"], reason)
        return error

    async def test_unique_enabled_target_inside_scope_and_rightmost_passes(self) -> None:
        scope = FakeElement(box(0, 0, 300, 100))
        left = FakeElement(box(210, 40))
        target = FakeElement(box(260, 40))
        resolved = await require_target(
            FakeLocator(target),
            component="chat-composer",
            version="test.v1",
            capability="chat.send",
            strategy="local semantic send -> rightmost action",
            require_enabled=True,
            scope=scope,
            peers=FakeLocator(left, target),
            position="rightmost",
            minimum_peer_count=2,
        )
        self.assertIs(resolved, target)

    async def test_zero_and_duplicate_candidates_fail_closed(self) -> None:
        empty = await self.assert_drift(FakeLocator(), "target_not_unique")
        self.assertEqual(empty.details["candidate_count"], 0)
        duplicate = await self.assert_drift(
            FakeLocator(FakeElement(box(1, 1)), FakeElement(box(2, 2))),
            "target_not_unique",
        )
        self.assertEqual(duplicate.details["visible_count"], 2)

    async def test_hidden_and_disabled_targets_fail_closed(self) -> None:
        await self.assert_drift(
            FakeLocator(FakeElement(box(1, 1), visible=False)),
            "target_not_unique",
        )
        error = await self.assert_drift(
            FakeLocator(FakeElement(box(1, 1), enabled=False)),
            "target_disabled",
            require_enabled=True,
        )
        self.assertEqual(error.details["enabled_count"], 0)

    async def test_target_outside_local_scope_fails_closed(self) -> None:
        await self.assert_drift(
            FakeLocator(FakeElement(box(400, 20))),
            "outside_local_scope",
            scope=FakeElement(box(0, 0, 300, 100)),
        )

    async def test_component_specific_scope_tolerance_remains_bounded(self) -> None:
        scope = FakeElement(box(214, 100, 100, 30))
        css_overflow = FakeElement(box(211, 100, 100, 20))
        resolved = await require_target(
            FakeLocator(css_overflow),
            component="video-comment-item",
            version="test.v1",
            capability="video.comment.like",
            strategy="comment-item-local stats container",
            scope=scope,
            scope_tolerance=4,
        )
        self.assertIs(resolved, css_overflow)
        await self.assert_drift(
            FakeLocator(FakeElement(box(209, 100, 100, 20))),
            "outside_local_scope",
            scope=scope,
            scope_tolerance=4,
        )

    async def test_wrong_relative_position_fails_closed(self) -> None:
        target = FakeElement(box(200, 20))
        await self.assert_drift(
            FakeLocator(target),
            "not_rightmost",
            peers=FakeLocator(target, FakeElement(box(260, 20))),
            position="rightmost",
            minimum_peer_count=2,
        )

    async def test_anchor_filter_selects_only_local_candidate(self) -> None:
        anchor = FakeElement(box(200, 500, 400, 50))
        local = FakeElement(box(220, 400, 120, 30))
        unrelated = FakeElement(box(40, 40, 120, 30))
        resolved = await require_target(
            FakeLocator(unrelated, local),
            component="fixture-component",
            version="test.v1",
            capability="fixture.action",
            strategy="anchor -> local candidate",
            anchor=anchor,
            relation="above",
            max_anchor_distance=300,
            filter_by_geometry=True,
        )
        self.assertIs(resolved, local)

    async def test_probe_reports_geometry_drift(self) -> None:
        report = await component_probe(
            FakeLocator(FakeElement(None)),
            component="fixture-component",
            version="test.v1",
            capability="fixture.write",
            strategy="fixture",
            exercised=True,
        )
        self.assertEqual(report["status"], "drifted")
        self.assertFalse(report["geometry_valid"])
        self.assertEqual(report["geometry_reason"], "target_box_missing")

    async def test_verified_mouse_click_uses_stable_dynamic_center(self) -> None:
        page = FakePage()
        target = FakeClickableElement(
            [box(100, 200, 80, 40), box(100, 200, 80, 40)],
            hit={"receives_events": True, "blocker": None},
        )
        evidence = await click_verified_target(
            page,
            target,
            component="conversation-list",
            version="test.v1",
            capability="chat.conversation.open",
            strategy="exact row -> stable hit target -> dynamic mouse click",
        )
        self.assertTrue(target.scrolled)
        self.assertEqual(page.mouse.clicks, [(140, 220)])
        self.assertTrue(evidence["stable"])
        self.assertTrue(evidence["receives_events"])

    async def test_verified_mouse_click_can_skip_redundant_actionability_scroll(
        self,
    ) -> None:
        page = FakePage()
        target = FakeClickableElement(
            [box(100, 200, 80, 40), box(100, 200, 80, 40)],
            hit={"receives_events": True, "blocker": None},
        )
        await click_verified_target(
            page,
            target,
            component="conversation-list",
            version="test.v1",
            capability="chat.conversation.open",
            strategy="already in local scope -> stable hit target -> dynamic click",
            scroll_into_view=False,
        )
        self.assertFalse(target.scrolled)
        self.assertEqual(page.mouse.clicks, [(140, 220)])

    async def test_verified_mouse_click_defers_timeout_to_state_postcondition(
        self,
    ) -> None:
        page = FakePage()
        page.mouse.click = AsyncMock(side_effect=asyncio.TimeoutError)
        target = FakeClickableElement(
            [box(100, 200, 80, 40), box(100, 200, 80, 40)],
            hit={"receives_events": True, "blocker": None},
        )
        evidence = await click_verified_target(
            page,
            target,
            component="conversation-list",
            version="test.v1",
            capability="chat.conversation.open",
            strategy="verified input -> state postcondition",
            scroll_into_view=False,
        )
        self.assertFalse(evidence["dispatch_completed"])

    async def test_verified_mouse_click_fails_closed_when_occluded(self) -> None:
        page = FakePage()
        target = FakeClickableElement(
            [box(100, 200, 80, 40), box(100, 200, 80, 40)],
            hit={
                "receives_events": False,
                "blocker": {"tag": "DIV", "class_name": "loading-mask"},
            },
        )
        with self.assertRaises(DoudouError) as context:
            await click_verified_target(
                page,
                target,
                component="conversation-list",
                version="test.v1",
                capability="chat.conversation.open",
                strategy="exact row -> stable hit target -> dynamic mouse click",
            )
        self.assertEqual(context.exception.code, "PAGE_ADAPTER_DRIFTED")
        self.assertEqual(
            context.exception.details["geometry_reason"], "target_occluded"
        )
        self.assertEqual(page.mouse.clicks, [])

    async def test_chat_composer_focuses_with_mouse_and_sends_with_enter(self) -> None:
        page = MagicMock()
        editor = MagicMock()
        editor.press = AsyncMock()
        editor.evaluate = AsyncMock(return_value=True)
        control = MagicMock()
        composer = ChatComposerComponent(page)
        composer.editor = AsyncMock(return_value=editor)
        composer.text = AsyncMock(return_value="")
        composer.send_control = AsyncMock(return_value=control)
        click = AsyncMock(return_value={"receives_events": True})
        with patch("doudou.ui.components.chat.click_verified_target", click):
            await composer.clear()
            await composer.send()
        self.assertEqual(click.await_count, 1)
        self.assertIs(click.await_args_list[0].args[1], editor)
        self.assertEqual(
            click.await_args_list[0].kwargs["capability"], "chat.compose.clear"
        )
        composer.send_control.assert_awaited_once()
        editor.press.assert_any_await("Enter", timeout=5_000)

    async def test_video_comment_validates_control_and_sends_with_enter(self) -> None:
        page = MagicMock()
        editor = MagicMock()
        editor.press = AsyncMock()
        control = MagicMock()
        control.click = AsyncMock()
        composer = VideoCommentComposerComponent(page)
        composer.editor = AsyncMock(return_value=editor)
        composer.send_control = AsyncMock(return_value=control)

        operations = await composer.publish()

        self.assertEqual(operations, 1)
        composer.send_control.assert_awaited_once()
        composer.editor.assert_awaited_once()
        editor.press.assert_awaited_once_with("Enter", timeout=5_000)
        control.click.assert_not_awaited()

    async def test_video_reply_composer_is_scoped_to_the_confirmed_comment(self) -> None:
        page = MagicMock()
        comment_item = MagicMock()
        local_containers = MagicMock()
        comment_item.locator.return_value = local_containers

        composer = VideoCommentComposerComponent(page, scope=comment_item)

        self.assertIs(composer.candidates(), local_containers)
        comment_item.locator.assert_called_once_with(
            VideoCommentComposerComponent.container_selector
        )
        page.locator.assert_not_called()
        self.assertEqual(composer._capability(), "video.comment.reply")

    async def test_video_comment_focus_uses_mouse_postcondition_once(self) -> None:
        page = MagicMock()
        page.keyboard.insert_text = AsyncMock()
        page.wait_for_timeout = AsyncMock()
        container = MagicMock()
        editors = MagicMock()
        container.locator.return_value = editors
        editor = MagicMock()
        editor.inner_text = AsyncMock(return_value="")
        editor.evaluate = AsyncMock(side_effect=[True, True])
        composer = VideoCommentComposerComponent(page)
        composer.container = AsyncMock(return_value=container)
        composer.editor = AsyncMock(return_value=editor)
        click = AsyncMock(return_value={"receives_events": True})

        with (
            patch(
                "doudou.ui.components.video.visible_candidates",
                new=AsyncMock(side_effect=[[], [editor]]),
            ),
            patch(
                "doudou.ui.components.video.require_target",
                new=AsyncMock(return_value=editor),
            ),
            patch("doudou.ui.components.video.click_verified_target", click),
        ):
            await composer.activate()
            returned = await composer.insert_text("测试评论")

        self.assertIs(returned, editor)
        click.assert_awaited_once()
        self.assertEqual(
            click.await_args.kwargs["capability"], "video.comment.compose"
        )
        self.assertIs(click.await_args.args[1], container)
        page.keyboard.insert_text.assert_awaited_once_with("测试评论")

    async def test_video_comment_focus_fails_closed_without_postcondition(self) -> None:
        page = MagicMock()
        container = MagicMock()
        editors = MagicMock()
        container.locator.return_value = editors
        editor = MagicMock()
        editor.evaluate = AsyncMock(side_effect=[False, False])
        composer = VideoCommentComposerComponent(page)
        composer.container = AsyncMock(return_value=container)
        click = AsyncMock(return_value={"receives_events": True})

        with (
            patch(
                "doudou.ui.components.video.visible_candidates",
                new=AsyncMock(return_value=[editor]),
            ),
            patch(
                "doudou.ui.components.video.require_target",
                new=AsyncMock(return_value=editor),
            ),
            patch("doudou.ui.components.video.click_verified_target", click),
            self.assertRaises(DoudouError) as context,
        ):
            await composer.activate()

        self.assertEqual(context.exception.code, "PAGE_ADAPTER_DRIFTED")
        self.assertEqual(
            context.exception.details["geometry_reason"],
            "comment_editor_not_focused_after_action",
        )


class FakeConversationRow(FakeElement):
    def __init__(
        self,
        label: str,
        initial_box: dict[str, float],
        *,
        after_scroll_box: dict[str, float] | None = None,
    ) -> None:
        super().__init__(initial_box)
        self.label = label
        self.after_scroll_box = after_scroll_box
        self.selected = False
        self.scroll_calls = 0

    async def count(self) -> int:
        return 1

    def nth(self, index: int) -> FakeConversationRow:
        if index != 0:
            raise IndexError(index)
        return self

    def get_by_text(self, text: str, exact: bool = True) -> FakeLocator:
        matched = (self.label == text) if exact else (text in self.label)
        return FakeLocator(FakeElement(self.box)) if matched else FakeLocator()

    async def scroll_into_view_if_needed(self, timeout: int = 5_000) -> None:
        self.scroll_calls += 1
        if self.after_scroll_box is not None:
            self.box = self.after_scroll_box

    async def get_attribute(self, name: str) -> str:
        if name == "class" and self.selected:
            return "conversationConversationItemcurConversation"
        return "conversationConversationItem"

    async def evaluate(self, _expression: str) -> dict[str, Any]:
        return {"receives_events": True, "blocker": None}


class FakeConversationContainer(FakeElement):
    def __init__(
        self,
        container_box: dict[str, float],
        rows_by_scroll: dict[int, list[FakeConversationRow]],
        *,
        initial_scroll_top: int = 0,
    ) -> None:
        super().__init__(container_box)
        self.rows_by_scroll = rows_by_scroll
        self.scroll_top = initial_scroll_top
        self.reset_calls = 0

    def locator(self, _selector: str) -> FakeLocator:
        return FakeLocator(*self.rows_by_scroll.get(self.scroll_top, []))

    async def evaluate(self, _expression: str, reset: bool = False) -> dict[str, Any]:
        before = self.scroll_top
        if reset:
            self.reset_calls += 1
            self.scroll_top = 0
        else:
            next_offsets = sorted(
                offset for offset in self.rows_by_scroll if offset > self.scroll_top
            )
            if next_offsets:
                self.scroll_top = next_offsets[0]
        return {
            "resolved": True,
            "before": before,
            "after": self.scroll_top,
            "client_height": 400,
            "scroll_height": 800,
            "moved": self.scroll_top != before,
        }


class FakeChatPage(FakePage):
    def __init__(
        self,
        containers_per_lookup: list[list[FakeConversationContainer]],
    ) -> None:
        super().__init__()
        self.containers_per_lookup = containers_per_lookup
        self.lookup_index = 0
        self.body = FakeElement(box(0, 0, 1280, 800))
        self.mouse.click = self._click

    async def _click(self, x: float, y: float) -> None:
        self.mouse.clicks.append((x, y))
        active = self.containers_per_lookup[
            min(self.lookup_index - 1, len(self.containers_per_lookup) - 1)
        ]
        for container in active:
            for row in container.rows_by_scroll.get(container.scroll_top, []):
                if row.box is None:
                    continue
                if (
                    row.box["x"] <= x <= row.box["x"] + row.box["width"]
                    and row.box["y"] <= y <= row.box["y"] + row.box["height"]
                ):
                    row.selected = True

    def locator(self, selector: str) -> Any:
        if selector == "body":
            return self.body
        index = min(self.lookup_index, len(self.containers_per_lookup) - 1)
        self.lookup_index += 1
        return FakeLocator(*self.containers_per_lookup[index])


class ConversationListComponentTests(unittest.IsolatedAsyncioTestCase):
    async def test_conversation_list_waits_for_unmounted_container_then_resets_scroll(
        self,
    ) -> None:
        target_row = FakeConversationRow("测试群聊", box(20, 120, 280, 60))
        old_row = FakeConversationRow("旧会话", box(20, 120, 280, 60))
        container = FakeConversationContainer(
            box(0, 100, 320, 400),
            {0: [target_row], 240: [old_row]},
            initial_scroll_top=240,
        )
        page = FakeChatPage([[], [], [container]])

        operations = await ConversationListComponent(page).open("测试群聊")

        self.assertEqual(operations, 1)
        self.assertTrue(target_row.selected)
        self.assertEqual(container.reset_calls, 1)
        self.assertEqual(page.mouse.clicks, [(160.0, 150.0)])

    async def test_conversation_list_fails_immediately_on_non_empty_container_drift(
        self,
    ) -> None:
        first = FakeConversationContainer(box(0, 100, 320, 400), {0: []})
        second = FakeConversationContainer(box(340, 100, 320, 400), {0: []})
        page = FakeChatPage([[first, second]])

        with self.assertRaises(DoudouError) as context:
            await ConversationListComponent(page).open("测试群聊")

        self.assertEqual(context.exception.code, "PAGE_ADAPTER_DRIFTED")
        self.assertEqual(context.exception.details["candidate_count"], 2)
        self.assertEqual(context.exception.details["visible_count"], 2)
        self.assertEqual(page.waits, [])

    async def test_conversation_list_scrolls_partially_clipped_row_into_scope(
        self,
    ) -> None:
        clipped_row = FakeConversationRow(
            "测试私聊",
            box(20, 470, 280, 60),
            after_scroll_box=box(20, 440, 280, 60),
        )
        container = FakeConversationContainer(
            box(0, 100, 320, 400),
            {0: [clipped_row]},
        )
        page = FakeChatPage([[container]])

        operations = await ConversationListComponent(page).open("测试私聊")

        self.assertEqual(operations, 1)
        self.assertEqual(clipped_row.scroll_calls, 1)
        self.assertTrue(clipped_row.selected)
        self.assertEqual(page.mouse.clicks, [(160.0, 470.0)])

    async def test_conversation_list_rejects_row_outside_scope_after_scroll(
        self,
    ) -> None:
        outside_row = FakeConversationRow(
            "越界会话",
            box(20, 520, 280, 60),
            after_scroll_box=box(20, 520, 280, 60),
        )
        container = FakeConversationContainer(
            box(0, 100, 320, 400),
            {0: [outside_row]},
        )
        page = FakeChatPage([[container]])

        with self.assertRaises(DoudouError) as context:
            await ConversationListComponent(page).open("越界会话")

        self.assertEqual(context.exception.code, "PAGE_ADAPTER_DRIFTED")
        self.assertEqual(
            context.exception.details["geometry_reason"], "outside_local_scope"
        )
        self.assertEqual(page.mouse.clicks, [])


if __name__ == "__main__":
    unittest.main()
