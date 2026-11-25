//! Wire-level Protobuf field encoder and decoder for Douyin IM envelopes.

use doudou_core::{DoudouError, Result};

/// Decoded wire value of a Protobuf field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldValue {
    Varint(u64),
    Bytes(Vec<u8>),
}

/// Single Protobuf field tag + wire type + payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub number: u32,
    pub wire: u8,
    pub value: FieldValue,
}

impl Field {
    /// Return varint value if this field is wire type 0.
    pub fn as_varint(&self) -> Option<u64> {
        match &self.value {
            FieldValue::Varint(v) => Some(*v),
            FieldValue::Bytes(_) => None,
        }
    }

    /// Return byte slice if this field is wire type 1, 2, or 5.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match &self.value {
            FieldValue::Bytes(b) => Some(b.as_slice()),
            FieldValue::Varint(_) => None,
        }
    }
}

/// Decode one unsigned LEB128 varint starting at `offset`, returning `(value, next_offset)`.
pub fn read_varint(data: &[u8], mut offset: usize) -> Result<(u64, usize)> {
    let mut value: u64 = 0;
    let mut shift: u32 = 0;
    while offset < data.len() && shift <= 63 {
        let byte = data[offset];
        offset += 1;
        value |= u64::from(byte & 0x7F) << shift;
        if (byte & 0x80) == 0 {
            return Ok((value, offset));
        }
        shift += 7;
    }
    Err(DoudouError::new(
        "INVALID_PROTOBUF",
        "invalid protobuf varint",
    ))
}

/// Encode an unsigned 64-bit integer as LEB128 varint.
pub fn encode_varint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(10);
    while value >= 0x80 {
        out.push(((value & 0x7F) as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
    out
}

/// Encode a signed integer using two's-complement 64-bit varint encoding (matching protobuf).
pub fn encode_signed_varint(value: i64) -> Vec<u8> {
    encode_varint(value as u64)
}

/// Decode all top-level wire fields from a Protobuf message buffer.
pub fn decode_fields(data: &[u8]) -> Result<Vec<Field>> {
    let mut out = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        let (key, next) = read_varint(data, offset)?;
        offset = next;
        let number = u32::try_from(key >> 3)
            .map_err(|_| DoudouError::new("INVALID_PROTOBUF", "invalid protobuf field number"))?;
        let wire = (key & 7) as u8;
        if number == 0 {
            return Err(DoudouError::new(
                "INVALID_PROTOBUF",
                "invalid protobuf field number",
            ));
        }
        let value = match wire {
            0 => {
                let (v, next) = read_varint(data, offset)?;
                offset = next;
                FieldValue::Varint(v)
            }
            1 | 5 => {
                let size = if wire == 1 { 8 } else { 4 };
                if offset + size > data.len() {
                    return Err(DoudouError::new(
                        "INVALID_PROTOBUF",
                        "truncated fixed protobuf field",
                    ));
                }
                let slice = data[offset..offset + size].to_vec();
                offset += size;
                FieldValue::Bytes(slice)
            }
            2 => {
                let (len, next) = read_varint(data, offset)?;
                offset = next;
                let size = usize::try_from(len).map_err(|_| {
                    DoudouError::new("INVALID_PROTOBUF", "protobuf length overflow")
                })?;
                if offset.checked_add(size).is_none_or(|end| end > data.len()) {
                    return Err(DoudouError::new(
                        "INVALID_PROTOBUF",
                        "truncated protobuf field",
                    ));
                }
                let slice = data[offset..offset + size].to_vec();
                offset += size;
                FieldValue::Bytes(slice)
            }
            _ => {
                return Err(DoudouError::new(
                    "INVALID_PROTOBUF",
                    format!("unsupported protobuf wire type {wire}"),
                ));
            }
        };
        out.push(Field {
            number,
            wire,
            value,
        });
    }
    Ok(out)
}

/// Encode a slice of Protobuf fields into raw wire bytes.
pub fn encode_fields(fields: &[Field]) -> Vec<u8> {
    let mut out = Vec::new();
    for item in fields {
        let tag = (u64::from(item.number) << 3) | u64::from(item.wire);
        out.extend_from_slice(&encode_varint(tag));
        match (&item.wire, &item.value) {
            (0, FieldValue::Varint(v)) => out.extend_from_slice(&encode_varint(*v)),
            (2, FieldValue::Bytes(b)) => {
                out.extend_from_slice(&encode_varint(b.len() as u64));
                out.extend_from_slice(b);
            }
            (1 | 5, FieldValue::Bytes(b)) => out.extend_from_slice(b),
            _ => {}
        }
    }
    out
}

/// Find the first field matching `number` and optional `wire`.
pub fn get_field(fields: &[Field], number: u32, wire: Option<u8>) -> Option<&Field> {
    fields
        .iter()
        .find(|f| f.number == number && wire.is_none_or(|w| f.wire == w))
}

/// Return all fields matching `number` and optional `wire`.
pub fn get_fields(fields: &[Field], number: u32, wire: Option<u8>) -> Vec<&Field> {
    fields
        .iter()
        .filter(|f| f.number == number && wire.is_none_or(|w| f.wire == w))
        .collect()
}

/// Extract clean non-empty UTF-8 text from a wire-type 2 field (rejecting NUL and U+FFFD).
pub fn text_value(item: Option<&Field>) -> Option<String> {
    let field = item?;
    if field.wire != 2 {
        return None;
    }
    let bytes = field.as_bytes()?;
    let text = std::str::from_utf8(bytes).ok()?;
    if text.is_empty() || text.contains('\0') || text.contains('\u{FFFD}') {
        None
    } else {
        Some(text.to_owned())
    }
}

/// Replace all occurrences of `number` in `fields` with `replacement` appended at the end.
pub fn replace_field(fields: Vec<Field>, number: u32, replacement: Field) -> Vec<Field> {
    let mut out: Vec<Field> = fields.into_iter().filter(|f| f.number != number).collect();
    out.push(replacement);
    out
}

/// Construct a UTF-8 length-delimited field.
pub fn string_field(number: u32, value: &str) -> Field {
    Field {
        number,
        wire: 2,
        value: FieldValue::Bytes(value.as_bytes().to_vec()),
    }
}

/// Construct a raw bytes length-delimited field.
pub fn bytes_field(number: u32, value: Vec<u8>) -> Field {
    Field {
        number,
        wire: 2,
        value: FieldValue::Bytes(value),
    }
}

/// Construct a varint field.
pub fn varint_field(number: u32, value: u64) -> Field {
    Field {
        number,
        wire: 0,
        value: FieldValue::Varint(value),
    }
}
