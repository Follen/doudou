from __future__ import annotations

import json
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

from ..common.core import atomic_write_json, random_id, read_json, sha256, utc_now
from ..common.errors import DoudouError
from ..runtime.paths import profile_paths


def payload_hash(operation: str, payload: dict[str, Any]) -> str:
    canonical = json.dumps(
        {"operation": operation, "payload": payload},
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    )
    return sha256(canonical)


def create_plan(
    data_root: Path,
    profile: str,
    operation: str,
    payload: dict[str, Any],
    ttl_seconds: int = 900,
) -> dict[str, Any]:
    created = datetime.now(timezone.utc)
    plan = {
        "schema": "doudou.write-plan.v1",
        "plan_id": random_id("plan"),
        "profile_id": profile_paths(data_root, profile).profile_id,
        "operation": operation,
        "payload": payload,
        "payload_sha256": payload_hash(operation, payload),
        "created_at": created.isoformat(timespec="milliseconds").replace("+00:00", "Z"),
        "expires_at": (created + timedelta(seconds=ttl_seconds))
        .isoformat(timespec="milliseconds")
        .replace("+00:00", "Z"),
        "status": "confirmation_required",
    }
    atomic_write_json(
        profile_paths(data_root, profile).plans / f"{plan['plan_id']}.json", plan
    )
    return plan


def confirm_plan(
    data_root: Path, profile: str, plan_id: str, operation: str, payload: dict[str, Any]
) -> dict[str, Any]:
    path = profile_paths(data_root, profile).plans / f"{plan_id}.json"
    plan = read_json(path, optional=True)
    if not isinstance(plan, dict):
        raise DoudouError(
            "PLAN_NOT_FOUND", "The write plan does not exist.", {"plan_id": plan_id}
        )
    if plan.get("operation") != operation or plan.get("payload_sha256") != payload_hash(
        operation, payload
    ):
        raise DoudouError(
            "PLAN_DRIFTED",
            "The confirmed write arguments differ from the preview.",
            {"plan_id": plan_id},
        )
    expires = datetime.fromisoformat(str(plan["expires_at"]).replace("Z", "+00:00"))
    if expires <= datetime.now(timezone.utc):
        raise DoudouError(
            "PLAN_EXPIRED",
            "The write plan expired before confirmation.",
            {"plan_id": plan_id},
        )
    if plan.get("status") not in {"confirmation_required", "confirmed"}:
        raise DoudouError(
            "PLAN_ALREADY_USED",
            "The write plan is no longer executable.",
            {"plan_id": plan_id, "status": plan.get("status")},
        )
    plan["status"] = "confirmed"
    plan["confirmed_at"] = utc_now()
    atomic_write_json(path, plan)
    return plan


def load_plan(
    data_root: Path, profile: str, plan_id: str, operation: str | None = None
) -> dict[str, Any]:
    plan = read_json(
        profile_paths(data_root, profile).plans / f"{plan_id}.json", optional=True
    )
    if not isinstance(plan, dict):
        raise DoudouError(
            "PLAN_NOT_FOUND", "The write plan does not exist.", {"plan_id": plan_id}
        )
    if operation and plan.get("operation") != operation:
        raise DoudouError(
            "PLAN_OPERATION_MISMATCH",
            "The plan belongs to a different operation.",
            {
                "plan_id": plan_id,
                "expected": operation,
                "actual": plan.get("operation"),
            },
        )
    return plan


def complete_plan(
    data_root: Path, profile: str, plan_id: str, status: str, receipt: str | None = None
) -> None:
    path = profile_paths(data_root, profile).plans / f"{plan_id}.json"
    plan = read_json(path)
    plan["status"] = status
    plan["completed_at"] = utc_now()
    plan["receipt"] = receipt
    atomic_write_json(path, plan)
