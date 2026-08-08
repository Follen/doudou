from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from doudou.cli import dispatch, parser
from doudou.common.errors import DoudouError


class DoctorCliTests(unittest.TestCase):
    def test_doctor_does_not_launch_browser_by_default(self) -> None:
        args = parser().parse_args(["--data-root", "data", "doctor"])
        with (
            patch("doudou.cli.initialize_data_root"),
            patch("doudou.cli.doctor", return_value={"status": "ok"}) as doctor,
        ):
            dispatch(args, Path("skill"))
        doctor.assert_called_once_with(Path("data").resolve(), launch_probe=False)

    def test_doctor_launch_probe_is_explicit(self) -> None:
        args = parser().parse_args(
            ["--data-root", "data", "doctor", "--launch-probe"]
        )
        with (
            patch("doudou.cli.initialize_data_root"),
            patch("doudou.cli.doctor", return_value={"status": "ok"}) as doctor,
        ):
            dispatch(args, Path("skill"))
        doctor.assert_called_once_with(Path("data").resolve(), launch_probe=True)

    def test_doctor_rejects_legacy_profile_before_probe(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Profile").mkdir()
            args = parser().parse_args(["--data-root", str(root), "doctor"])
            with (
                patch("doudou.cli.doctor") as doctor,
                self.assertRaises(DoudouError) as context,
            ):
                dispatch(args, Path("skill"))
        self.assertEqual(context.exception.code, "LEGACY_UNSUPPORTED")
        doctor.assert_not_called()

    def test_identity_legacy_record_fails_before_broker_start(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            filename = (
                root
                / "message"
                / "profiles"
                / "profile-a"
                / "conversations"
                / "fixture"
                / "messages.jsonl"
            )
            filename.parent.mkdir(parents=True)
            filename.write_text(
                json.dumps({"sender": {"uid": "1", "sec_uid": "SEC"}}) + "\n",
                encoding="utf-8",
            )
            args = parser().parse_args(
                [
                    "--data-root",
                    str(root),
                    "identity",
                    "resolve",
                    "--profile",
                    "profile-a",
                    "--douyin-id",
                    "target",
                ]
            )
            with (
                patch("doudou.cli._client") as client,
                self.assertRaises(DoudouError) as context,
            ):
                dispatch(args, Path("skill"))
        self.assertEqual(context.exception.code, "LEGACY_UNSUPPORTED")
        client.assert_not_called()

    def test_identity_migrate_is_not_a_cli_command(self) -> None:
        with self.assertRaises(SystemExit):
            parser().parse_args(
                ["identity", "migrate", "--profile", "profile-a"]
            )

    def test_legacy_at_fails_before_broker_start(self) -> None:
        args = parser().parse_args(
            [
                "--data-root",
                "data",
                "protocol",
                "send",
                "--profile",
                "profile-a",
                "--recipient",
                "group-a",
                "--text",
                "hello",
                "--at",
                "member-a",
            ]
        )
        with (
            patch("doudou.cli.initialize_data_root"),
            patch("doudou.cli._client") as client,
            self.assertRaises(DoudouError) as context,
        ):
            dispatch(args, Path("skill"))
        self.assertEqual(context.exception.code, "MENTION_UNSUPPORTED")
        client.assert_not_called()
