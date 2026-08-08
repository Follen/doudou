from __future__ import annotations

import asyncio
import json
from pathlib import Path
from unittest.mock import AsyncMock, MagicMock

import pytest
from doudou.common.errors import DoudouError
from doudou.models.identity import (
    IdentityCache,
    enrich_known_identities,
    enrich_message_identities,
    known_identity_sec_uids,
    merge_identity,
    normalize_identity,
)
from doudou.reads.chat import _resolve_user_identities
from doudou.runtime.paths import assert_supported_identity_records


def test_identity_normalization_keeps_unique_id_distinct_from_nickname() -> None:
    value = normalize_identity(
        {"uid": "1", "secUid": "SEC", "nickname": "Nickname"},
        source="fixture",
    )
    assert value["unique_id"] is None
    assert value["nickname"] == "Nickname"
    assert value["display_name"] == "Nickname"
    assert value["enrichment"]["status"] == "partial"


def test_identity_normalization_keeps_remark_distinct_and_prefers_it_for_display() -> None:
    value = normalize_identity(
        {
            "uid": "1",
            "sec_uid": "SEC",
            "unique_id": "douyin-id",
            "nickname": "Nickname",
            "remark_name": "My remark",
        },
        source="fixture",
    )
    assert value["remark_name"] == "My remark"
    assert value["nickname"] == "Nickname"
    assert value["display_name"] == "My remark"


def test_chat_identity_enrichment_prefers_page_captured_full_user_info(
    tmp_path: Path,
) -> None:
    response = json.dumps(
        {
            "status_code": 0,
            "data": [
                {
                    "uid": "1",
                    "sec_uid": "SEC",
                    "unique_id": "douyin-id",
                    "nickname": "Nickname",
                    "remark_name": "My remark",
                }
            ],
        }
    ).encode()
    session = MagicMock()
    session.captured_user_info_responses = AsyncMock(
        return_value=[b'{"status_code":0,"data":[]}', response]
    )
    session.captured_user_info_endpoint = AsyncMock()
    session.page.evaluate = AsyncMock()
    identities = asyncio.run(
        _resolve_user_identities(session, tmp_path, "work", ["SEC"])
    )
    assert identities["SEC"]["nickname"] == "Nickname"
    assert identities["SEC"]["remark_name"] == "My remark"
    assert identities["SEC"]["display_name"] == "My remark"
    session.page.evaluate.assert_not_awaited()


def test_chat_identity_enrichment_replays_the_page_signed_user_info_template(
    tmp_path: Path,
) -> None:
    full_endpoint = "https://www.douyin.com/aweme/v1/web/im/user/info/?a_bogus=signed"
    session = MagicMock()
    session.captured_user_info_responses = AsyncMock(return_value=[])
    session.captured_user_info_endpoint = AsyncMock(return_value=full_endpoint)
    session.classify_login = AsyncMock(return_value={"valid": True})
    session.page.evaluate = AsyncMock(
        return_value={
            "http_status": 200,
            "content_type": "application/json",
            "retry_after": None,
            "text": json.dumps(
                {
                    "status_code": 0,
                    "data": [
                        {
                            "uid": "1",
                            "sec_uid": "SEC",
                            "nickname": "Nickname",
                            "remark_name": "My remark",
                        }
                    ],
                }
            ),
        }
    )
    identities = asyncio.run(
        _resolve_user_identities(session, tmp_path, "work", ["SEC"])
    )
    assert identities["SEC"]["remark_name"] == "My remark"
    assert session.page.evaluate.await_args.args[1]["endpoint"] == full_endpoint


def test_chat_identity_enrichment_batches_seventy_sec_uids(tmp_path: Path) -> None:
    sec_uids = [f"SEC-{index:02d}" for index in range(70)]
    session = MagicMock()
    session.captured_user_info_responses = AsyncMock(return_value=[])
    session.captured_user_info_endpoint = AsyncMock(return_value="https://www.douyin.com/aweme/v1/web/im/user/info/")
    session.classify_login = AsyncMock(return_value={"valid": True})
    calls: list[list[str]] = []

    async def evaluate(_script, params):
        values = list(params["values"])
        calls.append(values)
        return {
            "http_status": 200,
            "content_type": "application/json",
            "retry_after": None,
            "text": json.dumps(
                {
                    "status_code": 0,
                    "data": [
                        {"sec_uid": value, "nickname": f"Nick {value}"}
                        for value in values
                    ],
                }
            ),
        }

    session.page.evaluate = AsyncMock(side_effect=evaluate)
    identities = asyncio.run(
        _resolve_user_identities(session, tmp_path, "work", sec_uids)
    )

    assert [len(batch) for batch in calls] == [20, 20, 20, 10]
    assert set(identities) == set(sec_uids)
    assert identities["SEC-69"]["display_name"] == "Nick SEC-69"


def test_identity_merge_never_allows_empty_values_to_downgrade() -> None:
    complete = normalize_identity(
        {
            "uid": "1",
            "sec_uid": "SEC",
            "unique_id": "douyin-id",
            "nickname": "Nickname",
        },
        source="complete",
    )
    merged = merge_identity(complete, {"uid": "1", "nickname": ""})
    assert merged["unique_id"] == "douyin-id"
    assert merged["nickname"] == "Nickname"


def test_profile_cache_enriches_nested_message_and_join_event(tmp_path: Path) -> None:
    cache = IdentityCache(tmp_path, "work")
    cache.merge(
        {
            "uid": "1",
            "sec_uid": "SEC",
            "unique_id": "douyin-id",
            "nickname": "Nickname",
        },
        source="user-info",
    )
    cache.save()
    value = {
        "sender": {"uid": "1", "sec_uid": "SEC"},
        "system_event": {
            "source_users": [{"uid": "1", "sec_uid": "SEC"}],
            "joined_users": [{"uid": "2", "sec_uid": "OTHER"}],
        },
    }
    enriched = enrich_known_identities(value, cache, source="collect")
    assert enriched["sender"]["unique_id"] == "douyin-id"
    assert enriched["system_event"]["source_users"][0]["nickname"] == "Nickname"
    assert known_identity_sec_uids(value) == ["SEC", "OTHER"]


def test_profile_cache_resolves_one_exact_unique_id_without_search(
    tmp_path: Path,
) -> None:
    cache = IdentityCache(tmp_path, "work")
    cache.merge(
        {
            "uid": "1",
            "sec_uid": "SEC",
            "unique_id": "Rurutia.wow",
            "nickname": "Nickname",
        },
        source="fixture",
    )
    assert cache.exact_unique_id("rurutia.WOW") == cache.records[0]
    cache.records.append({**cache.records[0], "sec_uid": "OTHER"})
    assert cache.exact_unique_id("Rurutia.wow") is None


def test_legacy_identity_record_is_rejected_without_mutation(tmp_path: Path) -> None:
    filename = (
        tmp_path
        / "message"
        / "profiles"
        / "work"
        / "conversations"
        / "fixture"
        / "messages.jsonl"
    )
    filename.parent.mkdir(parents=True)
    filename.write_text(
        json.dumps({"message_id": "1", "sender": {"uid": "1", "sec_uid": "SEC"}})
        + "\n",
        encoding="utf-8",
    )
    original = filename.read_bytes()

    with pytest.raises(DoudouError) as context:
        assert_supported_identity_records(tmp_path, "work")
    assert context.value.code == "LEGACY_UNSUPPORTED"
    assert context.value.details["legacy_format"] == "identity_record"
    assert context.value.details["path"] == "message/profiles/<PROFILE>/<RECORD>"
    assert filename.read_bytes() == original


def test_current_v1_identity_record_is_supported(tmp_path: Path) -> None:
    filename = (
        tmp_path
        / "message"
        / "profiles"
        / "work"
        / "conversations"
        / "fixture"
        / "messages.jsonl"
    )
    filename.parent.mkdir(parents=True)
    filename.write_text(
        json.dumps(
            {
                "message_id": "1",
                "sender": normalize_identity(
                    {"uid": "1", "sec_uid": "SEC"}, source="fixture"
                ),
            }
        )
        + "\n",
        encoding="utf-8",
    )

    assert_supported_identity_records(tmp_path, "work")


def test_message_identity_enrichment_links_mentions_and_reference_sender(
    tmp_path: Path,
) -> None:
    cache = IdentityCache(tmp_path, "work")
    cache.merge(
        {
            "uid": "2",
            "sec_uid": "SEC2",
            "unique_id": "douyin-2",
            "nickname": "Mentioned",
        },
        source="fixture",
    )
    messages = [
        {"message_id": "10", "sender": {"uid": "1", "sec_uid": "SEC1"}},
        {
            "message_id": "11",
            "sender": {"uid": "3", "sec_uid": "SEC3"},
            "reply_to_message_id": "10",
            "content": {
                "richTextInfos": [
                    {"infoType": 6, "info": {"uid": "2", "mention_id": "2"}}
                ]
            },
        },
    ]
    enriched = enrich_message_identities(messages, cache, source="fixture")
    assert enriched[1]["mentioned_users"][0]["unique_id"] == "douyin-2"
    assert enriched[1]["reference_user"]["uid"] == "1"
    assert enriched[1]["reference_user"]["unique_id"] is None
