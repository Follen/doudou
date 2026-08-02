from __future__ import annotations

import json
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path

from doudou.common.core import atomic_write_json
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

    def test_legacy_chromium_profile_is_atomically_backed_up(self) -> None:
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
            metadata, _paths = load_profile(root, "work")
            self.assertEqual(metadata["runtime"], "camoufox-python")
            backups = list((root / "profile").glob("work__chromium-backup-*"))
            self.assertEqual(len(backups), 1)
            self.assertTrue((backups[0] / "browser-data" / "Default").is_dir())
            self.assertTrue(
                any(
                    (root / "message" / "profiles" / "work" / "migrations").glob(
                        "*.json"
                    )
                )
            )


if __name__ == "__main__":
    unittest.main()
