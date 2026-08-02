from __future__ import annotations

import contextlib
from pathlib import Path
from typing import Any

from ...common.errors import DoudouError
from .base import component_probe, require_target, unique_visible, visible_candidates


class ConversationListComponent:
    name = "conversation-list"
    version = "2026-08-02.2"

    def __init__(self, page: Any) -> None:
        self.page = page

    async def open(self, display_name: str) -> int:
        matches = self.page.get_by_text(display_name, exact=True)
        with contextlib.suppress(Exception):
            await matches.first.wait_for(state="visible", timeout=15_000)
        target = await require_target(
            matches,
            component=self.name,
            version=self.version,
            capability="chat.conversation.open",
            strategy="exact visible conversation label after bounded render wait",
            require_enabled=True,
        )
        await target.click(timeout=15_000)
        return 1


class ChatComposerComponent:
    name = "chat-composer"
    version = "2026-08-02.2"
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
        await editor.press("@", timeout=10_000)
        await self.page.wait_for_timeout(800)
        match = await require_target(
            self.page.get_by_text(name, exact=True),
            component=self.name,
            version=self.version,
            capability="chat.mention",
            strategy="exact member label -> composer-bounded mention candidate",
            require_enabled=True,
            anchor=editor,
            relation="above",
            max_anchor_distance=450,
            filter_by_geometry=True,
        )
        await match.click(timeout=10_000)
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
    version = "2026-08-02.2"
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
        return await require_target(
            exact[0],
            component=self.name,
            version=self.version,
            capability=capability,
            strategy="data-e2e message item plus exact protocol text",
            scope=root,
        )

    async def menu_action(self, item: Any, label: str, capability: str) -> int:
        await item.click(button="right", timeout=15_000)
        action = await require_target(
            self.page.get_by_text(label, exact=True),
            component=self.name,
            version=self.version,
            capability=capability,
            strategy="message-local context menu semantic label",
            require_enabled=True,
            anchor=item,
            max_anchor_distance=600,
            filter_by_geometry=True,
        )
        await action.click(timeout=15_000)
        return 2


class ImagePreviewComponent:
    name = "chat-image-preview"
    version = "2026-08-02.2"
    upload_selector = '[role="button"].semi-upload-add'

    def __init__(self, page: Any) -> None:
        self.page = page

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
        return 2

    async def send(self) -> int:
        actions = self.page.get_by_text("发送", exact=True)
        action = await require_target(
            actions,
            component=self.name,
            version=self.version,
            capability="chat.image.send",
            strategy="semantic send label -> unique visible preview action",
            require_enabled=True,
        )
        preview = action.locator(
            "xpath=ancestor::*[@role='dialog' or contains(@class,'semi-modal') or contains(@class,'preview')][1]"
        )
        await require_target(
            preview,
            component=self.name,
            version=self.version,
            capability="chat.image.send",
            strategy="send action -> unique preview/dialog ancestor",
            scope=self.page.locator("body"),
        )
        await action.click(timeout=10_000)
        return 1
