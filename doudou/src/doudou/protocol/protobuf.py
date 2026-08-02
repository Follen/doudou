from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class Field:
    number: int
    wire: int
    value: int | bytes


def read_varint(data: bytes, offset: int = 0) -> tuple[int, int]:
    value = 0
    shift = 0
    while offset < len(data) and shift <= 70:
        byte = data[offset]
        offset += 1
        value |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return value, offset
        shift += 7
    raise ValueError("invalid protobuf varint")


def encode_varint(value: int) -> bytes:
    if value < 0:
        value &= (1 << 64) - 1
    output = bytearray()
    while value >= 0x80:
        output.append((value & 0x7F) | 0x80)
        value >>= 7
    output.append(value)
    return bytes(output)


def decode_fields(data: bytes) -> list[Field]:
    output: list[Field] = []
    offset = 0
    while offset < len(data):
        key, offset = read_varint(data, offset)
        number, wire = key >> 3, key & 7
        if not number:
            raise ValueError("invalid protobuf field number")
        if wire == 0:
            value, offset = read_varint(data, offset)
        elif wire in (1, 5):
            size = 8 if wire == 1 else 4
            value = data[offset : offset + size]
            if len(value) != size:
                raise ValueError("truncated fixed protobuf field")
            offset += size
        elif wire == 2:
            size, offset = read_varint(data, offset)
            value = data[offset : offset + size]
            if len(value) != size:
                raise ValueError("truncated protobuf field")
            offset += size
        else:
            raise ValueError(f"unsupported protobuf wire type {wire}")
        output.append(Field(number, wire, value))
    return output


def encode_fields(fields: list[Field]) -> bytes:
    output = bytearray()
    for item in fields:
        output.extend(encode_varint((item.number << 3) | item.wire))
        if item.wire == 0:
            output.extend(encode_varint(int(item.value)))
        elif item.wire == 2:
            value = bytes(item.value)
            output.extend(encode_varint(len(value)))
            output.extend(value)
        elif item.wire in (1, 5):
            output.extend(bytes(item.value))
        else:
            raise ValueError(f"unsupported protobuf wire type {item.wire}")
    return bytes(output)


def get_field(
    fields: list[Field], number: int, wire: int | None = None
) -> Field | None:
    return next(
        (
            item
            for item in fields
            if item.number == number and (wire is None or item.wire == wire)
        ),
        None,
    )


def get_fields(
    fields: list[Field], number: int, wire: int | None = None
) -> list[Field]:
    return [
        item
        for item in fields
        if item.number == number and (wire is None or item.wire == wire)
    ]


def text_value(item: Field | None) -> str | None:
    if not item or item.wire != 2:
        return None
    try:
        value = bytes(item.value).decode("utf-8")
    except UnicodeDecodeError:
        return None
    return value if value and "\x00" not in value and "\ufffd" not in value else None


def replace_field(fields: list[Field], number: int, replacement: Field) -> list[Field]:
    return [item for item in fields if item.number != number] + [replacement]


def string_field(number: int, value: str) -> Field:
    return Field(number, 2, value.encode("utf-8"))


def varint_field(number: int, value: int) -> Field:
    return Field(number, 0, value)
