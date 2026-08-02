from __future__ import annotations

from typing import Any

from ...common.errors import DoudouError


async def visible_candidates(locator: Any, limit: int = 40) -> list[Any]:
    return [
        locator.nth(index)
        for index in range(min(await locator.count(), limit))
        if await locator.nth(index).is_visible()
    ]


async def unique_visible(
    locator: Any,
    *,
    component: str,
    version: str,
    capability: str,
    strategy: str,
) -> Any:
    candidates = await visible_candidates(locator)
    if len(candidates) != 1:
        raise DoudouError(
            "PAGE_ADAPTER_DRIFTED",
            "The page component did not resolve to one visible target.",
            {
                "component": component,
                "component_version": version,
                "capability": capability,
                "strategy": strategy,
                "candidate_count": len(candidates),
            },
        )
    return candidates[0]


async def component_probe(
    locator: Any,
    *,
    component: str,
    version: str,
    capability: str,
    strategy: str,
    exercised: bool,
) -> dict[str, Any]:
    if not exercised:
        return {
            "component": component,
            "component_version": version,
            "capability": capability,
            "exercised": False,
            "status": "not_exercised",
            "strategy": strategy,
        }
    candidates = await visible_candidates(locator)
    enabled = [item for item in candidates if await item.is_enabled()]
    return {
        "component": component,
        "component_version": version,
        "capability": capability,
        "exercised": True,
        "status": "ok" if len(candidates) == 1 and len(enabled) == 1 else "drifted",
        "strategy": strategy,
        "candidate_count": len(candidates),
        "visible_count": len(candidates),
        "enabled_count": len(enabled),
    }
