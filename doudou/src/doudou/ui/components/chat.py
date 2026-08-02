from __future__ import annotations

import contextlib
from pathlib import Path
from typing import Any

from ...common.errors import DoudouError
from ..adapters import MENTION_PICKER_BOUNDS_SCRIPT
from ..common import first_visible
from .base import component_probe, unique_visible, visible_candidates


class ConversationListComponent:
    name = "conversation-list"
    version = "2026-08-02.1"

    def __init__(self, page: Any) -> None:
        self.page = page

    async def open(self, display_name: str) -> int:
        matches = self.page.get_by_text(display_name, exact=True)
        with contextlib.suppress(Exception):
            await matches.first.wait_for(state="visible", timeout=15_000)
        target = await unique_visible(
            matches,
            component=self.name,
            version=self.version,
            capability="chat.conversation.open",
            strategy="exact visible conversation label after bounded render wait",
        )
        await target.click(timeout=15_000)
        return 1


class ChatComposerComponent:
    name = "chat-composer"
    version = "2026-08-02.1"
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

    async def send_control(self) -> Any:
        editor = await self.editor()
        container = editor.locator(self.container_xpath)
        if await container.count() != 1:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The chat composer container is not unique.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "chat.send",
                    "strategy": "composer ancestor input row",
                },
            )
        controls = await visible_candidates(container.locator(self.send_selector))
        if len(controls) != 1 or not await controls[0].is_enabled():
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The chat send control is not uniquely enabled.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "chat.send",
                    "strategy": "composer-local e2e-send-msg-btn",
                    "candidate_count": len(controls),
                },
            )
        send_box = await controls[0].bounding_box()
        sibling_svgs = await visible_candidates(container.locator("svg"))
        sibling_boxes = [
            box for item in sibling_svgs if (box := await item.bounding_box())
        ]
        if (
            not send_box
            or len(sibling_boxes) < 2
            or send_box["x"] != max(box["x"] for box in sibling_boxes)
        ):
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The chat send control is not the rightmost composer action.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "chat.send",
                    "strategy": "unique local SVG plus rightmost position",
                },
            )
        return controls[0]

    async def clear(self) -> Any:
        editor = await self.editor()
        await editor.click(timeout=10_000)
        await editor.press("Control+A", timeout=10_000)
        await editor.press("Backspace", timeout=10_000)
        return editor

    async def insert_text(self, text: str) -> Any:
        editor = await self.editor()
        await editor.press_sequentially(text, delay=35, timeout=30_000)
        return editor

    async def select_mention(self, name: str) -> int:
        editor = await self.editor()
        box = await editor.bounding_box()
        await editor.press("@", timeout=10_000)
        await self.page.wait_for_timeout(800)
        match = await self.page.evaluate(
            MENTION_PICKER_BOUNDS_SCRIPT,
            {
                "name": name,
                "minY": max(0, (box or {}).get("y", 0) - 300),
                "maxY": (box or {}).get("y", 0),
            },
        )
        if not match:
            raise DoudouError(
                "MENTION_TARGET_NOT_VISIBLE",
                "The protocol-selected member is not visible in the composer-local mention picker.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "chat.mention",
                    "strategy": "composer-bounded exact member label",
                },
            )
        await self.page.mouse.click(
            match["x"] + match["width"] / 2, match["y"] + match["height"] / 2
        )
        return 2

    async def text(self) -> str:
        editor = await self.editor()
        return (
            (await editor.inner_text())
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


class MessageItemComponent:
    name = "chat-message-item"
    version = "2026-08-02.1"
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
        candidates = self.page.locator(self.item_selector).filter(has_text=text)
        exact = []
        for item in await visible_candidates(candidates):
            if (await item.inner_text()).strip() == text:
                exact.append(item)
        if len(exact) != 1:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The protocol-selected message component is not unique.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": capability,
                    "strategy": "data-e2e message item plus exact protocol text",
                    "candidate_count": len(exact),
                },
            )
        return exact[0]

    async def menu_action(self, item: Any, label: str, capability: str) -> int:
        await item.click(button="right", timeout=15_000)
        action = await first_visible(self.page.get_by_text(label, exact=True))
        if not action:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The message component action is unavailable.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": capability,
                    "strategy": "message-local context menu semantic label",
                },
            )
        await action.click(timeout=15_000)
        return 2


class ImagePreviewComponent:
    name = "chat-image-preview"
    version = "2026-08-02.1"
    upload_selector = '[role="button"].semi-upload-add'

    def __init__(self, page: Any) -> None:
        self.page = page

    def candidates(self) -> Any:
        return self.page.locator(self.upload_selector)

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
        upload = await unique_visible(
            self.page.locator(self.upload_selector),
            component=self.name,
            version=self.version,
            capability="chat.image.choose",
            strategy="composer-local upload role button",
        )
        async with self.page.expect_file_chooser(timeout=10_000) as chooser_info:
            await upload.click(timeout=10_000)
        await (await chooser_info.value).set_files(str(source))
        return 2

    async def send(self) -> int:
        action = await first_visible(self.page.get_by_text("发送", exact=True))
        if not action or not await action.is_enabled():
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The image preview send control is unavailable.",
                {
                    "component": self.name,
                    "component_version": self.version,
                    "capability": "chat.image.send",
                    "strategy": "preview-local semantic send label",
                },
            )
        await action.click(timeout=10_000)
        return 1
