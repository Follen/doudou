from __future__ import annotations

import json
import unittest

from doudou.common.errors import DoudouError
from doudou.features.video.service import verify_interaction_readback
from doudou.protocol.write_responses import (
    parse_page_write_response,
    request_fields,
    response_comment,
    validate_comment_request,
)


class VideoWriteProtocolTests(unittest.TestCase):
    def test_protocol_readback_verifies_reply_and_like_by_stable_id(self) -> None:
        payload = {
            "text": "回复内容",
            "comment": {"cid": "root-1"},
        }
        reply = verify_interaction_readback(
            "video.comment.reply",
            payload,
            {"created_comment": {"cid": "reply-1"}},
            [
                {
                    "cid": "reply-1",
                    "reply_id": "root-1",
                    "text": "回复内容",
                }
            ],
        )
        liked = verify_interaction_readback(
            "video.comment.like",
            {"comment": {"cid": "root-1"}},
            {},
            [{"cid": "root-1", "user_digged": True}],
        )
        self.assertTrue(reply["passed"])
        self.assertTrue(liked["passed"])

    def test_form_request_and_success_response_match_plan(self) -> None:
        fields = request_fields(
            "https://www.douyin.com/aweme/v1/web/comment/publish/?aweme_id=10",
            b"text=%E5%BE%88%E6%A3%92",
        )
        self.assertEqual(
            validate_comment_request(fields, aweme_id="10", text="很棒")["text"], "很棒"
        )
        value = parse_page_write_response(
            json.dumps(
                {"status_code": 0, "comment": {"cid": "20", "text": "很棒"}}
            ).encode(),
            http_status=200,
            operation="video.comment.create",
        )
        self.assertEqual(response_comment(value)["cid"], "20")

    def test_request_drift_is_unknown_after_click(self) -> None:
        with self.assertRaises(DoudouError) as context:
            validate_comment_request(
                {"aweme_id": "10", "text": "。"},
                aweme_id="10",
                text="讲得很清楚，辛苦啦。",
            )
        self.assertEqual(context.exception.code, "PAGE_WRITE_REQUEST_DRIFTED")
        self.assertTrue(context.exception.details["write_started"])

    def test_business_failure_and_kick_are_known(self) -> None:
        with self.assertRaises(DoudouError) as failure:
            parse_page_write_response(
                b'{"status_code":7}', http_status=200, operation="video.comment.like"
            )
        self.assertEqual(failure.exception.code, "VIDEO_WRITE_FAILED")
        self.assertTrue(failure.exception.details["result_known"])
        with self.assertRaises(DoudouError) as kick:
            parse_page_write_response(
                b'{"decision":"KICK"}',
                http_status=200,
                operation="video.comment.create",
            )
        self.assertEqual(kick.exception.code, "WRITE_VERIFICATION_REQUIRED")


if __name__ == "__main__":
    unittest.main()
