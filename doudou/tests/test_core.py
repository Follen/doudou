from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from doudou.common.core import atomic_write_jsonl, read_jsonl, safe_segment
from doudou.common.errors import DoudouError


class CoreTests(unittest.TestCase):
    def test_safe_segment_matches_existing_contract(self) -> None:
        self.assertEqual(safe_segment("产品/讨论"), "产品~002F讨论")
        self.assertEqual(safe_segment(".."), "~2e2e")
        self.assertEqual(safe_segment("a:b"), "a~003Ab")

    def test_jsonl_rejects_incomplete_tail(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            filename = Path(directory) / "records.jsonl"
            atomic_write_jsonl(filename, [{"a": 1}, {"a": 2}])
            self.assertEqual(read_jsonl(filename), [{"a": 1}, {"a": 2}])
            filename.write_text('{"a":1}', encoding="utf-8")
            with self.assertRaises(DoudouError) as context:
                read_jsonl(filename)
            self.assertEqual(context.exception.code, "INCOMPLETE_JSONL_LINE")


if __name__ == "__main__":
    unittest.main()
