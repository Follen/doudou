from __future__ import annotations

import json
from pathlib import Path

from doudou.models.identity import (
    IdentityCache,
    enrich_known_identities,
    enrich_message_identities,
    known_identity_sec_uids,
    merge_identity,
    normalize_identity,
)
from doudou.storage.identity_migration import migrate_identity_records


def test_identity_normalization_keeps_unique_id_distinct_from_nickname() -> None:
    value = normalize_identity(
        {"uid": "1", "secUid": "SEC", "nickname": "Nickname"},
        source="fixture",
    )
    assert value["unique_id"] is None
    assert value["nickname"] == "Nickname"
    assert value["display_name"] == "Nickname"
    assert value["enrichment"]["status"] == "partial"


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
