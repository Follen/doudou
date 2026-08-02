from __future__ import annotations

import argparse
import asyncio
import json
import os
import shutil
import sys
import uuid
import zipfile
from pathlib import Path

from .home import browser_root


def configure_cache(cache: Path | None = None) -> Path:
    from camoufox import multiversion, pkgman

    root = (cache or browser_root()).resolve()
    pkgman.INSTALL_DIR = root
    multiversion.INSTALL_DIR = root
    multiversion.BROWSERS_DIR = root / "browsers"
    multiversion.CONFIG_FILE = root / "config.json"
    multiversion.COMPAT_FLAG = root / ".0.5_FLAG"
    return root


def browser_status(cache: Path, version: str) -> dict[str, object]:
    root = cache.resolve()
    try:
        config = json.loads((root / "config.json").read_text(encoding="utf-8"))
        active = root / str(config["active_version"])
        metadata = json.loads((active / "version.json").read_text(encoding="utf-8"))
        executable = active / ("camoufox.exe" if os.name == "nt" else "camoufox-bin")
        actual = f"{metadata.get('version')}-{metadata.get('build')}"
        available = executable.is_file()
        return {
            "available": available,
            "matches": available and actual == version,
            "expected": version,
            "actual": actual,
            "path": str(active),
            "sha256": metadata.get("sha256"),
        }
    except (FileNotFoundError, KeyError, json.JSONDecodeError):
        return {
            "available": False,
            "matches": False,
            "expected": version,
            "actual": None,
            "path": str(root),
            "sha256": None,
        }


def install(args: argparse.Namespace) -> None:
    cache = args.cache.resolve()
    suffix = args.sha256[:8]
    final = cache / "browsers" / "official" / f"{args.version}-{suffix}"
    version_file = final / "version.json"
    if version_file.exists():
        value = json.loads(version_file.read_text(encoding="utf-8"))
        executable = final / ("camoufox.exe" if os.name == "nt" else "camoufox-bin")
        if value.get("sha256") == args.sha256 and executable.exists():
            print(json.dumps({"status": "cache-hit", "path": str(final)}))
            return
    staging = final.parent / f".{final.name}.install-{uuid.uuid4().hex}"
    shutil.rmtree(staging, ignore_errors=True)
    staging.mkdir(parents=True)
    try:
        with zipfile.ZipFile(args.archive) as archive:
            archive.extractall(staging)
        executable = staging / ("camoufox.exe" if os.name == "nt" else "camoufox-bin")
        if not executable.exists():
            raise RuntimeError("browser archive executable is missing")
        if os.name != "nt":
            executable.chmod(executable.stat().st_mode | 0o755)
        (staging / "version.json").write_text(
            json.dumps(
                {
                    "version": "152.0.4",
                    "build": "beta.28",
                    "prerelease": False,
                    "asset_size": args.size,
                    "sha256": args.sha256,
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        if final.exists():
            shutil.rmtree(final)
        staging.replace(final)
        staging = None
        cache.mkdir(parents=True, exist_ok=True)
        (cache / "config.json").write_text(
            json.dumps(
                {
                    "channel": "official/stable",
                    "active_version": f"browsers/official/{final.name}",
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        (cache / ".0.5_FLAG").touch()
        print(json.dumps({"status": "installed", "path": str(final)}))
    finally:
        if staging:
            shutil.rmtree(staging, ignore_errors=True)


async def launch_probe(args: argparse.Namespace) -> None:
    from camoufox.addons import DefaultAddons
    from camoufox.async_api import AsyncCamoufox

    configure_cache(args.cache)
    async with AsyncCamoufox(
        headless="virtual" if sys.platform.startswith("linux") else False,
        browser=args.version,
        exclude_addons=[DefaultAddons.UBO],
    ) as browser:
        page = await browser.new_page()
        await page.goto("about:blank")
        await page.title()
    print(json.dumps({"status": "verified", "version": args.version}))


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser()
    commands = root.add_subparsers(dest="command", required=True)
    add = commands.add_parser("install")
    add.add_argument("--archive", type=Path, required=True)
    add.add_argument("--cache", type=Path, required=True)
    add.add_argument("--version", required=True)
    add.add_argument("--sha256", required=True)
    add.add_argument("--size", type=int, required=True)
    probe = commands.add_parser("probe")
    probe.add_argument("--cache", type=Path, required=True)
    probe.add_argument("--version", required=True)
    return root


if __name__ == "__main__":
    arguments = parser().parse_args()
    if arguments.command == "install":
        install(arguments)
    else:
        asyncio.run(launch_probe(arguments))
