from __future__ import annotations

import json
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path

from doudou.common.core import atomic_write_json
from doudou.common.errors import DoudouError
from doudou.runtime.browser.session import BrowserSession
from doudou.runtime.profile import create_profile, list_profiles, load_profile


class ProfileTests(unittest.TestCase):
    def test_browser_health_state_updates_profile_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            _metadata, paths = create_profile(root, "work")
            session = BrowserSession(paths, fake=True)
            session._record_login_state(
                {
                    "status": "valid",
                    "reason_code": None,
                }
            )
            updated, _ = load_profile(root, "work")
            self.assertEqual(updated["login_status"], "valid")
            self.assertIsNotNone(updated["last_login_verified_at"])

    def test_profile_and_message_data_stay_in_calling_project(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            metadata, paths = create_profile(root, "工作/账号", "工作账号")
            self.assertEqual(metadata["profile_id"], "工作~002F账号")
            self.assertTrue(paths.browser_data.is_dir())
            self.assertTrue(paths.message.is_dir())
            self.assertEqual(
                load_profile(root, "工作/账号")[0]["display_name"], "工作账号"
            )
            self.assertEqual(len(list_profiles(root)), 1)
            self.assertTrue(paths.camoufox_config.is_file())

    def test_same_profile_name_in_two_projects_is_isolated(self) -> None:
        with (
            tempfile.TemporaryDirectory() as left,
            tempfile.TemporaryDirectory() as right,
        ):
            left_paths = create_profile(Path(left), "work")[1]
            right_paths = create_profile(Path(right), "work")[1]
            self.assertNotEqual(left_paths.profile, right_paths.profile)
            self.assertNotEqual(left_paths.message, right_paths.message)

    def test_profile_list_reports_non_sensitive_runtime_and_data_summaries(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            _metadata, paths = create_profile(root, "work")
            atomic_write_json(
                paths.message / "conversations.json",
                {"count": 2, "conversations": []},
            )
            atomic_write_json(
                paths.session_index / "latest.json",
                {"completed_at": "2026-08-02T00:00:00.000Z"},
            )
            (paths.conversations / "group__one__1").mkdir()
            (paths.message / "content" / "users" / "user-1").mkdir(parents=True)
            (paths.message / "content" / "videos" / "video-1").mkdir(parents=True)
            receipt = paths.sends / "2026-08-02" / "write-1" / "receipt.json"
            atomic_write_json(receipt, {"status": "sent"})

            listed = list_profiles(root)[0]

            self.assertEqual(listed["lock_status"], "free")
            self.assertEqual(
                listed["last_session_index_refresh"], "2026-08-02T00:00:00.000Z"
            )
            self.assertEqual(listed["chat_summary"]["conversation_count"], 2)
            self.assertEqual(listed["chat_summary"]["stored_conversation_count"], 1)
            self.assertEqual(listed["content_summary"], {"user_count": 1, "video_count": 1})
            self.assertIsNotNone(listed["recent_write_at"])
            datetime.fromisoformat(listed["recent_write_at"].replace("Z", "+00:00")).astimezone(timezone.utc)

    def test_legacy_profile_root_is_rejected_without_mutation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            legacy = root / "Profile"
            legacy.mkdir()
            marker = legacy / "marker.txt"
            marker.write_text("legacy", encoding="utf-8")

            with self.assertRaises(DoudouError) as context:
                create_profile(root, "work")

            self.assertEqual(context.exception.code, "LEGACY_UNSUPPORTED")
            self.assertEqual(context.exception.details["legacy_format"], "profile_root")
            self.assertEqual(marker.read_text(encoding="utf-8"), "legacy")
            self.assertNotIn("profile", {item.name for item in root.iterdir()})
            self.assertFalse((root / "message").exists())

    def test_legacy_chromium_profile_is_rejected_without_mutation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            legacy = root / "profile" / "work"
            (legacy / "browser-data" / "Default").mkdir(parents=True)
            (legacy / "profile.json").write_text(
                json.dumps(
                    {
                        "schema": "doudou.profile.v1",
                        "profile_id": "work",
                        "display_name": "Work",
                    }
                ),
                encoding="utf-8",
            )
            original = (legacy / "profile.json").read_bytes()

            with self.assertRaises(DoudouError) as context:
                load_profile(root, "work")

            self.assertEqual(context.exception.code, "LEGACY_UNSUPPORTED")
            self.assertEqual(
                context.exception.details["legacy_format"], "browser_profile"
            )
            self.assertEqual((legacy / "profile.json").read_bytes(), original)
            self.assertTrue((legacy / "browser-data" / "Default").is_dir())
            self.assertEqual(list((root / "profile").glob("*__chromium-backup-*")), [])
            self.assertFalse((root / "message").exists())

    def test_legacy_backup_coexists_without_blocking_current_profiles(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            current, _paths = create_profile(root, "current")
            backup = root / "profile" / "old__chromium-backup-20260101T000000Z"
            backup.mkdir()
            (backup / "profile.json").write_text(
                json.dumps(
                    {
                        "schema": "doudou.profile.v1",
                        "profile_id": backup.name,
                    }
                ),
                encoding="utf-8",
            )

            self.assertEqual(
                [profile["profile_id"] for profile in list_profiles(root)],
                [current["profile_id"]],
            )
            self.assertEqual(
                load_profile(root, "current")[0]["schema"], "doudou.profile.v2"
            )

            with self.assertRaises(DoudouError) as context:
                load_profile(root, backup.name)
            self.assertEqual(context.exception.code, "LEGACY_UNSUPPORTED")
            self.assertTrue((backup / "profile.json").exists())

    def test_create_does_not_overwrite_selected_legacy_profile(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            legacy = root / "profile" / "work"
            legacy.mkdir(parents=True)
            metadata = legacy / "profile.json"
            metadata.write_text(
                json.dumps({"schema": "doudou.profile.v1", "profile_id": "work"}),
                encoding="utf-8",
            )
            original = metadata.read_bytes()

            with self.assertRaises(DoudouError) as context:
                create_profile(root, "work")
            self.assertEqual(context.exception.code, "LEGACY_UNSUPPORTED")
            self.assertEqual(metadata.read_bytes(), original)

    def test_current_v1_identity_config_is_not_treated_as_legacy(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            _metadata, paths = create_profile(root, "work")

            self.assertEqual(
                json.loads(paths.camoufox_config.read_text(encoding="utf-8"))["schema"],
                "doudou.camoufox-config.v1",
            )
            self.assertEqual(load_profile(root, "work")[0]["schema"], "doudou.profile.v2")


if __name__ == "__main__":
    unittest.main()
