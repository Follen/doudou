//! Douyin IM Protobuf, Web API contracts, response classifiers, and AES-GCM media verification.

pub mod contracts;
pub mod im;
pub mod media;
pub mod protobuf;
pub mod responses;
pub mod web;
pub mod write_responses;

pub use contracts::{WEB_PROTOCOL_VERSION, contract_list, contract_value, field_aliases, web_path};
pub use im::{
    ImResponse, build_command_request, extract_messages, find_conversations, messages_request,
    normalize_message, normalize_system_event, parse_map_entries, parse_participants_page,
    parse_property_list, parse_reference_info, parse_send_request, parse_send_response,
    parse_stranger_conversations, parse_stranger_messages, participants_request, property_has_user,
    response_body, stranger_body_field, stranger_conversations_request, stranger_messages_request,
    stranger_request, stranger_response_body, structured_field,
};
pub use media::{
    decrypt_image, detect_image, download_message_images, exact_md5, first_image_url,
    match_image_md5, process_downloaded_image,
};
pub use protobuf::{
    Field, FieldValue, bytes_field, decode_fields, encode_fields, encode_signed_varint,
    encode_varint, get_field, get_fields, read_varint, replace_field, string_field, text_value,
    varint_field,
};
pub use responses::{FetchResponse, ResponseCodes, parse_json_response, response_metadata};
pub use web::{
    first_url, identities_in, identity_from, normalize_comment, normalize_video,
    normalize_video_detail, parse_render_data, resolve_comment_parents,
};
pub use write_responses::{
    parse_page_write_response, request_fields, response_comment, validate_comment_request,
};

#[cfg(test)]
mod tests {
    use aes_gcm::aead::{Aead as _, KeyInit as _};
    use aes_gcm::{Aes256Gcm, Nonce};
    use md5::{Digest as _, Md5};
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn protobuf_round_trip_and_stranger_envelope_mapping() {
        let init = encode_fields(&[
            varint_field(1, 200),
            varint_field(2, 1),
            string_field(3, "token"),
        ]);
        let req = stranger_request(
            &init,
            1001,
            &stranger_conversations_request(0, 50, true),
            Some(42),
        )
        .unwrap();
        let outer = decode_fields(&req).unwrap();
        assert_eq!(
            get_field(&outer, 1, Some(0)).unwrap().as_varint(),
            Some(1001)
        );
        assert_eq!(get_field(&outer, 2, Some(0)).unwrap().as_varint(), Some(42));
        let wrapper =
            decode_fields(get_field(&outer, 8, Some(2)).unwrap().as_bytes().unwrap()).unwrap();
        assert!(get_field(&wrapper, 1000, Some(2)).is_some());
    }

    #[test]
    fn im_send_request_and_response_round_trip() {
        let body = encode_fields(&[
            string_field(1, "0:1:10:20"),
            varint_field(2, 1),
            varint_field(3, 999),
            string_field(4, r#"{"text":"晚上好"}"#),
            varint_field(6, 7),
            string_field(8, "client-uuid-1"),
        ]);
        let envelope = encode_fields(&[
            varint_field(1, 100),
            bytes_field(8, encode_fields(&[bytes_field(100, body)])),
        ]);
        let parsed = parse_send_request(&envelope).unwrap();
        assert_eq!(parsed["conversation_id"], "0:1:10:20");
        assert_eq!(parsed["content"]["text"], "晚上好");
        assert_eq!(parsed["client_message_id"], "client-uuid-1");

        let resp_body = encode_fields(&[
            varint_field(1, 7_711_059_380_192_314_917),
            varint_field(3, 0),
            string_field(4, "client-uuid-1"),
        ]);
        let resp_env = encode_fields(&[
            varint_field(3, 0),
            string_field(4, "OK"),
            bytes_field(6, encode_fields(&[bytes_field(100, resp_body)])),
        ]);
        let parsed_resp = parse_send_response(&resp_env).unwrap();
        assert_eq!(parsed_resp["protobuf_status"], 0);
        assert_eq!(parsed_resp["business_status"], 0);
        assert_eq!(parsed_resp["server_message_id"], "7711059380192314917");
    }

    #[test]
    fn response_classifier_separates_rate_limit_kick_and_schema_drift() {
        let codes = ResponseCodes {
            empty: "EMPTY",
            invalid: "INVALID",
            failed: "FAILED",
        };
        let r429 = FetchResponse {
            http_status: 429,
            content_type: "application/json".to_owned(),
            retry_after: Some("5".to_owned()),
            text: "{}".to_owned(),
        };
        assert_eq!(
            parse_json_response(&r429, codes, true, false, "test")
                .unwrap_err()
                .code,
            "RATE_LIMITED"
        );

        let kick = FetchResponse {
            http_status: 200,
            content_type: "application/json".to_owned(),
            retry_after: None,
            text: r#"{"data":{"decision":"KICK"}}"#.to_owned(),
        };
        assert_eq!(
            parse_json_response(&kick, codes, true, false, "test")
                .unwrap_err()
                .code,
            "VERIFICATION_REQUIRED"
        );

        let payload = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(
            r#"{"status_code":0,"aweme_list":"wrong"}"#,
        )
        .unwrap();
        assert_eq!(
            contract_list(&payload, "video_items", "videos.collect")
                .unwrap_err()
                .code,
            "PROTOCOL_SCHEMA_DRIFTED"
        );
    }

    #[test]
    fn aes_gcm_image_decryption_and_md5_verification_pass_and_fail_closed() {
        let key = [0x42u8; 32];
        let nonce_bytes = [0x11u8; 12];
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(b"fake-png-body");
        let cipher = Aes256Gcm::new_from_slice(&key).unwrap();
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), png.as_ref())
            .unwrap();
        let mut envelope = nonce_bytes.to_vec();
        envelope.extend_from_slice(&ciphertext);

        let clear_md5 = hex::encode(Md5::digest(&png));
        let enc_md5 = hex::encode(Md5::digest(&envelope));
        let dir = TempDir::new().unwrap();
        let msg = json!({
            "message_id": "m100",
            "message_type": 27,
            "content": {
                "md5": clear_md5,
                "resource_url": {
                    "skey": hex::encode(key),
                    "md5": enc_md5,
                    "data_size": png.len(),
                    "origin_url_list": ["https://example.invalid/img.bin"]
                }
            }
        });
        let saved = process_downloaded_image(
            &msg,
            &envelope,
            "https://example.invalid/img.bin",
            dir.path(),
        )
        .unwrap();
        assert_eq!(saved["status"], "downloaded");
        assert_eq!(saved["format"], "png");
        assert_eq!(saved["resource_md5_scope"], "encrypted");

        let bad_msg = json!({
            "message_id": "m101",
            "message_type": 27,
            "content": {
                "md5": "00000000000000000000000000000000",
                "resource_url": {
                    "skey": hex::encode(key),
                    "origin_url_list": ["https://example.invalid/img.bin"]
                }
            }
        });
        assert_eq!(
            process_downloaded_image(
                &bad_msg,
                &envelope,
                "https://example.invalid/img.bin",
                dir.path()
            )
            .unwrap_err()
            .code,
            "MEDIA_MD5_MISMATCH"
        );
    }

    #[test]
    fn identities_in_extracts_search_sug_rich_identity() {
        let payload = json!({
            "status_code": 0,
            "sug_list": [
                {
                    "content": "Rurutia.wow",
                    "extra_info": {
                        "rich_sug_sec_uid": "MS4wLjABAAAA_test_sec_uid",
                        "rich_sug_user_id": "123456789",
                        "rich_sug_nickname": "Rurutia",
                        "rich_sug_short_id": "Rurutia.wow"
                    }
                }
            ]
        });
        let found = identities_in(&payload);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0]["unique_id"], "Rurutia.wow");
        assert_eq!(found[0]["sec_uid"], "MS4wLjABAAAA_test_sec_uid");
        assert_eq!(found[0]["uid"], "123456789");
        assert_eq!(found[0]["nickname"], "Rurutia");
    }
}
