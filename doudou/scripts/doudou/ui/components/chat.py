from __future__ import annotations

import asyncio
from pathlib import Path
from typing import Any

from ...common.errors import DoudouError
from ..adapters import CHAT_ADAPTER
from .base import (
    click_verified_target,
    component_probe,
    not_exercised_probe,
    require_target,
    resolution_probe,
    unique_visible,
    visible_candidates,
)


def message_semantics_match(expected: str, evidence: dict[str, Any]) -> bool:
    remaining = expected
    for label in evidence.get("media_labels") or []:
        if not label or label not in remaining:
            return False
        remaining = remaining.replace(label, "", 1)
    normalized_expected = (
        remaining.replace("\r\n", "\n")
        .rstrip("\n")
        .translate(
            {ord(char): None for char in ("\u200b", "\u200c", "\u200d", "\ufeff")}
        )
    )
    normalized_actual = (
        str(evidence.get("text") or "")
        .replace("\r\n", "\n")
        .rstrip("\n")
        .translate(
            {ord(char): None for char in ("\u200b", "\u200c", "\u200d", "\ufeff")}
        )
    )
    return normalized_actual == normalized_expected


class ConversationListComponent:
    name = "conversation-list"
    version = "2026-08-03.3"

    def __init__(self, page: Any) -> None:
        self.page = page

    async def _container(self) -> Any:
        return await require_target(
            CHAT_ADAPTER.locator(self.page, "conversation_list"),
            component=self.name,
            version=self.version,
            capability="chat.conversation.list",
            strategy="versioned conversation-list container with data-e2e items",
            scope=self.page.locator("body"),
        )

    async def _items(self) -> tuple[Any, list[Any], int]:
        container = await self._container()
        locator = container.locator(CHAT_ADAPTER.selectors["session_candidates"])
        items = await visible_candidates(locator)
        return container, items, min(await locator.count(), 100)

    async def evidence(self, wait_seconds: float = 0) -> dict[str, Any]:
        deadline = asyncio.get_running_loop().time() + wait_seconds
        while True:
            try:
                _container, items, total = await self._items()
                loaded = bool(items)
                container_count = 1
            except DoudouError:
                items, total, loaded, container_count = [], 0, False, 0
            if loaded or asyncio.get_running_loop().time() >= deadline:
                break
            await self.page.wait_for_timeout(250)
        return {
            "chat_route": str(self.page.url).startswith("https://www.douyin.com/chat"),
            "loaded": loaded,
            "candidate_count": total,
            "left_region_count": len(items),
            "container_count": container_count,
            "component": self.name,
            "component_version": self.version,
            "strategy": "unique versioned conversation list -> local data-e2e items",
        }

    async def _scroll_local_viewport(
        self, container: Any, *, reset: bool = False
    ) -> dict[str, Any]:
        return await container.evaluate(
            """(element, reset) => {
              const first = element.querySelector('[data-e2e="conversation-item"]');
              if (!first) return {resolved: false, reason: 'no_session_item'};
              const candidates = [];
              let current = first.parentElement;
              while (current && element.contains(current)) {
                if (current.scrollHeight > current.clientHeight + 1) candidates.push(current);
                if (current === element) break;
                current = current.parentElement;
              }
              if (candidates.length !== 1) {
                return {resolved: false, reason: 'scroll_viewport_not_unique', candidate_count: candidates.length};
              }
              const viewport = candidates[0];
              const before = viewport.scrollTop;
              viewport.scrollTop = reset
                ? 0
                : Math.min(viewport.scrollHeight, before + Math.max(160, viewport.clientHeight * 0.8));
              return {
                resolved: true,
                before,
                after: viewport.scrollTop,
                client_height: viewport.clientHeight,
                scroll_height: viewport.scrollHeight,
                moved: viewport.scrollTop !== before,
              };
            }""",
            reset,
        )

    async def open(self, display_name: str) -> int:
        deadline = asyncio.get_running_loop().time() + 15
        target = None
        diagnostics = {"candidate_count": 0, "visible_count": 0}
        initial_container = await self._container()
        scroll = await self._scroll_local_viewport(initial_container, reset=True)
        if scroll.get("resolved") and scroll.get("moved"):
            await self.page.wait_for_timeout(250)
        while asyncio.get_running_loop().time() < deadline:
            try:
                container, items, total_items = await self._items()
            except DoudouError as error:
                if error.code != "PAGE_ADAPTER_DRIFTED":
                    raise
                if error.details.get("candidate_count") or error.details.get(
                    "visible_count"
                ):
                    raise
                diagnostics = {
                    "candidate_count": 0,
                    "visible_count": 0,
                    "matching_items": 0,
                    "render_state": "conversation_list_pending",
                }
                await self.page.wait_for_timeout(250)
                continue
            matches: list[Any] = []
            for item in items:
                labels = await visible_candidates(
                    item.get_by_text(display_name, exact=True), limit=5
                )
                if len(labels) == 1:
                    matches.append(item)
            diagnostics = {
                "candidate_count": total_items,
                "visible_count": len(items),
                "matching_items": len(matches),
                "scroll": scroll,
            }
            if len(matches) == 1:
                target = await require_target(
                    matches[0],
                    component=self.name,
                    version=self.version,
                    capability="chat.conversation.open",
                    strategy="versioned session items -> left list region -> exact protocol label",
                    require_enabled=True,
                    scope=container,
                )
                break
            if len(matches) > 1:
                break
            scroll = await self._scroll_local_viewport(container)
            if not scroll.get("resolved") or not scroll.get("moved"):
                break
            await self.page.wait_for_timeout(250)
        if target is None:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The left conversation list did not resolve the protocol-selected target.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "chat.conversation.open",
                    "strategy": "versioned session items -> left list region -> exact protocol label",
                    **diagnostics,
                    "geometry_reason": "left_list_target_not_unique",
                },
            )
        class_name = str(await target.get_attribute("class") or "")
        if "conversationConversationItemcurConversation" in class_name:
            return 0
        click_evidence = await click_verified_target(
            self.page,
            target,
            component=self.name,
            version=self.version,
            capability="chat.conversation.open",
            strategy=(
                "versioned session items -> exact protocol label -> stable hit target "
                "-> dynamic mouse click"
            ),
            scroll_into_view=False,
        )
        selected_deadline = asyncio.get_running_loop().time() + 5
        while asyncio.get_running_loop().time() < selected_deadline:
            class_name = str(await target.get_attribute("class") or "")
            if "conversationConversationItemcurConversation" in class_name:
                return 1
            await self.page.wait_for_timeout(100)
        raise DoudouError(
            "PAGE_ADAPTER_DRIFTED",
            "The verified conversation click did not select the target row.",
            {
                "component": self.name,
                "component_version": self.version,
                "capability": "chat.conversation.open",
                "strategy": (
                    "versioned session items -> exact protocol label -> stable hit "
                    "target -> selected-state confirmation"
                ),
                "geometry_reason": "conversation_not_selected_after_click",
                "click_evidence": click_evidence,
            },
        )

    async def probe(self, exercised: bool = True) -> dict[str, Any]:
        if not exercised:
            return not_exercised_probe(
                component=self.name,
                version=self.version,
                capability="chat.conversation.open",
                strategy="unique conversation list -> local data-e2e items",
                reason="chat_route_not_open",
            )
        evidence = await self.evidence(wait_seconds=15)
        return {
            **evidence,
            "capability": "chat.conversation.open",
            "exercised": True,
            "status": "ok" if evidence["loaded"] else "drifted",
            "visible_count": evidence["left_region_count"],
            "unique": evidence["container_count"] == 1,
            "geometry_valid": evidence["container_count"] == 1,
            "geometry_reason": None
            if evidence["loaded"]
            else "left_list_not_loaded",
        }


class ChatComposerComponent:
    name = "chat-composer"
    version = "2026-08-03.1"
    composer_selector = '[data-e2e="msg-input"] [contenteditable="true"]'
    container_xpath = (
        "xpath=ancestor::div[contains(@class,'messageMsgInputinputRow')][1]"
    )
    send_selector = "svg.e2e-send-msg-btn"

    def __init__(self, page: Any) -> None:
        self.page = page

    def candidates(self) -> Any:
        return self.page.locator(self.composer_selector)

    async def editor(self) -> Any:
        return await unique_visible(
            self.candidates(),
            component=self.name,
            version=self.version,
            capability="chat.compose",
            strategy="data-e2e msg-input -> visible contenteditable",
        )

    async def container(self) -> Any:
        editor = await self.editor()
        return await require_target(
            editor.locator(self.container_xpath),
            component=self.name,
            version=self.version,
            capability="chat.compose",
            strategy="contenteditable -> unique local input-row ancestor",
            scope=self.page.locator("body"),
        )

    async def root(self) -> Any:
        return await require_target(
            self.page.locator('[data-e2e="msg-input"]'),
            component=self.name,
            version=self.version,
            capability="chat.compose",
            strategy="unique data-e2e msg-input root",
            scope=self.page.locator("body"),
        )

    async def send_control(self) -> Any:
        container = await self.container()
        return await require_target(
            container.locator(self.send_selector),
            component=self.name,
            version=self.version,
            capability="chat.send",
            strategy="composer-local e2e send control -> rightmost local SVG",
            require_enabled=True,
            scope=container,
            peers=container.locator("svg"),
            position="rightmost",
            minimum_peer_count=2,
        )

    async def send(self) -> int:
        await self.send_control()
        editor = await self.editor()
        await editor.press("Enter", timeout=5_000)
        return 1

    async def clear(self) -> Any:
        editor = await self.editor()
        await click_verified_target(
            self.page,
            editor,
            component=self.name,
            version=self.version,
            capability="chat.compose.clear",
            strategy="unique editor -> stable hit target -> dynamic mouse focus",
            scroll_into_view=False,
        )
        focused = await editor.evaluate(
            "element => element === document.activeElement || "
            "element.contains(document.activeElement)"
        )
        if not focused:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The verified composer focus action did not focus the editor.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "chat.compose.clear",
                    "strategy": (
                        "unique editor -> stable hit target -> activeElement confirmation"
                    ),
                    "geometry_reason": "composer_not_focused_after_action",
                },
            )
        await editor.press("Control+A", timeout=5_000)
        await editor.press("Backspace", timeout=5_000)
        if await self.text():
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The confirmed chat composer did not clear.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "chat.compose.clear",
                    "strategy": "unique contenteditable -> semantic fill empty -> state check",
                    "geometry_reason": "composer_clear_state_mismatch",
                },
            )
        return editor

    async def insert_text(self, text: str) -> Any:
        editor = await self.editor()
        await editor.press_sequentially(text, delay=35, timeout=30_000)
        return editor

    async def text(self) -> str:
        editor = await self.editor()
        return (
            (await editor.inner_text(timeout=5_000))
            .replace("\r\n", "\n")
            .rstrip("\n")
            .translate(
                {ord(char): None for char in ("\u200b", "\u200c", "\u200d", "\ufeff")}
            )
        )

    async def probe(self, exercised: bool) -> dict[str, Any]:
        return await component_probe(
            self.candidates(),
            component=self.name,
            version=self.version,
            capability="chat.compose",
            strategy="data-e2e msg-input -> contenteditable",
            exercised=exercised,
        )

    async def operation_probes(
        self,
        exercised: bool,
    ) -> list[dict[str, Any]]:
        composer = await self.probe(exercised)
        send = await resolution_probe(
            self.send_control,
            component=self.name,
            version=self.version,
            capability="chat.send",
            strategy="composer -> local input row -> rightmost send control",
            exercised=exercised,
            not_exercised_reason="conversation_not_selected",
        )
        return [composer, send]


class MessageItemComponent:
    name = "chat-message-item"
    version = "2026-08-03.1"
    item_selector = '[data-e2e="msg-item-content"]'

    def __init__(self, page: Any) -> None:
        self.page = page

    def candidates(self) -> Any:
        return self.page.locator(self.item_selector)

    async def probe(self, exercised: bool) -> dict[str, Any]:
        return await component_probe(
            self.candidates(),
            component=self.name,
            version=self.version,
            capability="chat.message.target",
            strategy="data-e2e message item plus protocol text",
            exercised=exercised,
        )

    async def target(self, text: str, capability: str) -> Any:
        root = await require_target(
            self.page.locator("body"),
            component=self.name,
            version=self.version,
            capability=capability,
            strategy="page body local scope",
        )
        async def resolve() -> tuple[list[Any], Any]:
            candidates = self.page.locator(self.item_selector)
            exact = []
            for item in await visible_candidates(candidates, limit=100):
                evidence = await item.evaluate(
                    """element => ({
                      text: element.innerText || element.textContent || '',
                      media_labels: [...element.querySelectorAll('img')]
                        .map(image => image.getAttribute('title') || image.getAttribute('alt'))
                        .filter(Boolean),
                    })"""
                )
                if message_semantics_match(text, evidence):
                    exact.append(item)
            return exact, candidates

        exact, candidates = await resolve()
        if len(exact) != 1:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The protocol-selected message component is not unique.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": capability,
                    "strategy": (
                        "data-e2e message item -> exact rendered text plus local "
                        "emoji semantics"
                    ),
                    "candidate_count": len(exact),
                    "visible_message_items": min(await candidates.count(), 100),
                },
            )
        await exact[0].scroll_into_view_if_needed(timeout=10_000)
        await self.page.wait_for_timeout(150)
        exact, candidates = await resolve()
        if len(exact) != 1:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The virtualized message component did not remain uniquely bound after scrolling.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": capability,
                    "strategy": (
                        "protocol text plus emoji semantics -> component scroll -> "
                        "post-scroll semantic rebind"
                    ),
                    "candidate_count": len(exact),
                    "visible_message_items": min(await candidates.count(), 100),
                    "geometry_reason": "message_rebind_not_unique_after_scroll",
                },
            )
        return await require_target(
            exact[0],
            component=self.name,
            version=self.version,
            capability=capability,
            strategy=(
                "data-e2e message item -> exact rendered text plus local emoji semantics"
            ),
            scope=root,
        )

    async def menu_action(self, item: Any, label: str, capability: str) -> int:
        await item.click(button="right", timeout=15_000)
        menu = await self.context_menu(item, capability)
        ready_deadline = asyncio.get_running_loop().time() + 3
        while asyncio.get_running_loop().time() < ready_deadline:
            if "MessageOperatePopWindowisReady" in str(
                await menu.get_attribute("class") or ""
            ):
                break
            await self.page.wait_for_timeout(50)
        else:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The message action menu did not reach its ready state.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": capability,
                    "strategy": "target message -> local menu -> ready state",
                    "geometry_reason": "message_menu_not_ready",
                },
            )
        action_rows = menu.locator(
            CHAT_ADAPTER.selectors["message_context_action"]
        ).filter(has_text=label)
        exact_rows = [
            row
            for row in await visible_candidates(action_rows)
            if (await row.inner_text()).strip() == label
        ]
        if len(exact_rows) != 1:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The message action menu did not resolve one exact action row.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": capability,
                    "strategy": (
                        "target message -> ready local action popup -> exact "
                        "semantic action row"
                    ),
                    "candidate_count": len(exact_rows),
                    "geometry_reason": "message_menu_action_not_unique",
                },
            )
        action = await require_target(
            exact_rows[0],
            component=self.name,
            version=self.version,
            capability=capability,
            strategy=(
                "target message -> ready local action popup -> exact semantic action row"
            ),
            require_enabled=True,
            scope=menu,
            anchor=item,
            max_anchor_distance=600,
            filter_by_geometry=True,
        )
        stable_samples = 0
        previous_box = None
        stable_deadline = asyncio.get_running_loop().time() + 3
        while asyncio.get_running_loop().time() < stable_deadline:
            box = await action.bounding_box()
            if box and previous_box:
                delta = max(
                    abs(box[key] - previous_box[key])
                    for key in ("x", "y", "width", "height")
                )
                stable_samples = stable_samples + 1 if delta <= 1 else 0
                if stable_samples >= 3:
                    break
            previous_box = box
            await self.page.wait_for_timeout(80)
        else:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The message action row did not settle before the click.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": capability,
                    "strategy": "ready local action row -> stable geometry samples",
                    "geometry_reason": "message_menu_action_unstable",
                },
            )
        await click_verified_target(
            self.page,
            action,
            component=self.name,
            version=self.version,
            capability=capability,
            strategy="ready local action row -> stable hit target -> dynamic mouse click",
            scroll_into_view=False,
        )
        return 2

    async def context_menu(self, item: Any, capability: str) -> Any:
        menu = await require_target(
            CHAT_ADAPTER.locator(self.page, "message_context_menu"),
            component=self.name,
            version=self.version,
            capability=capability,
            strategy="target message -> unique versioned local action popup",
            anchor=item,
            max_anchor_distance=700,
            filter_by_geometry=True,
        )
        return menu

    async def context_menu_probe(self) -> dict[str, Any]:
        visible = await visible_candidates(
            CHAT_ADAPTER.locator(self.page, "message_context_menu")
        )
        return not_exercised_probe(
            component=self.name,
            version=self.version,
            capability="chat.message.context_menu",
            strategy="target message -> versioned local action popup",
            reason="context_menu_not_open" if not visible else "target_message_not_bound",
        )


class ImagePreviewComponent:
    name = "chat-image-preview"
    version = "2026-08-03.1"
    upload_selector = '[role="button"].semi-upload-add'

    def __init__(self, page: Any) -> None:
        self.page = page
        self.source: Path | None = None

    def candidates(self) -> Any:
        return self.page.locator('[data-e2e="msg-input"]').locator(
            self.upload_selector
        )

    async def probe(self, exercised: bool) -> dict[str, Any]:
        return await component_probe(
            self.candidates(),
            component=self.name,
            version=self.version,
            capability="chat.image.choose",
            strategy="composer-local upload role button",
            exercised=exercised,
        )

    async def choose(self, source: Path) -> int:
        composer_root = await require_target(
            self.page.locator('[data-e2e="msg-input"]'),
            component=self.name,
            version=self.version,
            capability="chat.image.choose",
            strategy="unique visible composer root",
        )
        upload = await unique_visible(
            composer_root.locator(self.upload_selector),
            component=self.name,
            version=self.version,
            capability="chat.image.choose",
            strategy="composer-local upload role button",
        )
        upload = await require_target(
            upload,
            component=self.name,
            version=self.version,
            capability="chat.image.choose",
            strategy="composer-local upload role button",
            require_enabled=True,
            scope=composer_root,
        )
        async with self.page.expect_file_chooser(timeout=10_000) as chooser_info:
            await upload.click(timeout=10_000)
        await (await chooser_info.value).set_files(str(source))
        self.source = source
        return 2

    async def send(self) -> int:
        preview = await self.preview()
        image = await require_target(
            preview.locator("img"),
            component=self.name,
            version=self.version,
            capability="chat.image.send",
            strategy="preview dialog -> unique visible image",
            scope=preview,
        )
        if self.source is None:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The image preview is not bound to a selected source file.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "chat.image.send",
                    "strategy": "selected source -> preview dialog -> exact file label",
                    "geometry_reason": "image_source_not_bound",
                },
            )
        await require_target(
            preview.get_by_text(self.source.name, exact=True),
            component=self.name,
            version=self.version,
            capability="chat.image.send",
            strategy="selected source -> preview dialog -> exact file label",
            scope=preview,
        )
        action = await require_target(
            preview.locator(CHAT_ADAPTER.selectors["image_preview_send"]),
            component=self.name,
            version=self.version,
            capability="chat.image.send",
            strategy="preview dialog -> exact source label -> versioned send action",
            require_enabled=True,
            scope=preview,
            anchor=image,
            relation="below",
            max_anchor_distance=1000,
        )
        await action.click(timeout=10_000)
        return 1

    async def preview(self) -> Any:
        return await require_target(
            CHAT_ADAPTER.locator(self.page, "image_preview_dialog"),
            component=self.name,
            version=self.version,
            capability="chat.image.send",
            strategy="uploaded image -> unique visible preview/dialog",
        )

    async def preview_probe(self) -> dict[str, Any]:
        visible = bool(
            await visible_candidates(
                CHAT_ADAPTER.locator(self.page, "image_preview_dialog")
            )
        )
        return await resolution_probe(
            self.preview,
            component=self.name,
            version=self.version,
            capability="chat.image.send",
            strategy="uploaded image -> unique preview dialog -> local action",
            exercised=visible,
            not_exercised_reason="image_preview_not_open",
        )
