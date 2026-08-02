from __future__ import annotations

import unittest
from typing import Any

from doudou.common.errors import DoudouError
from doudou.ui.components.base import component_probe, require_target


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


class FakeLocator:
    def __init__(self, *items: FakeElement) -> None:
        self.items = items

    async def count(self) -> int:
        return len(self.items)

    def nth(self, index: int) -> FakeElement:
        return self.items[index]


def box(x: float, y: float, width: float = 20, height: float = 20) -> dict[str, float]:
    return {"x": x, "y": y, "width": width, "height": height}


class ComponentGeometryTests(unittest.IsolatedAsyncioTestCase):
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

    async def test_wrong_relative_position_fails_closed(self) -> None:
        target = FakeElement(box(200, 20))
        await self.assert_drift(
            FakeLocator(target),
            "not_rightmost",
            peers=FakeLocator(target, FakeElement(box(260, 20))),
            position="rightmost",
            minimum_peer_count=2,
        )

    async def test_anchor_filter_selects_only_local_mention_candidate(self) -> None:
        editor = FakeElement(box(200, 500, 400, 50))
        local = FakeElement(box(220, 400, 120, 30))
        unrelated = FakeElement(box(40, 40, 120, 30))
        resolved = await require_target(
            FakeLocator(unrelated, local),
            component="chat-composer",
            version="test.v1",
            capability="chat.mention",
            strategy="exact label -> composer-bounded candidate",
            require_enabled=True,
            anchor=editor,
            relation="above",
            max_anchor_distance=450,
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


if __name__ == "__main__":
    unittest.main()
