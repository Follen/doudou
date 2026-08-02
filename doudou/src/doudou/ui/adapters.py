from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from ..common.errors import DoudouError


@dataclass(frozen=True)
class PageAdapter:
    name: str
    version: str
    route_prefix: str
    route_aliases: tuple[str, ...]
    selectors: dict[str, str]
    labels: dict[str, str]

    def matches_route(self, path: str) -> bool:
        return path.startswith((self.route_prefix, *self.route_aliases))

    def locator(self, page: Any, key: str) -> Any:
        return page.locator(self.selectors[key])

    def text(self, page: Any, key: str, *, exact: bool = True) -> Any:
        return page.get_by_text(self.labels[key], exact=exact)

    async def require(self, page: Any, capability: str, *keys: str) -> dict[str, Any]:
        missing: list[str] = []
        counts: dict[str, int] = {}
        for key in keys:
            locator = self.locator(page, key)
            visible = 0
            for index in range(min(await locator.count(), 20)):
                if await locator.nth(index).is_visible():
                    visible += 1
            counts[key] = visible
            if visible == 0:
                missing.append(key)
        if missing:
            raise DoudouError(
                "PAGE_ADAPTER_DRIFTED",
                "The versioned page adapter no longer matches the controls required for this write.",
                {
                    "adapter": self.name,
                    "adapter_version": self.version,
                    "capability": capability,
                    "page_path": page.url.split("?", 1)[0],
                    "missing_controls": missing,
                    "visible_counts": counts,
                },
            )
        return counts


CHAT_ADAPTER = PageAdapter(
    name="douyin-chat",
    version="2026-08-02.2",
    route_prefix="/chat",
    route_aliases=(),
    selectors={
        "composer": '[data-e2e="msg-input"] [contenteditable="true"]',
        "message_item": '[data-e2e="msg-item-content"]',
        "image_upload": '[role="button"].semi-upload-add',
        "send_button": "svg.e2e-send-msg-btn",
    },
    labels={"reply": "回复", "send": "发送", "like": "点赞"},
)


VIDEO_ADAPTER = PageAdapter(
    name="douyin-video",
    version="2026-08-02.4",
    route_prefix="/video/",
    route_aliases=("/note/",),
    selectors={
        "render_data": "#RENDER_DATA",
        "inventory_controls": "button, [role=button], [data-e2e]",
        "comment_button": '[data-e2e="feed-comment-icon"], [data-e2e*="comment" i], button[aria-label*="评论"], [title*="评论"]',
        "comment_item": '[data-e2e="comment-item"]',
        "comment_container": ".comment-input-container",
        "comment_placeholder": ".public-DraftEditorPlaceholder-root:visible",
        "comment_send_controls": ".commentInput-right-ct span",
        "comment_editor": 'textarea, input, [contenteditable="true"]',
        "fallback_editor": '[contenteditable="true"][role="combobox"]:visible',
        "verification_mask": ".second-verify-mask",
        "like_count": "p",
    },
    labels={"tutorial": "我知道了", "reply": "回复"},
)


PROFILE_ADAPTER = PageAdapter(
    name="douyin-profile",
    version="2026-08-02.1",
    route_prefix="/user/",
    route_aliases=(),
    selectors={"scroll_container": ".route-scroll-container"},
    labels={},
)


AUTH_ADAPTER = PageAdapter(
    name="douyin-auth",
    version="2026-08-02.1",
    route_prefix="/",
    route_aliases=(),
    selectors={
        "slider": 'iframe[src*="captcha" i], [class*="captcha" i] [class*="slide" i], [class*="captcha" i] [class*="drag" i]',
        "sms_code": '.second-verify-mask input, input[placeholder*="验证码"], input[maxlength="6"]',
        "qr_login": '[class*="login" i] img[src*="qr" i], [class*="login" i] canvas[class*="qr" i], [class*="qrcode" i] img, [class*="qr-code" i] img',
        "generic_verification": 'iframe[src*="verify" i], .second-verify-mask:has(input), [role="dialog"][class*="verify" i]:has(button)',
    },
    labels={
        "quick_login": "快捷登录",
        "get_code": "获取验证码",
        "send_code": "发送验证码",
    },
)


MENTION_PICKER_BOUNDS_SCRIPT = """({name, minY, maxY}) => {
  const visible = element => { const style = getComputedStyle(element); const rect = element.getBoundingClientRect();
    return style.display !== 'none' && style.visibility !== 'hidden' && rect.width > 0 && rect.height > 0 && rect.y >= minY && rect.y < maxY; };
  const values = [...document.querySelectorAll('div,span,p')].filter(element => visible(element) && (element.textContent || '').trim() === name)
    .map(element => { const rect = element.getBoundingClientRect(); return {x: rect.x, y: rect.y, width: rect.width, height: rect.height, area: rect.width * rect.height}; })
    .sort((left, right) => left.area - right.area);
  return values[0] || null;
}"""


ADAPTERS = (AUTH_ADAPTER, CHAT_ADAPTER, PROFILE_ADAPTER, VIDEO_ADAPTER)
