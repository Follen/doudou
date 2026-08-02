from __future__ import annotations

import asyncio
import json
from pathlib import Path
from unittest.mock import AsyncMock, MagicMock

from doudou.models.identity import (
    IdentityCache,
    enrich_known_identities,
    enrich_message_identities,
    known_identity_sec_uids,
    merge_identity,
    normalize_identity,
)
from doudou.reads.chat import _resolve_user_identities
from doudou.storage.identity_migration import _fixed_point, migrate_identity_records


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


def test_identity_migration_is_idempotent(tmp_path: Path) -> None:
    cache = IdentityCache(tmp_path, "work")
    cache.merge(
        {
            "uid": "1",
            "sec_uid": "SEC",
            "unique_id": "douyin-id",
            "nickname": "Nickname",
        },
        source="fixture",
    )
    cache.save()
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
    first = migrate_identity_records(tmp_path, "work")
    content = filename.read_text(encoding="utf-8")
    second = migrate_identity_records(tmp_path, "work")
    assert first["changed_files"] == 1
    assert second["changed_files"] == 0
    assert filename.read_text(encoding="utf-8") == content
    assert json.loads(content)["sender"]["unique_id"] == "douyin-id"


def test_identity_migration_reaches_nested_fixed_point_in_one_run() -> None:
    def enrich(value: dict[str, int]) -> dict[str, int]:
        stage = int(value.get("stage") or 0)
        return {"stage": min(stage + 1, 2)}

    assert _fixed_point({"stage": 0}, enrich) == {"stage": 2}


def test_identity_migration_crosses_conversation_index_container(tmp_path: Path) -> None:
    filename = tmp_path / "message" / "profiles" / "work" / "conversations.json"
    filename.parent.mkdir(parents=True)
    filename.write_text(
        json.dumps(
            {
                "conversations": [
                    {
                        "kind": "direct",
                        "participants": {
                            "participants": [{"uid": "1", "sec_uid": "SEC"}]
                        },
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    first = migrate_identity_records(tmp_path, "work")
    content = filename.read_text(encoding="utf-8")
    second = migrate_identity_records(tmp_path, "work")
    participant = json.loads(content)["conversations"][0]["participants"][
        "participants"
    ][0]
    assert first["changed_files"] == 1
    assert second["changed_files"] == 0
    assert participant["schema"] == "doudou.user-identity.v1"
    assert participant["unique_id"] is None


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
