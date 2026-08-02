from __future__ import annotations

import hashlib
import json
import os
import re
import secrets
import tempfile
import unicodedata
from collections.abc import Iterable
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from .errors import DoudouError


def utc_now() -> str:
    return (
        datetime.now(timezone.utc)
        .isoformat(timespec="milliseconds")
        .replace("+00:00", "Z")
    )


def random_id(prefix: str) -> str:
    stamp = datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S")
    return f"{prefix}-{stamp}-{secrets.token_hex(4)}"


def sha256(value: bytes | str) -> str:
    if isinstance(value, str):
        value = value.encode("utf-8")
    return hashlib.sha256(value).hexdigest()


def safe_segment(value: Any, max_length: int = 96) -> str:
    text = unicodedata.normalize("NFKC", str(value or "")).strip()
    if not text:
        raise DoudouError("INVALID_PATH_SEGMENT", "A non-empty path value is required.")
    encoded: list[str] = []
    for character in text:
        if re.fullmatch(r"[A-Za-z0-9._-]", character) or character.isalnum():
            encoded.append(character)
        else:
            encoded.append(f"~{ord(character):04X}")
    result = "".join(encoded)
    if re.fullmatch(r"\.+", result):
        result = "~" + result.encode("utf-8").hex()
    if len(result) > max_length:
        result = f"{result[: max_length - 13]}~{sha256(text)[:12]}"
    return result


def normalize_lookup(value: Any) -> str:
    text = unicodedata.normalize("NFKC", str(value or ""))
    return " ".join(text.split()).casefold()


def resolve_data_root(candidate: str | Path | None = None) -> Path:
    selected = candidate or Path.cwd()
    return Path(selected).expanduser().resolve()


def atomic_write_bytes(path: Path, value: bytes, mode: int | None = None) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", suffix=".tmp", dir=path.parent
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(value)
            handle.flush()
            os.fsync(handle.fileno())
        if mode is not None:
            os.chmod(temporary, mode)
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def atomic_write_text(path: Path, value: str, mode: int | None = None) -> None:
    atomic_write_bytes(path, value.encode("utf-8"), mode=mode)


def atomic_write_json(path: Path, value: Any, mode: int | None = None) -> None:
    atomic_write_text(
        path, json.dumps(value, ensure_ascii=False, indent=2) + "\n", mode=mode
    )


def atomic_write_jsonl(path: Path, values: Iterable[Any]) -> None:
    lines = [
        json.dumps(value, ensure_ascii=False, separators=(",", ":")) for value in values
    ]
    atomic_write_text(path, ("\n".join(lines) + "\n") if lines else "")


def read_json(path: Path, optional: bool = False) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        if optional:
            return None
        raise
    except json.JSONDecodeError as error:
        raise DoudouError(
            "INVALID_JSON",
            f"Invalid JSON in {path}.",
            {"path": str(path), "message": str(error)},
        ) from error


def read_jsonl(path: Path, optional: bool = False) -> list[Any]:
    try:
        content = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        if optional:
            return []
        raise
    if content and not content.endswith("\n"):
        raise DoudouError(
            "INCOMPLETE_JSONL_LINE",
            f"JSONL file has an incomplete final line: {path}.",
            {"path": str(path)},
        )
    records: list[Any] = []
    for index, line in enumerate(content.splitlines(), 1):
        if not line:
            continue
        try:
            records.append(json.loads(line))
        except json.JSONDecodeError as error:
            raise DoudouError(
                "INVALID_JSONL",
                f"Invalid JSONL at {path}:{index}.",
                {"path": str(path), "line": index},
            ) from error
    return records


def emit(value: dict[str, Any]) -> None:
    print(json.dumps(value, ensure_ascii=False, separators=(",", ":")), flush=True)
