from __future__ import annotations

from typing import Any


class DoudouError(Exception):
    def __init__(
        self,
        code: str,
        message: str,
        details: dict[str, Any] | None = None,
        exit_code: int = 1,
    ) -> None:
        super().__init__(message)
        self.code = code
        self.message = message
        self.details = details
        self.exit_code = exit_code

    def as_event(self) -> dict[str, Any]:
        event: dict[str, Any] = {
            "event": "error",
            "code": self.code,
            "message": self.message,
        }
        if self.details is not None:
            event["details"] = self.details
        return event
