from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

from cryptography.exceptions import InvalidTag
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from doudou.features.chat.service import verify_like_readback, verify_send_readback
from doudou.protocol.im import (
    extract_messages,
    find_conversations,
    normalize_system_event,
    parse_participants_page,
    parse_send_request,
    parse_send_response,
)
from doudou.protocol.protobuf import (
    Field,
    decode_fields,
    encode_fields,
    string_field,
    varint_field,
)
from doudou.protocol.web import normalize_comment
from doudou.reads.video import _reply_url
from doudou.runtime.profile import create_profile
from doudou.storage.content import read_comment_checkpoint, write_comment_checkpoint
from doudou.storage.media import decrypt_image, exact_md5, match_image_md5
from doudou.storage.repository import merge_messages, read_messages
from doudou.ui.adapters import CHAT_ADAPTER


def embedded(number: int, fields: list[Field]) -> Field:
    return Field(number, 2, encode_fields(fields))


class ProtocolStorageTests(unittest.TestCase):
    def test_chat_write_readback_uses_server_ids_and_current_user_reaction(self) -> None:
        payload = {
            "mode": "reply",
            "text": "收到",
            "conversation": {"conversation_id": "group-1"},
            "reply": {"message_id": "old-1"},
        }
        result = {"verification": {"server_message_id": "new-1"}}
        messages = [
            {
                "message_id": "new-1",
                "conversation_id": "group-1",
                "message_type": 7,
                "text": "收到",
                "reply_to_message_id": "old-1",
            },
            {
                "message_id": "old-1",
                "property_list": {"e:love": {"items": [{"uid": "self"}]}},
            },
        ]
        self.assertTrue(verify_send_readback(payload, result, messages)["passed"])
        self.assertTrue(
            verify_like_readback(
                {"message": {"message_id": "old-1"}, "self_uid": "self"},
                messages,
            )["passed"]
        )

    def test_reply_url_discards_stale_signatures_and_root_only_fields(self) -> None:
        value = _reply_url(
            "https://www.douyin.com/aweme/v1/web/comment/list/?"
            "aweme_id=video&cursor=5&msToken=old&a_bogus=old&"
            "insert_ids=old&rcFT=old&aid=6383",
            "video-1",
            "root-1",
            20,
        )
        parsed = urlsplit(value)
        query = parse_qs(parsed.query)
        self.assertEqual(parsed.path, "/aweme/v1/web/comment/list/reply/")
        self.assertEqual(query["item_id"], ["video-1"])
        self.assertEqual(query["comment_id"], ["root-1"])
        self.assertEqual(query["cursor"], ["20"])
        self.assertEqual(query["item_type"], ["0"])
        self.assertEqual(query["aid"], ["6383"])
        self.assertFalse(
            {"aweme_id", "msToken", "a_bogus", "insert_ids", "rcFT"} & query.keys()
        )

    def test_root_comment_zero_reply_id_normalizes_to_its_own_cid(self) -> None:
        root = normalize_comment(
            {
                "cid": "root-1",
                "reply_id": "0",
                "reply_to_reply_id": "0",
                "reply_comment_total": 1,
            },
            "video-1",
        )
        reply = normalize_comment(
            {"cid": "reply-1", "reply_id": "root-1"}, "video-1"
        )
        self.assertEqual(root["reply_id"], "root-1")
        self.assertIsNone(root["reply_to_reply_id"])
        self.assertTrue(root["parent_resolved"])
        self.assertEqual(reply["reply_id"], "root-1")

    def test_chat_send_adapter_targets_the_visible_publish_svg(self) -> None:
        self.assertEqual(CHAT_ADAPTER.version, "2026-08-02.2")
        self.assertEqual(CHAT_ADAPTER.selectors["send_button"], "svg.e2e-send-msg-btn")

    def test_send_request_and_response_are_parsed_for_receipt_verification(
        self,
    ) -> None:
        content = json.dumps(
            {
                "text": "@露露缇娅\u2005很好",
                "aweType": 700,
                "richTextInfos": [
                    {"infoType": 1, "location": 0, "length": 6, "info": {"uid": "106"}}
                ],
            },
            ensure_ascii=False,
        )
        ext = encode_fields(
            [string_field(1, "s:mentioned_users"), string_field(2, "106")]
        )
        reference = encode_fields([varint_field(1, 9001)])
        send_body = encode_fields(
            [
                string_field(1, "conversation"),
                varint_field(2, 2),
                varint_field(3, 123),
                string_field(4, content),
                Field(5, 2, ext),
                varint_field(6, 7),
                string_field(8, "client-id"),
                varint_field(9, 106),
                Field(11, 2, reference),
            ]
        )
        request = encode_fields(
            [Field(8, 2, encode_fields([Field(100, 2, send_body)]))]
        )
        parsed_request = parse_send_request(request)
        self.assertEqual(parsed_request["mentioned_users"], ["106"])
        self.assertEqual(parsed_request["referenced_message_id"], "9001")
        self.assertEqual(parsed_request["ext"]["s:mentioned_users"], "106")

        response_body = encode_fields(
            [
                varint_field(1, 8001),
                varint_field(3, 0),
                string_field(4, "client-id"),
            ]
        )
        response = encode_fields(
            [
                varint_field(3, 0),
                string_field(4, "OK"),
                Field(6, 2, encode_fields([Field(100, 2, response_body)])),
            ]
        )
        parsed_response = parse_send_response(response)
        self.assertEqual(parsed_response["server_message_id"], "8001")
        self.assertEqual(parsed_response["business_status"], 0)
        self.assertEqual(parsed_response["client_message_id"], "client-id")

    def test_comment_checkpoint_round_trip_deduplicates_partial_records(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            create_profile(root, "work")
            write_comment_checkpoint(
                root,
                "work",
                "video-1",
                {
                    "status": "running",
                    "root_cursor": "20",
                    "completed_root_pages": 1,
                    "completed_reply_root_ids": ["c1"],
                },
                [
                    {"cid": "c1", "text": "old"},
                    {"cid": "c1", "text": "new"},
                    {"cid": "c2", "text": "reply"},
                ],
            )
            checkpoint, comments = read_comment_checkpoint(root, "work", "video-1")
            self.assertEqual(checkpoint["root_cursor"], "20")
            self.assertEqual([item["cid"] for item in comments], ["c1", "c2"])
            self.assertEqual(comments[0]["text"], "new")

    def test_invalid_optional_participants_blob_does_not_abort_conversation_scan(
        self,
    ) -> None:
        self.assertEqual(
            parse_participants_page(b"\x80"),
            {"participants": [], "has_more": False, "cursor": "0"},
        )

    def test_conversation_name_comes_from_field_fifty_and_ticket_is_not_exposed(
        self,
    ) -> None:
        core = encode_fields(
            [
                string_field(1, "group:2"),
                varint_field(2, 22),
                varint_field(3, 2),
                varint_field(4, 1),
                string_field(5, "露露群"),
            ]
        )
        conversation = encode_fields(
            [
                string_field(1, "group:2"),
                varint_field(2, 22),
                varint_field(3, 2),
                string_field(4, "secret-ticket"),
                Field(50, 2, core),
            ]
        )
        [parsed] = find_conversations(encode_fields([Field(1, 2, conversation)]))
        self.assertEqual(parsed["display_name"], "露露群")
        self.assertNotIn("ticket", parsed)

    def test_unknown_system_event_keeps_a_structured_inventory(self) -> None:
        event = normalize_system_event({"aweType": 999, "payload": {"x": 1}}, 1001)
        self.assertEqual(event["kind"], "unknown")
        self.assertEqual(event["protocol"], {"message_type": 1001, "awe_type": 999})
        self.assertEqual(event["content_shape"], ["aweType", "payload"])

    def test_message_merge_fills_nested_sender_name(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            create_profile(root, "work")
            conversation = {
                "conversation_id": "group:2",
                "kind": "group",
                "display_name": "group",
            }
            first = {
                "message_id": "1",
                "create_time_ms": 1,
                "sender": {"uid": "7", "sec_uid": "sec", "name": None},
            }
            enriched = {
                "message_id": "1",
                "create_time_ms": 1,
                "sender": {"uid": "7", "sec_uid": "sec", "name": "露露"},
            }
            merge_messages(root, "work", conversation, [first])
            merge_messages(root, "work", conversation, [enriched])
            [stored] = read_messages(root, "work", conversation)
            self.assertEqual(stored["sender"]["name"], "露露")

    def test_protobuf_round_trip_and_conversation_types(self) -> None:
        direct = encode_fields(
            [
                string_field(1, "direct:1"),
                varint_field(2, 11),
                varint_field(3, 1),
                string_field(4, "ticket-a"),
            ]
        )
        group = encode_fields(
            [
                string_field(1, "group:2"),
                varint_field(2, 22),
                varint_field(3, 2),
                string_field(4, "ticket-b"),
            ]
        )
        envelope = encode_fields([Field(9, 2, direct), Field(9, 2, group)])
        self.assertEqual(encode_fields(decode_fields(envelope)), envelope)
        conversations = {
            item["conversation_id"]: item for item in find_conversations(envelope)
        }
        self.assertEqual(conversations["direct:1"]["kind"], "direct")
        self.assertEqual(conversations["group:2"]["kind"], "group")

    def test_reference_unknown_fields_and_member_join_event(self) -> None:
        reference = encode_fields(
            [
                varint_field(1, 9001),
                string_field(2, json.dumps({"text": "quoted"})),
                varint_field(3, 7),
                varint_field(99, 3),
            ]
        )
        content = {
            "aweType": 100140,
            "active_users": [{"uid": "1", "nickname": "source"}],
            "passive_users": [{"uid": "2", "nickname": "joined"}],
            "locale_resources": [{"key": "member_joined"}],
            "template": [{"action": "profile_homepage"}],
        }
        message = encode_fields(
            [
                string_field(1, "group:2"),
                varint_field(2, 2),
                varint_field(3, 10001),
                varint_field(5, 22),
                varint_field(6, 1001),
                varint_field(7, 1),
                string_field(8, json.dumps(content)),
                varint_field(10, 123456789),
                Field(18, 2, reference),
                varint_field(77, 5),
            ]
        )
        parsed = extract_messages(encode_fields([Field(1, 2, message)]), "group:2")[0]
        self.assertEqual(parsed["reference_info"]["referenced_message_id"], "9001")
        self.assertEqual(parsed["reference_info"]["unknown_fields"][0]["field"], 99)
        self.assertEqual(parsed["system_event"]["kind"], "member_joined")
        self.assertEqual(parsed["system_event"]["joined_users"][0]["uid"], "2")
        self.assertEqual(parsed["unknown_fields"][0]["field"], 77)

    def test_aes_gcm_image_decryption(self) -> None:
        key = bytes(range(32))
        nonce = bytes(range(12))
        clear = b"\x89PNG\r\n\x1a\nfixture"
        encrypted = nonce + AESGCM(key).encrypt(nonce, clear, None)
        self.assertEqual(decrypt_image(encrypted, key.hex()), clear)
        with self.assertRaises(InvalidTag):
            decrypt_image(encrypted[:-1] + bytes([encrypted[-1] ^ 1]), key.hex())

    def test_message_image_md5_requires_an_exact_digest(self) -> None:
        self.assertEqual(
            exact_md5("21baf2fc08a986aea3571262ca8a397a"),
            "21baf2fc08a986aea3571262ca8a397a",
        )
        self.assertEqual(
            exact_md5("21BAF2FC08A986AEA3571262CA8A397A"),
            "21baf2fc08a986aea3571262ca8a397a",
        )
        self.assertIsNone(
            exact_md5(
                "5e06b60d0ce3875b56b0bc57e1d53482243370295296IMG_20260801.jpeg"
            )
        )
        self.assertIsNone(exact_md5("21baf2fc08a986aea3571262ca8a397"))

    def test_message_image_md5_can_describe_encrypted_or_clear_bytes(self) -> None:
        self.assertEqual(
            match_image_md5(
                "a" * 32,
                encrypted_md5="a" * 32,
                clear_md5="b" * 32,
            ),
            "encrypted",
        )
        self.assertEqual(
            match_image_md5(
                "b" * 32,
                encrypted_md5="a" * 32,
                clear_md5="b" * 32,
            ),
            "clear",
        )
        with self.assertRaisesRegex(ValueError, "MEDIA_MD5_MISMATCH"):
            match_image_md5(
                "c" * 32,
                encrypted_md5="a" * 32,
                clear_md5="b" * 32,
            )

    def test_message_merge_fills_missing_values_and_records_conflicts(self) -> None:
        conversation = {
            "conversation_id": "group:2",
            "kind": "group",
            "display_name": "group",
        }
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first = {
                "schema": "doudou.message.v2",
                "message_id": "1",
                "create_time_ms": 1,
                "text": None,
                "type": "text",
                "images": [{"status": "failed"}],
            }
            second = {
                "schema": "doudou.message.v2",
                "message_id": "1",
                "create_time_ms": 1,
                "text": "hello",
                "type": "image",
                "images": [{"status": "downloaded"}],
            }
            merge_messages(root, "work", conversation, [first])
            result = merge_messages(root, "work", conversation, [second])
            self.assertEqual(result["total"], 1)
            self.assertEqual(result["duplicates"], 1)
            self.assertEqual(result["conflicts"], 1)
            stored = read_messages(root, "work", conversation)
            self.assertEqual(stored[0]["images"], [{"status": "downloaded"}])


if __name__ == "__main__":
    unittest.main()
