# Douyin IM Protobuf field map

Evidence sources:

- Loaded web bundle `93791.36fe9dc9.js`, generated `im_proto` encode/decode methods.
- Loaded IM runtime bundle `80030.fbe4ae35.js`, `Message.fromServerMessage` and reference construction.
- Captured successful `SEND_MESSAGE` request/response and subsequent server message readback.

## Server MessageBody

| Field | Wire | SDK name | Parsed output |
| --- | --- | --- | --- |
| 1 | 2 | `conversation_id` | string |
| 2 | 0 | `conversation_type` | int32 |
| 3 | 0 | `server_message_id` | decimal string when unsafe as JS number |
| 4 | 0 | `index_in_conversation` | int64 |
| 5 | 0 | `conversation_short_id` | int64 |
| 6 | 0 | `message_type` | int32 |
| 7 | 0 | `sender` | int64 |
| 8 | 2 | `content` | JSON when valid, otherwise string |
| 9 | 2 | `ext` | `map<string,string>` |
| 10 | 0 | `create_time` | Unix milliseconds |
| 11 | 0 | `version` | int64 |
| 12 | 0 | `status` | int32 |
| 13 | 0 | `order_in_conversation` | int64 |
| 14 | 2 | `sec_sender` | string |
| 15 | 2 | `property_list` | `map<string,PropertyItemList>` |
| 16 | - | not defined | retained in `unknown_fields` if observed |
| 17 | 0 | `index_in_conversation_v2` | int64 |
| 18 | 2 | `reference_info` | `ReferenceInfo` |

Fields outside this table remain in `protocol.unknown_fields`. Field 21 has only
been observed as a monotonic varint on `message_type=50001` command messages.
The loaded SDK has no field 21 descriptor, so no semantic name is assigned.

## PropertyItemList

Field 15 is a map entry: field 1 is the property key and field 2 is a
`PropertyItemList`. Each list item is field 1 with this shape:

| Field | Name | Type |
| --- | --- | --- |
| 1 | `uid` | int64 |
| 2 | `sec_uid` | string |
| 3 | `create_time` | int64 |
| 4 | `idempotent_id` | string |
| 5 | `value` | string |

## Server ReferenceInfo

| Field | SDK name | Type |
| --- | --- | --- |
| 1 | `referenced_message_id` | int64 |
| 2 | `hint` | string, usually JSON |
| 3 | `ref_message_type` | int64 |
| 4 | `referenced_message_status` | int32 |
| 5 | `root_message_id` | int64 |
| 6 | `root_message_conv_index` | int64 |
| 7 | `ext` | `map<string,string>` |

The server currently also emits fields 8 and 9 for replies. Their values match
the referenced message's conversation index and v2 index in captured samples,
but the loaded SDK does not name them. They remain in
`reference_info.unknown_fields`.

## SendMessageRequestBody

| Field | SDK name | Type |
| --- | --- | --- |
| 1 | `conversation_id` | string |
| 2 | `conversation_type` | int32 |
| 3 | `conversation_short_id` | int64 |
| 4 | `content` | string |
| 5 | `ext` | `map<string,string>` |
| 6 | `message_type` | int32 |
| 7 | `ticket` | string |
| 8 | `client_message_id` | string |
| 9 | `mentioned_users` | repeated int64 |
| 10 | `ignore_badge_count` | bool |
| 11 | `ref_msg_info` | `ReferencedMessageInfo` |

`ReferencedMessageInfo` fields are: 1 `referenced_message_id`, 2 `hint`,
3 `root_message_id`, 4 `root_message_conv_index`, and 5 `ext`.

## SendMessageResponseBody

| Field | SDK name | Type |
| --- | --- | --- |
| 1 | `server_message_id` | int64 |
| 2 | `extra_info` | string |
| 3 | `status` | int32 |
| 4 | `client_message_id` | string |
| 5 | `check_code` | int64 |
| 6 | `check_message` | string |
