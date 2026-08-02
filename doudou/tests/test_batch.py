from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest.mock import AsyncMock, patch

from doudou.common.errors import DoudouError
from doudou.features.chat.service import execute_batch
from doudou.writes.plans import create_plan


class FakeClock:
    def __init__(self) -> None:
        self.now = 0.0
        self.sleeps: list[float] = []

    async def sleep(self, seconds: float) -> None:
        self.sleeps.append(seconds)
        self.now += seconds


def batch_plan(root: Path, task_count: int) -> dict[str, object]:
    tasks = [
        {
            "conversation": {
                "conversation_id": str(index + 1),
                "conversation_short_id": str(index + 1),
                "conversation_type": 2,
            },
            "mode": "text",
            "text": "hello",
        }
        for index in range(task_count)
    ]
    return create_plan(
        root,
        "work",
        "chat.send.batch",
        {"tasks": tasks, "interval_seconds": 5.0},
    )


class BatchTests(unittest.IsolatedAsyncioTestCase):
    async def test_successful_attempts_are_spaced_by_at_least_five_seconds(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            plan = batch_plan(root, 3)
            clock = FakeClock()
            attempts: list[float] = []

            async def send(_session: object, _plan: dict[str, object]) -> dict[str, str]:
                attempts.append(clock.now)
                return {"status": "sent", "server_message_id": str(len(attempts))}

            with (
                patch(
                    "doudou.features.chat.service.send_message",
                    new=AsyncMock(side_effect=send),
                ),
                patch(
                    "doudou.features.chat.service._collect_readback",
                    new=AsyncMock(return_value=({"run_id": "run"}, [])),
                ),
                patch(
                    "doudou.features.chat.service.verify_send_readback",
                    return_value={"passed": True},
                ),
                patch(
                    "doudou.features.chat.service.write_receipt",
                    return_value={"receipt": "receipt.json"},
                ),
                patch("doudou.features.chat.service.asyncio.sleep", new=clock.sleep),
            ):
                result = await execute_batch(
                    object(), root, "work", {"plan_id": plan["plan_id"]}
                )

            self.assertEqual(result["status"], "sent")
            self.assertEqual(attempts, [0.0, 5.0, 10.0])
            self.assertEqual(clock.sleeps, [5.0, 5.0])

    async def test_first_failure_stops_and_marks_remaining_items_skipped(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            plan = batch_plan(root, 3)
            clock = FakeClock()
            attempts: list[float] = []

            async def send(_session: object, _plan: dict[str, object]) -> dict[str, str]:
                attempts.append(clock.now)
                if len(attempts) == 2:
                    raise DoudouError("TEST_WRITE_FAILED", "fixture failure")
                return {"status": "sent", "server_message_id": "1"}

            with (
                patch(
                    "doudou.features.chat.service.send_message",
                    new=AsyncMock(side_effect=send),
                ),
                patch(
                    "doudou.features.chat.service._collect_readback",
                    new=AsyncMock(return_value=({"run_id": "run"}, [])),
                ),
                patch(
                    "doudou.features.chat.service.verify_send_readback",
                    return_value={"passed": True},
                ),
                patch(
                    "doudou.features.chat.service.write_receipt",
                    return_value={"receipt": "receipt.json"},
                ),
                patch("doudou.features.chat.service.asyncio.sleep", new=clock.sleep),
            ):
                result = await execute_batch(
                    object(), root, "work", {"plan_id": plan["plan_id"]}
                )

            self.assertEqual(result["status"], "failed")
            self.assertEqual(attempts, [0.0, 5.0])
            self.assertEqual(clock.sleeps, [5.0])
            self.assertEqual(
                [item["status"] for item in result["results"]],
                ["sent", "failed", "skipped"],
            )


if __name__ == "__main__":
    unittest.main()
