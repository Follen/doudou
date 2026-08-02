from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from doudou.runtime.broker import registration
from doudou.runtime.paths import profile_paths


class BrokerRegistrationTests(unittest.TestCase):
    def test_managed_runtime_broker_is_registered_and_unregistered(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            runtimes = root / "cache" / "doudou" / "runtimes"
            interpreter = runtimes / "runtime-1"
            executable = interpreter / ("Scripts/python.exe")
            executable.parent.mkdir(parents=True)
            executable.write_bytes(b"")
            paths = profile_paths(root / "project", "work")
            descriptor = {"pid": 123, "process_start": "start"}

            with (
                patch.object(registration, "runtime_root", return_value=runtimes),
                patch.object(registration.sys, "prefix", str(interpreter)),
                patch.object(registration.sys, "executable", str(executable)),
            ):
                registered = registration.register_broker(paths, descriptor)

            self.assertIsNotNone(registered)
            assert registered is not None
            value = json.loads(registered.read_text(encoding="utf-8"))
            self.assertEqual(value["schema"], "doudou.broker-registration.v1")
            self.assertEqual(value["runtime"], str(interpreter.resolve()))
            self.assertEqual(value["descriptor"], str(paths.broker_descriptor.resolve()))
            registration.unregister_broker(registered)
            self.assertFalse(registered.exists())

    def test_unmanaged_python_does_not_create_registration(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            paths = profile_paths(root / "project", "work")
            with (
                patch.object(
                    registration,
                    "runtime_root",
                    return_value=root / "cache" / "doudou" / "runtimes",
                ),
                patch.object(registration.sys, "prefix", str(root / "system-python")),
            ):
                registered = registration.register_broker(
                    paths, {"pid": 123, "process_start": "start"}
                )
            self.assertIsNone(registered)


if __name__ == "__main__":
    unittest.main()
