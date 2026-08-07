from __future__ import annotations

import asyncio
import json
import tempfile
import unittest
from collections.abc import Awaitable, Callable
from pathlib import Path
from unittest.mock import patch

from doudou.common.errors import DoudouError
from doudou.runtime.broker.client import (
    BROKER_RESPONSE_LIMIT_BYTES,
    BrokerClient,
)
from doudou.runtime.profile import create_profile


class BrokerFrameLimitTests(unittest.TestCase):
    def _run_server(
        self,
        response: bytes,
        request: Callable[[BrokerClient], Awaitable[dict]],
    ) -> tuple[dict | None, DoudouError | None, bool]:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            create_profile(root, "work")
            client = BrokerClient(root, "work")

            async def exercise() -> tuple[dict | None, DoudouError | None, bool]:
                connection_closed = asyncio.Event()

                async def handler(
                    reader: asyncio.StreamReader, writer: asyncio.StreamWriter
                ) -> None:
                    try:
                        await reader.readline()
                        writer.write(response)
                        try:
                            await writer.drain()
                            await reader.read()
                        except (ConnectionError, OSError):
                            pass
                    finally:
                        writer.close()
                        try:
                            await writer.wait_closed()
                        except (ConnectionError, OSError):
                            pass
                        connection_closed.set()

                server = await asyncio.start_server(handler, "127.0.0.1", 0)
                port = server.sockets[0].getsockname()[1]
                descriptor = {
                    "endpoint": {"host": "127.0.0.1", "port": port}
                }
                result = None
                raised = None
                try:
                    with (
                        patch.object(client, "_descriptor", return_value=descriptor),
                        patch.object(client, "_token", return_value="test-token"),
                    ):
                        try:
                            result = await request(client)
                        except DoudouError as error:
                            raised = error
                    await asyncio.wait_for(connection_closed.wait(), timeout=2)
                finally:
                    server.close()
                    await server.wait_closed()
                return result, raised, connection_closed.is_set()

            return asyncio.run(exercise())

    def test_accepts_response_larger_than_128_kib(self) -> None:
        blob = "x" * (128 * 1024)
        response = (
            json.dumps({"ok": True, "result": {"blob": blob}}).encode() + b"\n"
        )

        async def request(client: BrokerClient) -> dict:
            return await client.request_async("large.read", start=False)

        result, raised, connection_closed = self._run_server(response, request)

        self.assertIsNone(raised)
        self.assertEqual(result["result"]["blob"], blob)
        self.assertTrue(connection_closed)

    def test_read_response_over_limit_has_stable_error(self) -> None:
        response = b"x" * (BROKER_RESPONSE_LIMIT_BYTES + 1) + b"\n"

        async def request(client: BrokerClient) -> dict:
            return await client.request_async("oversized.read", start=False)

        result, raised, connection_closed = self._run_server(response, request)

        self.assertIsNone(result)
        self.assertIsNotNone(raised)
        self.assertEqual(raised.code, "BROKER_RESPONSE_TOO_LARGE")
        self.assertEqual(
            raised.details["response_limit_bytes"], BROKER_RESPONSE_LIMIT_BYTES
        )
        self.assertTrue(connection_closed)

    def test_write_response_over_limit_is_unknown_and_not_replayed(self) -> None:
        response = b"x" * (BROKER_RESPONSE_LIMIT_BYTES + 1) + b"\n"
        request_count = 0

        async def request(client: BrokerClient) -> dict:
            nonlocal request_count
            request_count += 1
            return await client.request_async(
                "oversized.write",
                semantics="write",
                request_id="write-large-frame",
                start=False,
            )

        result, raised, connection_closed = self._run_server(response, request)

        self.assertIsNone(result)
        self.assertIsNotNone(raised)
        self.assertEqual(raised.code, "WRITE_RESULT_UNKNOWN")
        self.assertEqual(raised.details["request_id"], "write-large-frame")
        self.assertTrue(raised.details["write_started"])
        self.assertEqual(request_count, 1)
        self.assertTrue(connection_closed)


if __name__ == "__main__":
    unittest.main()
