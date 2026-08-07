from doudou.protocol.im import (
    parse_stranger_conversations,
    parse_stranger_messages,
    stranger_conversations_request,
    stranger_messages_request,
    stranger_request,
)
from doudou.protocol.protobuf import (
    Field,
    decode_fields,
    encode_fields,
    get_field,
    varint_field,
)


def test_stranger_command_body_mapping_and_requests():
    init = encode_fields([varint_field(1, 0), varint_field(2, 1)])
    envelope = stranger_request(
        init, 1001, stranger_conversations_request(7, 50), sequence=9
    )
    outer = decode_fields(envelope)
    wrapper = decode_fields(bytes(get_field(outer, 8, 2).value))
    assert bytes(get_field(wrapper, 1000, 2).value) == stranger_conversations_request(
        7, 50
    )
    msg = decode_fields(stranger_messages_request("123"))
    assert get_field(msg, 1, 0).value == 123
    assert get_field(msg, 2, 0).value == 0


def test_parse_stranger_conversations_includes_nickname_and_pagination():
    participant = encode_fields(
        [
            varint_field(1, 88),
            varint_field(2, 1),
            Field(4, 2, b"Display Name"),
            Field(5, 2, b"sec-88"),
        ]
    )
    last_message = encode_fields(
        [
            Field(1, 2, b"conv-1"),
            varint_field(3, 99),
            varint_field(5, 123),
            varint_field(6, 7),
            varint_field(7, 88),
            Field(8, 2, b'{"text":"hello"}'),
            varint_field(10, 1000),
        ]
    )
    conversation = encode_fields(
        [
            varint_field(1, 123),
            varint_field(2, 4),
            Field(3, 2, last_message),
            Field(4, 2, b"conv-1"),
            Field(5, 2, participant),
        ]
    )
    body = encode_fields(
        [
            varint_field(1, 456),
            varint_field(2, 1),
            varint_field(3, 4),
            Field(4, 2, conversation),
        ]
    )
    parsed = parse_stranger_conversations(body)
    assert parsed["next_cursor"] == "456"
    assert parsed["has_more"] is True
    assert parsed["total_unread"] == 4
    assert parsed["conversations"][0]["display_name"] == "Display Name"
    assert parsed["conversations"][0]["inbox"] == "request"
    assert parsed["conversations"][0]["conversation_id"] == "conv-1"
    assert parsed["conversations"][0]["conversation_short_id"] == "123"
    assert parsed["conversations"][0]["unread_count"] == 4
    assert parsed["conversations"][0]["last_message"]["text"] == "hello"


def test_parse_stranger_messages_uses_standard_normalization():
    message = encode_fields(
        [
            Field(1, 2, b"conv-1"),
            varint_field(3, 99),
            varint_field(5, 123),
            varint_field(6, 7),
            varint_field(7, 88),
            Field(8, 2, b'{"text":"hello"}'),
            varint_field(10, 1000),
        ]
    )
    body = encode_fields([Field(3, 2, message)])
    parsed = parse_stranger_messages(body, "conv-1")
    assert len(parsed) == 1
    assert parsed[0]["text"] == "hello"
    assert parsed[0]["sender"]["uid"] == "88"
