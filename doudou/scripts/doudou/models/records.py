from __future__ import annotations

from dataclasses import asdict, dataclass, field
from typing import Any


@dataclass(frozen=True)
class Conversation:
    conversation_id: str
    conversation_short_id: str
    conversation_type: int
    kind: str
    display_name: str
    members: tuple[dict[str, Any], ...] = ()
    ext: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        value = asdict(self)
        value["members"] = list(self.members)
        return value


@dataclass(frozen=True)
class Identity:
    unique_id: str
    sec_uid: str
    uid: str
    nickname: str

    def to_dict(self) -> dict[str, str]:
        return asdict(self)


def conversation_kind(conversation_type: int) -> str:
    if conversation_type == 1:
        return "direct"
    if conversation_type == 2:
        return "group"
    return "unknown"
