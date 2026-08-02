from __future__ import annotations

import tempfile
import unittest
from datetime import datetime
from pathlib import Path

from doudou.cli import parser
from doudou.common.errors import DoudouError
from doudou.models.resolver import default_message_window, infer_self_uid
from doudou.protocol.contracts import WEB_PROTOCOL
from doudou.protocol.im import property_has_user
from doudou.protocol.web import (
    identities_in,
    normalize_comment,
    normalize_video,
    normalize_video_detail,
    parse_render_data,
    resolve_comment_parents,
)
from doudou.runtime.profile import create_profile
from doudou.writes.plans import confirm_plan, create_plan


class WebPlansCliTests(unittest.TestCase):
    def test_self_uid_and_message_reaction_are_derived_from_protocol_records(self) -> None:
        conversations = [
            {
                "conversation_type": 1,
                "participants": {"participants": [{"uid": "self"}, {"uid": "a"}]},
            },
            {
                "conversation_type": 1,
                "participants": {"participants": [{"uid": "self"}, {"uid": "b"}]},
            },
        ]
        self.assertEqual(infer_self_uid(conversations), "self")
        self.assertTrue(
            property_has_user({"e:love": {"items": [{"uid": "self"}]}}, "self")
        )
        self.assertFalse(property_has_user({}, "self"))

    def test_default_message_window_starts_at_local_midnight(self) -> None:
        clock = datetime.fromisoformat("2026-08-02T12:34:56+08:00")
        start, end = default_message_window(clock)
        self.assertEqual(start, "2026-08-02T00:00:00+08:00")
        self.assertEqual(end, "2026-08-02T12:34:56+08:00")

    def test_identity_video_and_comment_normalizers(self) -> None:
        source = {
            "users": [
                {
                    "uniqueId": "Rurutia.wow",
                    "secUid": "sec",
                    "uid": 1,
                    "nickname": "name",
                }
            ]
        }
        self.assertEqual(identities_in(source)[0]["unique_id"], "Rurutia.wow")
        video = normalize_video(
            {
                "aweme_id": "10",
                "author": {"sec_uid": "sec"},
                "video": {"cover": {"url_list": ["https://cover"]}},
            }
        )
        self.assertEqual(video["cover_url"], "https://cover")
        detail = normalize_video_detail(
            {"awemeId": "10", "authorInfo": {"secUid": "sec"}, "video": {"duration": 3}}
        )
        self.assertEqual(detail["aweme_id"], "10")
        self.assertEqual(parse_render_data('%7B"app"%3A%7B%7D%7D'), {"app": {}})
        comments = [
            normalize_comment({"cid": "1"}, "10"),
            normalize_comment(
                {"cid": "2", "reply_id": "1", "reply_to_reply_id": "missing"}, "10"
            ),
        ]
        self.assertFalse(resolve_comment_parents(comments)[1]["parent_resolved"])

    def test_web_protocol_contract_accepts_aliases_and_detects_shape_drift(
        self,
    ) -> None:
        self.assertEqual(
            WEB_PROTOCOL.list_value(
                {"awemeList": [{"awemeId": "1"}]}, "video_items", "video.list"
            )[0]["awemeId"],
            "1",
        )
        with self.assertRaises(DoudouError) as context:
            WEB_PROTOCOL.list_value({"items_v2": []}, "video_items", "video.list")
        self.assertEqual(context.exception.code, "PROTOCOL_SCHEMA_DRIFTED")
        self.assertEqual(context.exception.details["response_keys"], ["items_v2"])

    def test_plan_confirmation_detects_argument_drift(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            create_profile(root, "work")
            payload = {"conversation": {"conversation_id": "1"}, "text": "hello"}
            plan = create_plan(root, "work", "chat.send", payload)
            confirmed = confirm_plan(
                root, "work", plan["plan_id"], "chat.send", payload
            )
            self.assertEqual(confirmed["status"], "confirmed")
            with self.assertRaises(DoudouError) as context:
                confirm_plan(
                    root,
                    "work",
                    plan["plan_id"],
                    "chat.send",
                    {**payload, "text": "changed"},
                )
            self.assertEqual(context.exception.code, "PLAN_DRIFTED")

    def test_cli_has_session_refresh_and_no_warmup_command(self) -> None:
        parsed = parser().parse_args(["session", "refresh", "--profile", "work"])
        self.assertEqual(
            (parsed.command, parsed.session_command), ("session", "refresh")
        )
        with self.assertRaises(SystemExit):
            parser().parse_args(["warmup", "--profile", "work"])


if __name__ == "__main__":
    unittest.main()
