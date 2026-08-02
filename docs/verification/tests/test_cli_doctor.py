from __future__ import annotations

import unittest
from pathlib import Path
from unittest.mock import patch

from doudou.cli import dispatch, parser


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
