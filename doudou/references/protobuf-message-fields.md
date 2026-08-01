# Douyin IM Protobuf field map

Evidence sources:

- Loaded web bundle `93791.36fe9dc9.js`, generated `im_proto` encode/decode methods.
- Loaded IM runtime bundle `80030.fbe4ae35.js`, `Message.fromServerMessage` and reference construction.
- Captured successful `SEND_MESSAGE` request/response and subsequent server message readback.

The `/chat` init request is the transport template, not a send template. Outer field 1 is command, field 2 is sequence, field 8 is `RequestBody`, and the command number is also the nested request-body field number. The implementation preserves authenticated outer metadata, then replaces fields 1, 2 and 8. Confirmed command mappings are:

| Command | Request body | HTTP path |
| --- | --- | --- |
| 100 | `SendMessageRequestBody` | `/v1/message/send` |
| 301 | `MessagesInConversationRequestBody` | `/v1/message/get_by_conversation` |
| 605 | `ConversationParticipantsListRequestBody` | `/v1/conversation/participants_list` |

`ConversationInfoV2` fields 1-4 are conversation ID, short ID, type and ticket. Field 6 is the first `ParticipantsPage`; field 50 is core info.

Conversation type values used by the loaded IM SDK are `1` for a direct
conversation and `2` for a group conversation. Discovery retains both values;
send requests must match the selected index entry exactly.

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

Mention content uses `aweType=700`. Each mentioned token has matching `richTextInfos` entries of `infoType=1` (`uid`, `con_id`) and `infoType=2` (`font_weight=2`); locations and lengths are JavaScript UTF-16 code units. Field 9 repeats every UID and ext contains `s:mentioned_users`.

Image sends use `message_type=27` and `aweType=2702`. Content includes `resource_url` (`oid`, `skey`, byte size, MD5 and built URL lists), dimensions, top-level plaintext MD5 and `from_gallery=1`.

## History and participants

`MessagesInConversationRequestBody` fields 1-6 are conversation ID, type, short ID, direction, anchor index and limit. Its response fields are repeated messages, next cursor and has-more.

`ConversationParticipantsListRequestBody` fields 1-5 are conversation ID, short ID, type, cursor and limit. `ParticipantsPage` contains repeated participant field 1, has-more field 2 and cursor field 3. Participant fields 1-5 are UID, sort order, role, alias and sec UID; field 8 is ext.

## SendMessageResponseBody

| Field | SDK name | Type |
| --- | --- | --- |
| 1 | `server_message_id` | int64 |
| 2 | `extra_info` | string |
| 3 | `status` | int32 |
| 4 | `client_message_id` | string |
| 5 | `check_code` | int64 |
| 6 | `check_message` | string |
