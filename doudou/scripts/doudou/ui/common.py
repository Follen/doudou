from __future__ import annotations

import asyncio
from typing import Any
from urllib.parse import parse_qsl, urlparse

from ..common.core import sha256


def endpoint_summary(url: str) -> dict[str, Any]:
    parsed = urlparse(url)
    return {
        "origin": f"{parsed.scheme}://{parsed.netloc}",
        "path": parsed.path,
        "query_keys": sorted(
            {key for key, _ in parse_qsl(parsed.query, keep_blank_values=True)}
        ),
    }


async def first_visible(locator: Any) -> Any | None:
    for index in range(await locator.count()):
        candidate = locator.nth(index)
        if await candidate.is_visible():
            return candidate
    return None


class NetworkCapture:
    def __init__(
        self, page: Any, paths: tuple[str, ...], *, exact_paths: bool = False
    ) -> None:
        self.page = page
        self.paths = paths
        self.exact_paths = exact_paths
        self.requests: list[dict[str, Any]] = []
        self.request_urls: list[tuple[str, str]] = []
        self.request_bodies: list[tuple[str, bytes]] = []
        self.responses: list[dict[str, Any]] = []
        self.response_bodies: list[tuple[str, bytes]] = []
        self.tasks: set[asyncio.Task[Any]] = set()

    def _matches(self, url: str) -> bool:
        if self.exact_paths:
            actual = urlparse(url).path.rstrip("/")
            return any(actual == path.rstrip("/") for path in self.paths)
        return any(path in url for path in self.paths)

    def start(self) -> None:
        self.page.on("request", self._on_request)
        self.page.on("response", self._on_response)

    def _on_request(self, request: Any) -> None:
        if not self._matches(request.url):
            return
        body = bytes(request.post_data_buffer or b"")
        path = urlparse(request.url).path
        self.request_urls.append((path, request.url))
        self.request_bodies.append((path, body))
        self.requests.append(
            {
                "method": request.method,
                "endpoint": endpoint_summary(request.url),
                "bytes": len(body),
                "sha256": sha256(body),
            }
        )

    async def _consume_response(self, response: Any) -> None:
        if not self._matches(response.url):
            return
        try:
            body = bytes(await response.body())
        except Exception:  # noqa: BLE001 - detached Playwright response callback
            body = b""
        path = urlparse(response.url).path
        self.responses.append(
            {
                "endpoint": endpoint_summary(response.url),
                "http_status": response.status,
                "bytes": len(body),
                "sha256": sha256(body),
            }
        )
        self.response_bodies.append((path, body))

    def _on_response(self, response: Any) -> None:
        task = asyncio.create_task(self._consume_response(response))
        self.tasks.add(task)
        task.add_done_callback(self.tasks.discard)

    async def stop(self) -> None:
        if self.tasks:
            await asyncio.gather(*list(self.tasks), return_exceptions=True)
        self.page.remove_listener("request", self._on_request)
        self.page.remove_listener("response", self._on_response)
