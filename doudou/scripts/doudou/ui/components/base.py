from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable
from typing import Any, Literal

from ...common.errors import DoudouError


async def visible_candidates(locator: Any, limit: int = 40) -> list[Any]:
    return [
        locator.nth(index)
        for index in range(min(await locator.count(), limit))
        if await locator.nth(index).is_visible()
    ]


def _drift(
    message: str,
    *,
    component: str,
    version: str,
    capability: str,
    strategy: str,
    **details: Any,
) -> DoudouError:
    return DoudouError(
        "PAGE_ADAPTER_DRIFTED",
        message,
        {
            "component": component,
            "component_version": version,
            "capability": capability,
            "strategy": strategy,
            **details,
        },
    )


def _valid_box(box: Any) -> bool:
    return bool(
        isinstance(box, dict)
        and box.get("width", 0) > 0
        and box.get("height", 0) > 0
    )


def _box_delta(first: dict[str, float], second: dict[str, float]) -> float:
    return max(abs(float(first[key]) - float(second[key])) for key in first)


async def click_verified_target(
    page: Any,
    target: Any,
    *,
    component: str,
    version: str,
    capability: str,
    strategy: str,
    stability_tolerance: float = 1.0,
    scroll_into_view: bool = True,
) -> dict[str, Any]:
    """Click a verified DOM target using its measured center, never fixed coordinates."""
    try:
        if scroll_into_view:
            await target.scroll_into_view_if_needed(timeout=5_000)
        first_box = await target.bounding_box()
        await page.wait_for_timeout(100)
        second_box = await target.bounding_box()
    except Exception as error:
        raise _drift(
            "The page target could not be prepared for a normal mouse click.",
            component=component,
            version=version,
            capability=capability,
            strategy=strategy,
            geometry_reason="target_actionability_failed",
            cause_type=type(error).__name__,
        ) from error
    if not _valid_box(first_box) or not _valid_box(second_box):
        raise _drift(
            "The page target lost its usable geometry before the click.",
            component=component,
            version=version,
            capability=capability,
            strategy=strategy,
            geometry_reason="target_box_missing_before_click",
        )
    delta = _box_delta(first_box, second_box)
    if delta > stability_tolerance:
        raise _drift(
            "The page target was still moving before the click.",
            component=component,
            version=version,
            capability=capability,
            strategy=strategy,
            geometry_reason="target_unstable",
            box_delta=round(delta, 3),
        )
    hit = await target.evaluate(
        """element => {
          const rect = element.getBoundingClientRect();
          const x = rect.left + rect.width / 2;
          const y = rect.top + rect.height / 2;
          const top = document.elementFromPoint(x, y);
          const blocker = top && !element.contains(top) ? top : null;
          return {
            receives_events: Boolean(top && (top === element || element.contains(top))),
            blocker: blocker ? {
              tag: blocker.tagName,
              class_name: String(blocker.className || '').slice(0, 160),
            } : null,
          };
        }"""
    )
    if not hit.get("receives_events"):
        raise _drift(
            "The page target is covered and cannot receive the normal mouse click.",
            component=component,
            version=version,
            capability=capability,
            strategy=strategy,
            geometry_reason="target_occluded",
            blocker=hit.get("blocker"),
        )
    center_x = second_box["x"] + second_box["width"] / 2
    center_y = second_box["y"] + second_box["height"] / 2
    dispatch_completed = True
    try:
        async with asyncio.timeout(5):
            await page.mouse.click(center_x, center_y)
    except TimeoutError:
        # Hidden headed Firefox can apply the input event without acknowledging the
        # protocol command. Callers must decide success from their state postcondition.
        dispatch_completed = False
    return {
        "stable": True,
        "box_delta": round(delta, 3),
        "receives_events": True,
        "dispatch_completed": dispatch_completed,
        "click_point": {"x": round(center_x, 3), "y": round(center_y, 3)},
    }


def _contains(scope: dict[str, float], target: dict[str, float], tolerance: int = 2) -> bool:
    return (
        target["x"] >= scope["x"] - tolerance
        and target["y"] >= scope["y"] - tolerance
        and target["x"] + target["width"]
        <= scope["x"] + scope["width"] + tolerance
        and target["y"] + target["height"]
        <= scope["y"] + scope["height"] + tolerance
    )


async def require_target(
    locator: Any,
    *,
    component: str,
    version: str,
    capability: str,
    strategy: str,
    require_enabled: bool = False,
    scope: Any | None = None,
    peers: Any | None = None,
    position: Literal["rightmost", "leftmost"] | None = None,
    minimum_peer_count: int = 1,
    anchor: Any | None = None,
    relation: Literal["above", "below", "left_of", "right_of"] | None = None,
    max_anchor_distance: float | None = None,
    filter_by_geometry: bool = False,
    scope_tolerance: int = 2,
) -> Any:
    total_count = min(await locator.count(), 40)
    candidates = await visible_candidates(locator)
    scope_box = await scope.bounding_box() if scope is not None else None
    anchor_box = await anchor.bounding_box() if anchor is not None else None
    if filter_by_geometry:
        filtered: list[Any] = []
        for item in candidates:
            box = await item.bounding_box()
            if not _valid_box(box):
                continue
            if scope is not None and (
                not _valid_box(scope_box)
                or not _contains(scope_box, box, tolerance=scope_tolerance)
            ):
                continue
            if anchor is not None:
                if not _valid_box(anchor_box):
                    continue
                item_center = (
                    box["x"] + box["width"] / 2,
                    box["y"] + box["height"] / 2,
                )
                anchor_center = (
                    anchor_box["x"] + anchor_box["width"] / 2,
                    anchor_box["y"] + anchor_box["height"] / 2,
                )
                relation_ok = {
                    None: True,
                    "above": item_center[1] < anchor_center[1],
                    "below": item_center[1] > anchor_center[1],
                    "left_of": item_center[0] < anchor_center[0],
                    "right_of": item_center[0] > anchor_center[0],
                }[relation]
                distance = (
                    (item_center[0] - anchor_center[0]) ** 2
                    + (item_center[1] - anchor_center[1]) ** 2
                ) ** 0.5
                if not relation_ok or (
                    max_anchor_distance is not None
                    and distance > max_anchor_distance
                ):
                    continue
            filtered.append(item)
        candidates = filtered
    enabled = [item for item in candidates if await item.is_enabled()]
    if len(candidates) != 1:
        raise _drift(
            "The page component did not resolve to one visible target.",
            component=component,
            version=version,
            capability=capability,
            strategy=strategy,
            candidate_count=total_count,
            visible_count=len(candidates),
            enabled_count=len(enabled),
            geometry_reason="target_not_unique",
        )
    target = candidates[0]
    if require_enabled and len(enabled) != 1:
        raise _drift(
            "The page component target is not enabled.",
            component=component,
            version=version,
            capability=capability,
            strategy=strategy,
            candidate_count=total_count,
            visible_count=1,
            enabled_count=0,
            geometry_reason="target_disabled",
        )
    target_box = await target.bounding_box()
    if not _valid_box(target_box):
        raise _drift(
            "The page component target has no usable geometry.",
            component=component,
            version=version,
            capability=capability,
            strategy=strategy,
            candidate_count=total_count,
            visible_count=1,
            enabled_count=len(enabled),
            geometry_reason="target_box_missing",
        )
    if scope is not None and (
        not _valid_box(scope_box)
        or not _contains(scope_box, target_box, tolerance=scope_tolerance)
    ):
        raise _drift(
            "The page component target is outside its confirmed local scope.",
            component=component,
            version=version,
            capability=capability,
            strategy=strategy,
            candidate_count=total_count,
            visible_count=1,
            enabled_count=len(enabled),
            geometry_reason="outside_local_scope",
        )
    if peers is not None and position is not None:
        peer_items = await visible_candidates(peers)
        peer_boxes = [await item.bounding_box() for item in peer_items]
        peer_boxes = [box for box in peer_boxes if _valid_box(box)]
        if len(peer_boxes) < minimum_peer_count:
            raise _drift(
                "The local component actions have no usable geometry.",
                component=component,
                version=version,
                capability=capability,
                strategy=strategy,
                candidate_count=total_count,
                visible_count=1,
                enabled_count=len(enabled),
                geometry_reason="peer_boxes_missing",
                peer_count=len(peer_boxes),
                minimum_peer_count=minimum_peer_count,
            )
        expected = (
            max(box["x"] for box in peer_boxes)
            if position == "rightmost"
            else min(box["x"] for box in peer_boxes)
        )
        if abs(target_box["x"] - expected) > 2:
            raise _drift(
                f"The page component target is not the expected {position} action.",
                component=component,
                version=version,
                capability=capability,
                strategy=strategy,
                candidate_count=total_count,
                visible_count=1,
                enabled_count=len(enabled),
                geometry_reason=f"not_{position}",
            )
    if anchor is not None:
        if not _valid_box(anchor_box):
            raise _drift(
                "The page component anchor has no usable geometry.",
                component=component,
                version=version,
                capability=capability,
                strategy=strategy,
                geometry_reason="anchor_box_missing",
            )
        target_center = (
            target_box["x"] + target_box["width"] / 2,
            target_box["y"] + target_box["height"] / 2,
        )
        anchor_center = (
            anchor_box["x"] + anchor_box["width"] / 2,
            anchor_box["y"] + anchor_box["height"] / 2,
        )
        relation_ok = {
            None: True,
            "above": target_center[1] < anchor_center[1],
            "below": target_center[1] > anchor_center[1],
            "left_of": target_center[0] < anchor_center[0],
            "right_of": target_center[0] > anchor_center[0],
        }[relation]
        distance = (
            (target_center[0] - anchor_center[0]) ** 2
            + (target_center[1] - anchor_center[1]) ** 2
        ) ** 0.5
        if not relation_ok or (
            max_anchor_distance is not None and distance > max_anchor_distance
        ):
            raise _drift(
                "The page component target violates its anchor geometry contract.",
                component=component,
                version=version,
                capability=capability,
                strategy=strategy,
                candidate_count=total_count,
                visible_count=1,
                enabled_count=len(enabled),
                geometry_reason="anchor_relation_mismatch",
            )
    return target


async def unique_visible(
    locator: Any,
    *,
    component: str,
    version: str,
    capability: str,
    strategy: str,
) -> Any:
    return await require_target(
        locator,
        component=component,
        version=version,
        capability=capability,
        strategy=strategy,
    )


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
    total_count = min(await locator.count(), 40)
    candidates = await visible_candidates(locator)
    enabled = [item for item in candidates if await item.is_enabled()]
    box = await candidates[0].bounding_box() if len(candidates) == 1 else None
    geometry_valid = _valid_box(box)
    reason = None
    if len(candidates) != 1:
        reason = "target_not_unique"
    elif len(enabled) != 1:
        reason = "target_disabled"
    elif not geometry_valid:
        reason = "target_box_missing"
    return {
        "component": component,
        "component_version": version,
        "capability": capability,
        "exercised": True,
        "status": "ok"
        if len(candidates) == 1 and len(enabled) == 1 and geometry_valid
        else "drifted",
        "strategy": strategy,
        "candidate_count": total_count,
        "visible_count": len(candidates),
        "enabled_count": len(enabled),
        "unique": len(candidates) == 1,
        "geometry_valid": geometry_valid,
        "geometry_reason": reason,
    }


def not_exercised_probe(
    *,
    component: str,
    version: str,
    capability: str,
    strategy: str,
    reason: str,
) -> dict[str, Any]:
    return {
        "component": component,
        "component_version": version,
        "capability": capability,
        "exercised": False,
        "status": "not_exercised",
        "strategy": strategy,
        "reason": reason,
    }


async def resolution_probe(
    resolver: Callable[[], Awaitable[Any]],
    *,
    component: str,
    version: str,
    capability: str,
    strategy: str,
    exercised: bool,
    not_exercised_reason: str,
) -> dict[str, Any]:
    if not exercised:
        return not_exercised_probe(
            component=component,
            version=version,
            capability=capability,
            strategy=strategy,
            reason=not_exercised_reason,
        )
    try:
        target = await resolver()
        box = await target.bounding_box()
        enabled = await target.is_enabled()
        return {
            "component": component,
            "component_version": version,
            "capability": capability,
            "exercised": True,
            "status": "ok",
            "strategy": strategy,
            "candidate_count": 1,
            "visible_count": 1,
            "enabled_count": int(enabled),
            "unique": True,
            "geometry_valid": _valid_box(box),
            "geometry_reason": None,
        }
    except DoudouError as error:
        if error.code != "PAGE_ADAPTER_DRIFTED":
            raise
        return {
            "component": component,
            "component_version": version,
            "capability": capability,
            "exercised": True,
            "status": "drifted",
            "strategy": strategy,
            **error.details,
        }
