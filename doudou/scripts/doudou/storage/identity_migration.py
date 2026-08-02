from __future__ import annotations

from pathlib import Path
from typing import Any

from ..common.core import atomic_write_json, atomic_write_jsonl, read_json, read_jsonl
from ..models.identity import (
    IdentityCache,
    enrich_known_identities,
    enrich_message_identities,
)
from ..runtime.paths import profile_paths


def migrate_identity_records(data_root: Path, profile: str) -> dict[str, Any]:
    paths = profile_paths(data_root, profile)
    cache = IdentityCache(data_root, profile)
    changed = scanned = 0
    roots = [paths.message]
    for root in roots:
        if not root.exists():
            continue
        for filename in root.rglob("*"):
            if filename == cache.filename or not filename.is_file():
                continue
            if filename.suffix == ".json":
                value = read_json(filename, optional=True)
                if not isinstance(value, (dict, list)):
                    continue
                scanned += 1
                migrated = enrich_known_identities(value, cache, source="migration-cache")
                if migrated != value:
                    atomic_write_json(filename, migrated)
                    changed += 1
            elif filename.suffix == ".jsonl":
                value = read_jsonl(filename, optional=True)
                scanned += 1
                if value and all(isinstance(item, dict) for item in value) and any(
                    item.get("message_id") for item in value
                ):
                    migrated = enrich_message_identities(
                        value, cache, source="migration-cache"
                    )
                else:
                    migrated = enrich_known_identities(
                        value, cache, source="migration-cache"
                    )
                if migrated != value:
                    atomic_write_jsonl(filename, migrated)
                    changed += 1
    cache.save()
    return {
        "status": "migrated",
        "profile": paths.profile_id,
        "scanned_files": scanned,
        "changed_files": changed,
        "cache": str(cache.filename),
    }
