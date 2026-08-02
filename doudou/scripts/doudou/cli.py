from __future__ import annotations

import argparse
from pathlib import Path
from typing import Any

from . import __version__
from .common.core import emit, resolve_data_root
from .common.errors import DoudouError
from .models.resolver import parse_selection, resolve_conversation
from .runtime.broker.client import BrokerClient
from .runtime.environment import doctor
from .runtime.paths import initialize_data_root
from .runtime.profile import create_profile, list_profiles, load_profile, update_profile


def _profile_argument(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--profile", required=True)


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(
        prog="doudou", description="Doudou Camoufox/Python CLI"
    )
    root.add_argument("--data-root", type=Path, default=None)
    root.add_argument("--version", action="version", version=__version__)
    commands = root.add_subparsers(dest="command", required=True)

    doctor_parser = commands.add_parser("doctor")
    doctor_probe = doctor_parser.add_mutually_exclusive_group()
    doctor_probe.add_argument("--launch-probe", action="store_true")
    doctor_probe.add_argument("--skip-launch", action="store_true", help=argparse.SUPPRESS)
    profile = commands.add_parser("profile").add_subparsers(
        dest="profile_command", required=True
    )
    profile.add_parser("list")
    create = profile.add_parser("create")
    _profile_argument(create)
    create.add_argument("--display-name")
    for name in ("status", "login"):
        item = profile.add_parser(name)
        _profile_argument(item)
        if name == "login":
            item.add_argument("--timeout", type=float, default=600.0)

    broker = commands.add_parser("broker").add_subparsers(
        dest="broker_command", required=True
    )
    for name in ("status", "ping", "stop", "test-start"):
        item = broker.add_parser(name)
        _profile_argument(item)
        if name == "test-start":
            item.add_argument("--fake-browser", action="store_true")

    session = commands.add_parser("session").add_subparsers(
        dest="session_command", required=True
    )
    for name in ("status", "refresh"):
        item = session.add_parser(name)
        _profile_argument(item)

    conversation = commands.add_parser("conversations").add_subparsers(
        dest="conversation_command", required=True
    )
    refresh = conversation.add_parser("refresh")
    _profile_argument(refresh)
    refresh.add_argument("--kind", choices=("all", "group", "direct"), default="all")
    refresh.add_argument("--force", action="store_true")
    resolve = conversation.add_parser("resolve")
    _profile_argument(resolve)
    resolve.add_argument("--target", required=True)
    select = conversation.add_parser("select")
    _profile_argument(select)
    select.add_argument("--selection", required=True)
    select.add_argument("--refreshed-at", required=True)

    messages = commands.add_parser("messages").add_subparsers(
        dest="messages_command", required=True
    )
    collect = messages.add_parser("collect")
    _profile_argument(collect)
    collect.add_argument("--conversation", required=True)
    collect.add_argument("--from", dest="from_time")
    collect.add_argument("--to", dest="to_time")
    collect.add_argument("--max-pages", type=int, default=100)
    collect.add_argument("--skip-images", action="store_true")

    protocol = commands.add_parser("protocol").add_subparsers(
        dest="protocol_command", required=True
    )
    send = protocol.add_parser("send")
    _profile_argument(send)
    send.add_argument("--recipient")
    send.add_argument("--conversation-id")
    send.add_argument("--text")
    send.add_argument("--image", type=Path)
    send.add_argument(
        "--at", action="append", default=[], help=argparse.SUPPRESS
    )
    send.add_argument("--reply")
    send.add_argument("--confirm", action="store_true")
    send.add_argument("--plan-id")
    send_batch = protocol.add_parser("send-batch")
    _profile_argument(send_batch)
    send_batch.add_argument("--recipients")
    send_batch.add_argument("--conversation-ids")
    send_batch.add_argument("--text")
    send_batch.add_argument("--confirm", action="store_true")
    send_batch.add_argument("--plan-id")
    reply = protocol.add_parser("reply")
    _profile_argument(reply)
    reply.add_argument("--recipient")
    reply.add_argument("--conversation-id")
    reply.add_argument("--message", required=False)
    reply.add_argument("--text")
    reply.add_argument("--confirm", action="store_true")
    reply.add_argument("--plan-id")
    capture = protocol.add_parser("capture")
    _profile_argument(capture)
    capture.add_argument("--timeout", type=float, default=600.0)

    chat = commands.add_parser("chat").add_subparsers(
        dest="chat_command", required=True
    )
    like = chat.add_parser("like")
    _profile_argument(like)
    like.add_argument("--conversation")
    like.add_argument("--message")
    like.add_argument("--confirm", action="store_true")
    like.add_argument("--plan-id")

    identity = commands.add_parser("identity").add_subparsers(
        dest="identity_command", required=True
    )
    identity_resolve = identity.add_parser("resolve")
    _profile_argument(identity_resolve)
    identity_resolve.add_argument("--douyin-id", required=True)
    identity_migrate = identity.add_parser("migrate")
    _profile_argument(identity_migrate)

    videos = commands.add_parser("videos").add_subparsers(
        dest="videos_command", required=True
    )
    videos_collect = videos.add_parser("collect")
    _profile_argument(videos_collect)
    videos_collect.add_argument("--douyin-id", required=True)
    videos_collect.add_argument("--max-pages", type=int, default=1000)

    video = commands.add_parser("video").add_subparsers(
        dest="video_command", required=True
    )
    video_collect = video.add_parser("collect")
    _profile_argument(video_collect)
    video_collect.add_argument("--sec-uid", required=True)
    video_collect.add_argument("--aweme-id", required=True)
    video_collect.add_argument("--max-pages", type=int, default=1000)
    for name in ("comment", "reply", "like"):
        item = video.add_parser(name)
        _profile_argument(item)
        item.add_argument("--sec-uid")
        item.add_argument("--aweme-id")
        if name in {"comment", "reply"}:
            item.add_argument("--text")
        if name in {"reply", "like"}:
            item.add_argument("--comment-id")
            item.add_argument("--comment-text")
        if name == "like":
            item.add_argument("--digg-count", type=int, default=0)
            item.add_argument("--already-liked", action="store_true")
        item.add_argument("--confirm", action="store_true")
        item.add_argument("--plan-id")

    page_doctor = commands.add_parser("page-doctor")
    _profile_argument(page_doctor)
    page_doctor.add_argument("--baseline", type=Path)
    return root


def _client(data_root: Path, profile: str, fake_browser: bool = False) -> BrokerClient:
    return BrokerClient(data_root, profile, fake_browser=fake_browser)


def _target(args: argparse.Namespace) -> str:
    value = (
        getattr(args, "conversation_id", None)
        or getattr(args, "recipient", None)
        or getattr(args, "conversation", None)
    )
    if not value:
        raise DoudouError(
            "MISSING_ARGUMENT",
            "A conversation ID or existing conversation name is required.",
        )
    return str(value)


def _mentions(values: list[str]) -> list[str]:
    return [
        item.strip() for value in values for item in value.split(",") if item.strip()
    ]


def dispatch(args: argparse.Namespace, skill_root: Path) -> dict[str, Any]:
    data_root = resolve_data_root(args.data_root)
    if args.command == "doctor":
        initialize_data_root(data_root)
        return doctor(data_root, launch_probe=args.launch_probe)
    initialize_data_root(data_root)

    if args.command == "profile":
        if args.profile_command == "list":
            profiles = list_profiles(data_root)
            return {"status": "ok", "count": len(profiles), "profiles": profiles}
        if args.profile_command == "create":
            metadata, paths = create_profile(data_root, args.profile, args.display_name)
            return {
                "status": "created",
                "profile": metadata,
                "paths": {
                    "profile": str(paths.profile),
                    "browser_data": str(paths.browser_data),
                    "message": str(paths.message),
                },
            }
        metadata, paths = load_profile(data_root, args.profile)
        client = _client(data_root, args.profile)
        if args.profile_command == "status":
            descriptor = client._descriptor()
            broker_status = (
                client.request("status", start=False)["result"] if descriptor else None
            )
            return {
                "status": "ok",
                "profile": metadata,
                "broker": broker_status,
                "paths": {"profile": str(paths.profile), "message": str(paths.message)},
            }
        client.request("browser.visible", {"visible": True}, semantics="control")
        client.request(
            "page.open", {"url": "https://www.douyin.com/"}, semantics="control"
        )
        result = client.request(
            "login.wait", {"timeout_seconds": args.timeout}, semantics="control"
        )["result"]
        profile_record = update_profile(
            data_root,
            args.profile,
            {
                "login_status": "valid",
                "last_login_verified_at": result["initialized_at"],
                "last_error_code": None,
            },
        )
        return {"status": "logged_in", "profile": profile_record, "session": result}

    if args.command == "broker":
        client = _client(
            data_root, args.profile, fake_browser=getattr(args, "fake_browser", False)
        )
        if args.broker_command == "status":
            return client.request("status", start=False)["result"]
        if args.broker_command == "ping":
            return client.request("ping")["result"]
        if args.broker_command == "stop":
            result = client.request("shutdown", semantics="control", start=False)[
                "result"
            ]
            if not client.wait_stopped(timeout=60):
                raise DoudouError(
                    "BROKER_STOP_TIMEOUT",
                    "The Profile broker did not release its process and lock in time.",
                    {"profile_id": args.profile, "timeout_seconds": 60},
                )
            return {**result, "stopped": True}
        return client.request("status")["result"]

    if (
        args.command == "protocol"
        and args.protocol_command == "send"
        and _mentions(args.at)
    ):
        raise DoudouError(
            "MENTION_UNSUPPORTED",
            "Group mentions are not supported.",
        )

    client = _client(data_root, args.profile)
    if args.command == "session":
        command = (
            "session.refresh" if args.session_command == "refresh" else "session.status"
        )
        return client.request(
            command,
            semantics="control" if args.session_command == "refresh" else "read",
        )["result"]
    if args.command == "conversations":
        result = client.request(
            "conversations.refresh",
            {
                "force": args.conversation_command == "refresh" and args.force,
                "kind": getattr(args, "kind", "all"),
            },
        )["result"]
        if args.conversation_command == "refresh":
            return result
        if args.conversation_command == "resolve":
            return {
                "status": "resolved",
                "conversation": resolve_conversation(
                    result["conversations"], args.target
                ),
                "refreshed_at": result["refreshed_at"],
            }
        if result["refreshed_at"] != args.refreshed_at:
            raise DoudouError(
                "CONVERSATION_SNAPSHOT_DRIFTED",
                "The numbered conversation list has changed.",
            )
        selected = parse_selection(args.selection, result["count"])
        return {
            "status": "selected",
            "refreshed_at": result["refreshed_at"],
            "conversations": [result["conversations"][index - 1] for index in selected],
        }
    if args.command == "messages":
        return client.request(
            "messages.collect",
            {
                "target": args.conversation,
                "from": args.from_time,
                "to": args.to_time,
                "max_pages": args.max_pages,
                "skip_images": args.skip_images,
            },
        )["result"]
    if args.command == "protocol":
        if args.protocol_command == "capture":
            return client.request(
                "protocol.capture",
                {"timeout_seconds": args.timeout},
                semantics="control",
            )["result"]
        if args.protocol_command == "send-batch":
            if args.confirm:
                if not args.plan_id:
                    raise DoudouError(
                        "PLAN_ID_REQUIRED", "--confirm requires --plan-id."
                    )
                return client.request(
                    "chat.send.batch.execute",
                    {"plan_id": args.plan_id},
                    semantics="write",
                )["result"]
            values = args.conversation_ids or args.recipients or ""
            targets = [item.strip() for item in values.split(",") if item.strip()]
            return client.request(
                "chat.send.batch.preview", {"targets": targets, "text": args.text}
            )["result"]
        if args.confirm:
            if not args.plan_id:
                raise DoudouError("PLAN_ID_REQUIRED", "--confirm requires --plan-id.")
            return client.request(
                "chat.send.execute", {"plan_id": args.plan_id}, semantics="write"
            )["result"]
        target = _target(args)
        if args.protocol_command == "reply":
            params = {
                "target": target,
                "text": args.text,
                "reply": args.message,
                "at": [],
            }
        else:
            params = {
                "target": target,
                "text": args.text,
                "image": str(args.image.resolve()) if args.image else None,
                "reply": args.reply,
                "at": _mentions(args.at),
            }
        return client.request("chat.send.preview", params)["result"]
    if args.command == "chat":
        if args.confirm:
            if not args.plan_id:
                raise DoudouError("PLAN_ID_REQUIRED", "--confirm requires --plan-id.")
            return client.request(
                "chat.like.execute", {"plan_id": args.plan_id}, semantics="write"
            )["result"]
        return client.request(
            "chat.like.preview", {"target": args.conversation, "message": args.message}
        )["result"]
    if args.command == "identity" and args.identity_command == "migrate":
        from .storage.identity_migration import migrate_identity_records

        return migrate_identity_records(data_root, args.profile)
    if args.command == "identity":
        return client.request("identity.resolve", {"douyin_id": args.douyin_id})[
            "result"
        ]
    if args.command == "videos":
        return client.request(
            "videos.collect", {"douyin_id": args.douyin_id, "max_pages": args.max_pages}
        )["result"]
    if args.command == "video":
        if args.video_command == "collect":
            return client.request(
                "video.collect",
                {
                    "sec_uid": args.sec_uid,
                    "aweme_id": args.aweme_id,
                    "max_pages": args.max_pages,
                },
            )["result"]
        operation = {
            "comment": "video.comment.create",
            "reply": "video.comment.reply",
            "like": "video.comment.like",
        }[args.video_command]
        if args.confirm:
            if not args.plan_id:
                raise DoudouError("PLAN_ID_REQUIRED", "--confirm requires --plan-id.")
            return client.request(
                "video.interaction.execute",
                {"operation": operation, "plan_id": args.plan_id},
                semantics="write",
            )["result"]
        payload = {"sec_uid": args.sec_uid, "aweme_id": args.aweme_id}
        if args.video_command in {"comment", "reply"}:
            payload["text"] = args.text
        if args.video_command in {"reply", "like"}:
            payload["comment"] = {
                "cid": args.comment_id,
                "text": args.comment_text,
                "digg_count": getattr(args, "digg_count", 0),
                "user_digged": getattr(args, "already_liked", False),
            }
        return client.request(
            "video.interaction.preview", {"operation": operation, "payload": payload}
        )["result"]
    if args.command == "page-doctor":
        return client.request("page.doctor", {})["result"]
    raise DoudouError("UNKNOWN_COMMAND", "Unknown command.")


def main(argv: list[str] | None = None, skill_root: Path | None = None) -> int:
    arguments = parser().parse_args(argv)
    root = skill_root or Path(__file__).resolve().parents[2]
    emit(
        {
            "event": "started",
            "command": arguments.command,
            "data_root": str(resolve_data_root(arguments.data_root)),
            "version": __version__,
        }
    )
    try:
        result = dispatch(arguments, root)
        emit({"event": "result", "result": result})
        return 0 if result.get("status") != "failed" else 1
    except DoudouError as error:
        emit(error.as_event())
        return error.exit_code
    except RuntimeError as error:
        if str(error) == "PROFILE_ROOT_CONFLICT":
            emit(
                DoudouError(
                    "PROFILE_ROOT_CONFLICT",
                    "Both profile/ and Profile/ exist in the data root.",
                ).as_event()
            )
            return 1
        raise
