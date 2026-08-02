from __future__ import annotations

import hashlib
import re
import urllib.request
from pathlib import Path
from typing import Any

from cryptography.hazmat.primitives.ciphers.aead import AESGCM

from ..common.core import atomic_write_bytes, sha256

_MD5_RE = re.compile(r"[0-9a-fA-F]{32}")


def exact_md5(value: object) -> str | None:
    if not isinstance(value, str) or _MD5_RE.fullmatch(value) is None:
        return None
    return value.lower()


def match_image_md5(
    declared: str | None, *, encrypted_md5: str, clear_md5: str
) -> str | None:
    if declared is None:
        return None
    if declared == encrypted_md5:
        return "encrypted"
    if declared == clear_md5:
        return "clear"
    raise ValueError("MEDIA_MD5_MISMATCH")


def first_image_url(resource: dict[str, Any]) -> str | None:
    for key in (
        "origin_url_list",
        "large_url_list",
        "medium_url_list",
        "thumb_url_list",
    ):
        values = resource.get(key)
        if isinstance(values, list) and values and isinstance(values[0], str):
            return values[0]
    return None


def detect_image(value: bytes) -> tuple[str, str]:
    if value.startswith(b"\x89PNG\r\n\x1a\n"):
        return "png", "image/png"
    if value.startswith(b"\xff\xd8\xff"):
        return "jpg", "image/jpeg"
    if value.startswith((b"GIF87a", b"GIF89a")):
        return "gif", "image/gif"
    if len(value) >= 12 and value[8:12] == b"WEBP":
        return "webp", "image/webp"
    if len(value) >= 12 and value[4:12] in (
        b"ftypheic",
        b"ftypheix",
        b"ftyphevc",
        b"ftyphevx",
        b"ftypmif1",
    ):
        return "heic", "image/heic"
    raise ValueError("unrecognized image format")


def decrypt_image(encrypted: bytes, secret_key: str) -> bytes:
    key = bytes.fromhex(secret_key)
    if len(key) != 32 or len(encrypted) < 29:
        raise ValueError("invalid AES-GCM image envelope")
    return AESGCM(key).decrypt(encrypted[:12], encrypted[12:], None)


def download_message_images(
    message: dict[str, Any], directory: Path
) -> list[dict[str, Any]]:
    content = message.get("content")
    resource = content.get("resource_url") if isinstance(content, dict) else None
    if message.get("message_type") != 27 or not isinstance(resource, dict):
        return []
    url, secret = first_image_url(resource), resource.get("skey")
    if not url or not isinstance(secret, str):
        return [
            {
                "status": "failed",
                "retryable": True,
                "error_code": "MEDIA_RESOURCE_MISSING",
            }
        ]
    try:
        request = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
        with urllib.request.urlopen(request, timeout=45) as response:
            encrypted = response.read()
        declared_transport_md5 = exact_md5(resource.get("md5"))
        raw_transport_md5 = resource.get("md5")
        if raw_transport_md5 and declared_transport_md5 is None:
            raise ValueError("MEDIA_MD5_INVALID")
        encrypted_md5 = hashlib.md5(encrypted).hexdigest()
        clear = decrypt_image(encrypted, secret)
        extension, mime = detect_image(clear)
        declared_size = resource.get("data_size")
        actual_md5 = hashlib.md5(clear).hexdigest()
        resource_md5_scope = match_image_md5(
            declared_transport_md5,
            encrypted_md5=encrypted_md5,
            clear_md5=actual_md5,
        )
        declared_content_md5 = exact_md5(content.get("md5"))
        if declared_content_md5 and declared_content_md5 != actual_md5:
            raise ValueError("MEDIA_MD5_MISMATCH")
        if isinstance(declared_size, int) and declared_size != len(clear):
            raise ValueError("MEDIA_SIZE_MISMATCH")
        target = directory / f"{message['message_id']}.{extension}"
        atomic_write_bytes(target, clear)
        return [
            {
                "status": "downloaded",
                "retryable": False,
                "local_path": str(target),
                "format": extension,
                "mime": mime,
                "bytes": len(clear),
                "md5": actual_md5,
                "sha256": sha256(clear),
                "encrypted_bytes": len(encrypted),
                "encrypted_md5": encrypted_md5,
                "resource_md5_scope": resource_md5_scope,
                "resource_url_sha256": sha256(url),
            }
        ]
    except Exception as error:  # noqa: BLE001 - media failures are persisted per attachment
        code = (
            str(error) if str(error).startswith("MEDIA_") else "MEDIA_PROCESSING_FAILED"
        )
        return [
            {
                "status": "failed",
                "retryable": True,
                "error_code": code,
                "resource_url_sha256": sha256(url),
            }
        ]
