from __future__ import annotations

from copy import deepcopy
from pathlib import Path
from typing import Any

from ..common.core import atomic_write_json, read_json, utc_now
from ..runtime.paths import profile_paths

IDENTITY_FIELDS = (
    "uid",
    "sec_uid",
    "unique_id",
    "nickname",
    "remark_name",
    "display_name",
)


def _text(value: Any) -> str | None:
    if value in (None, ""):
        return None
    return str(value)


def normalize_identity(
    value: dict[str, Any] | None,
    *,
    source: str,
) -> dict[str, Any]:
    raw = value or {}
    uid = _text(raw.get("uid") or raw.get("user_id") or raw.get("userId"))
    sec_uid = _text(raw.get("sec_uid") or raw.get("secUid") or raw.get("sec_user_id"))
    unique_id = _text(raw.get("unique_id") or raw.get("uniqueId"))
    nickname = _text(raw.get("nickname") or raw.get("nick_name") or raw.get("name"))
    remark_name = _text(raw.get("remark_name") or raw.get("remarkName") or raw.get("alias"))
    display_name = _text(raw.get("display_name")) or remark_name or nickname or unique_id
    fields = {
        "uid": uid,
        "sec_uid": sec_uid,
        "unique_id": unique_id,
        "nickname": nickname,
        "remark_name": remark_name,
        "display_name": display_name,
    }
    available = [key for key, item in fields.items() if item]
    existing_enrichment = raw.get("enrichment") or {}
    sources = list(existing_enrichment.get("sources") or [source])
    return {
        "schema": "doudou.user-identity.v1",
        **fields,
        "enrichment": {
            "status": "complete"
            if all(fields[key] for key in ("uid", "sec_uid", "unique_id", "nickname"))
            else "partial",
            "sources": sources,
            "available_fields": available,
            "missing_fields": [key for key in IDENTITY_FIELDS if not fields[key]],
            "updated_at": existing_enrichment.get("updated_at") or utc_now(),
        },
    }


def merge_identity(
    current: dict[str, Any] | None, candidate: dict[str, Any] | None
) -> dict[str, Any]:
    left = normalize_identity(current, source="existing")
    right = normalize_identity(candidate, source="candidate")
    merged = {
        key: right.get(key) or left.get(key) for key in IDENTITY_FIELDS
    }
    sources = list(
        dict.fromkeys(
            [
                *(current or {}).get("enrichment", {}).get("sources", []),
                *(candidate or {}).get("enrichment", {}).get("sources", []),
            ]
        )
    )
    value = normalize_identity(merged, source=sources[0] if sources else "merged")
    value["enrichment"]["sources"] = sources or ["merged"]
    current_sources = (current or {}).get("enrichment", {}).get("sources", [])
    if all((current or {}).get(key) == value.get(key) for key in IDENTITY_FIELDS) and list(
        current_sources
    ) == value["enrichment"]["sources"]:
        value["enrichment"] = deepcopy((current or {})["enrichment"])
    return value


def identity_matches(left: dict[str, Any], right: dict[str, Any]) -> bool:
    return any(
        left.get(key) and left.get(key) == right.get(key)
        for key in ("uid", "sec_uid", "unique_id")
    )


class IdentityCache:
    def __init__(self, data_root: Path, profile: str):
        self.filename = profile_paths(data_root, profile).identities / "index.json"
        value = read_json(self.filename, optional=True) or {}
        self.records: list[dict[str, Any]] = list(value.get("identities") or [])

    def merge(self, value: dict[str, Any], *, source: str) -> dict[str, Any]:
        candidate = normalize_identity(value, source=source)
        for index, record in enumerate(self.records):
            if identity_matches(record, candidate):
                self.records[index] = merge_identity(record, candidate)
                return deepcopy(self.records[index])
        self.records.append(candidate)
        return deepcopy(candidate)

    def enrich(self, value: dict[str, Any], *, source: str) -> dict[str, Any]:
        candidate = normalize_identity(value, source=source)
        for record in self.records:
            if identity_matches(record, candidate):
                return merge_identity(candidate, record)
        return candidate

    def save(self) -> None:
        atomic_write_json(
            self.filename,
            {
                "schema": "doudou.identity-cache.v1",
                "updated_at": utc_now(),
                "count": len(self.records),
                "identities": sorted(
                    self.records,
                    key=lambda item: (
                        item.get("sec_uid") or "",
                        item.get("uid") or "",
                        item.get("unique_id") or "",
                    ),
                ),
            },
        )


IDENTITY_KEYS = {
    "sender",
    "author",
    "user",
    "mention",
    "mentions",
    "reference_user",
    "source_users",
    "joined_users",
    "participants",
    "members",
    "mentioned_users",
}

IDENTITY_CONTAINER_KEYS = {
    "candidates",
    "comments",
    "conversations",
    "messages",
    "records",
    "replies",
    "reply",
    "videos",
}


def enrich_known_identities(value: Any, cache: IdentityCache, *, source: str) -> Any:
    if isinstance(value, list):
        return [enrich_known_identities(item, cache, source=source) for item in value]
    if not isinstance(value, dict):
        return value
    output = dict(value)
    if any(output.get(key) for key in ("uid", "sec_uid", "unique_id", "secUid")):
        output = {**output, **cache.enrich(output, source=source)}
    for key in IDENTITY_KEYS:
        if key in output:
            output[key] = enrich_known_identities(output[key], cache, source=source)
    for key in IDENTITY_CONTAINER_KEYS:
        if key in output:
            output[key] = enrich_known_identities(output[key], cache, source=source)
    if isinstance(output.get("system_event"), dict):
        output["system_event"] = enrich_known_identities(
            output["system_event"], cache, source=source
        )
    if isinstance(output.get("property_list"), dict):
        output["property_list"] = enrich_known_identities(
            output["property_list"], cache, source=source
        )
    return output


def known_identity_sec_uids(value: Any) -> list[str]:
    output: list[str] = []

    def visit(current: Any, identity_context: bool = False) -> None:
        if isinstance(current, list):
            for item in current:
                visit(item, identity_context)
            return
        if not isinstance(current, dict):
            return
        if identity_context and current.get("sec_uid"):
            output.append(str(current["sec_uid"]))
        for key in IDENTITY_KEYS:
            if key in current:
                visit(current[key], True)
        for key in IDENTITY_CONTAINER_KEYS:
            if key in current:
                visit(current[key])
        if isinstance(current.get("system_event"), dict):
            visit(current["system_event"])
        if isinstance(current.get("property_list"), dict):
            visit(current["property_list"], True)

    visit(value)
    return list(dict.fromkeys(output))


def enrich_message_identities(
    messages: list[dict[str, Any]],
    cache: IdentityCache,
    *,
    source: str,
    reference_catalog: list[dict[str, Any]] | None = None,
) -> list[dict[str, Any]]:
    catalog = {
        str(item.get("message_id")): item
        for item in [*(reference_catalog or []), *messages]
        if item.get("message_id")
    }
    output: list[dict[str, Any]] = []
    for message in messages:
        value = dict(message)
        content = value.get("content")
        raw_mentions = content.get("richTextInfos") if isinstance(content, dict) else []
        mentions = list(value.get("mentioned_users") or [])
        for item in raw_mentions or []:
            if not isinstance(item, dict) or int(item.get("infoType") or 0) not in {1, 6}:
                continue
            info = item.get("info") or {}
            uid = _text(info.get("uid")) if isinstance(info, dict) else None
            if uid:
                mentions.append({"uid": uid})
        merged_mentions: list[dict[str, Any]] = []
        for mention in mentions:
            candidate = cache.enrich(mention, source=f"{source}-mention")
            for index, existing in enumerate(merged_mentions):
                if identity_matches(existing, candidate):
                    merged_mentions[index] = merge_identity(existing, candidate)
                    break
            else:
                merged_mentions.append(candidate)
        if merged_mentions:
            value["mentioned_users"] = merged_mentions

        reference_id = value.get("reply_to_message_id") or (
            (value.get("reference_info") or {}).get("referenced_message_id")
            if isinstance(value.get("reference_info"), dict)
            else None
        )
        if reference_id:
            target = catalog.get(str(reference_id)) or {}
            reference_user = target.get("sender") or value.get("reference_user") or {}
            value["reference_user"] = cache.enrich(
                reference_user, source=f"{source}-reference"
            )
        output.append(enrich_known_identities(value, cache, source=source))
    return output
