from __future__ import annotations

from typing import Any

from ...common.errors import DoudouError
from .. import adapters
from .base import require_target, resolution_probe, visible_candidates


class AuthChallengeComponent:
    name = "auth-challenge"
    version = "2026-08-03.1"
    container_xpath = (
        "xpath=ancestor-or-self::*[@role='dialog' or contains(@class,'verify') "
        "or contains(@class,'captcha') or contains(@class,'login') "
        "or contains(@class,'second-verify-mask')][1]"
    )

    def __init__(self, page: Any, adapter: Any | None = None) -> None:
        self.page = page
        self.adapter = adapter or adapters.AUTH_ADAPTER

    async def _container(self, challenge: Any, capability: str) -> Any:
        return await require_target(
            challenge.locator(self.container_xpath),
            component=self.name,
            version=self.version,
            capability=capability,
            strategy="semantic auth challenge -> local dialog/mask container",
            scope=self.page.locator("body"),
        )

    async def _challenge(self, kind: str) -> tuple[Any, Any]:
        challenge = await require_target(
            self.adapter.locator(self.page, kind),
            component=self.name,
            version=self.version,
            capability=f"auth.{kind}",
            strategy=f"versioned semantic {kind} candidate",
        )
        container = await self._container(challenge, f"auth.{kind}")
        if kind == "generic_verification":
            text = (await container.inner_text(timeout=1000)).casefold()
            tag_name = await challenge.evaluate("element => element.tagName")
            if tag_name != "IFRAME" and not any(
                marker in text for marker in ("验证", "验证码", "安全", "滑块", "拖动")
            ):
                raise DoudouError(
                    "PAGE_ADAPTER_DRIFTED",
                    "The generic auth candidate lacks local verification evidence.",
                    {
                        "component": self.name,
                        "component_version": self.version,
                        "capability": "auth.generic_verification",
                        "strategy": "generic auth candidate -> local verification evidence",
                        "geometry_reason": "state_mismatch",
                    },
                )
        return challenge, container

    async def detect(self) -> str | None:
        for kind in (
            "sms_code",
            "slider",
            "qr_login",
            "passkey",
            "generic_verification",
        ):
            candidates = await visible_candidates(self.adapter.locator(self.page, kind))
            if not candidates:
                continue
            try:
                _challenge, container = await self._challenge(kind)
            except DoudouError as error:
                details = error.details or {}
                if (
                    error.code == "PAGE_ADAPTER_DRIFTED"
                    and details.get("candidate_count") == 0
                    and details.get("visible_count") == 0
                ):
                    continue
                raise
            if kind == "sms_code":
                quick = await visible_candidates(
                    container.get_by_text(
                        self.adapter.labels["quick_login"], exact=False
                    ),
                    limit=5,
                )
                if len(quick) > 1:
                    await require_target(
                        container.get_by_text(
                            self.adapter.labels["quick_login"], exact=False
                        ),
                        component=self.name,
                        version=self.version,
                        capability="auth.quick_login_sms",
                        strategy="SMS dialog -> quick-login semantic label",
                        scope=container,
                    )
                if len(quick) == 1:
                    return "quick_login_sms"
            return kind
        return None

    async def request_sms_code(self) -> bool:
        kind = await self.detect()
        if kind not in {"sms_code", "quick_login_sms"}:
            return False
        challenge = await require_target(
            self.adapter.locator(self.page, "sms_code"),
            component=self.name,
            version=self.version,
            capability="auth.sms_code",
            strategy="versioned SMS input candidate",
        )
        container = await self._container(challenge, "auth.sms_code.request")
        controls = container.get_by_text(
            self.adapter.labels["get_code"], exact=False
        ).or_(container.get_by_text(self.adapter.labels["send_code"], exact=False))
        control = await require_target(
            controls,
            component=self.name,
            version=self.version,
            capability="auth.sms_code.request",
            strategy="SMS dialog -> local get/send-code action",
            require_enabled=True,
            scope=container,
        )
        await control.click(timeout=10_000)
        return True

    async def probe(self, exercised: bool = True) -> list[dict[str, Any]]:
        reports = []
        for kind in (
            "sms_code",
            "slider",
            "qr_login",
            "passkey",
            "generic_verification",
        ):
            visible = bool(
                exercised
                and await visible_candidates(self.adapter.locator(self.page, kind))
            )

            async def resolve(kind: str = kind) -> Any:
                _challenge, container = await self._challenge(kind)
                return container

            reports.append(
                await resolution_probe(
                    resolve,
                    component=self.name,
                    version=self.version,
                    capability=f"auth.{kind}",
                    strategy=f"versioned semantic {kind} -> local auth container",
                    exercised=visible,
                    not_exercised_reason="challenge_not_present",
                )
            )
        return reports
