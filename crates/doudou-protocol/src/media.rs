//! Encrypted Douyin chat image download, AES-GCM decryption, MD5 & format verification.

use std::path::Path;
use std::time::Duration;

use aes_gcm::aead::{Aead as _, KeyInit as _};
use aes_gcm::{Aes256Gcm, Nonce};
use doudou_core::{DoudouError, Result, atomic_write_bytes, sha256_bytes, sha256_str};
use md5::{Digest as _, Md5};
use serde_json::{Value, json};

/// Validate a 32-hex-character MD5 string and normalize to lowercase.
pub fn exact_md5(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    if text.len() == 32 && text.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(text.to_ascii_lowercase())
    } else {
        None
    }
}

/// Match a declared MD5 against either the encrypted envelope or decrypted plaintext MD5.
pub fn match_image_md5(
    declared: Option<&str>,
    encrypted_md5: &str,
    clear_md5: &str,
) -> Result<Option<&'static str>> {
    let Some(expected) = declared else {
        return Ok(None);
    };
    if expected == encrypted_md5 {
        return Ok(Some("encrypted"));
    }
    if expected == clear_md5 {
        return Ok(Some("clear"));
    }
    Err(DoudouError::new(
        "MEDIA_MD5_MISMATCH",
        "Image MD5 did not match encrypted or plaintext digest.",
    ))
}

/// Extract the highest-resolution image URL from a Douyin message `resource_url` object.
pub fn first_image_url(resource: &serde_json::Map<String, Value>) -> Option<String> {
    for key in [
        "origin_url_list",
        "large_url_list",
        "medium_url_list",
        "thumb_url_list",
    ] {
        if let Some(first) = resource
            .get(key)
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(Value::as_str)
            && !first.is_empty()
        {
            return Some(first.to_owned());
        }
    }
    None
}

/// Detect image extension and MIME type from decoded file magic bytes.
pub fn detect_image(value: &[u8]) -> Result<(&'static str, &'static str)> {
    if value.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok(("png", "image/png"));
    }
    if value.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok(("jpg", "image/jpeg"));
    }
    if value.starts_with(b"GIF87a") || value.starts_with(b"GIF89a") {
        return Ok(("gif", "image/gif"));
    }
    if value.len() >= 12 && &value[8..12] == b"WEBP" {
        return Ok(("webp", "image/webp"));
    }
    if value.len() >= 12
        && matches!(
            &value[4..12],
            b"ftypheic" | b"ftypheix" | b"ftyphevc" | b"ftyphevx" | b"ftypmif1"
        )
    {
        return Ok(("heic", "image/heic"));
    }
    Err(DoudouError::new(
        "MEDIA_PROCESSING_FAILED",
        "unrecognized image format",
    ))
}

/// Decrypt a Douyin AES-256-GCM encrypted image blob (`12-byte nonce || ciphertext || 16-byte tag`).
pub fn decrypt_image(encrypted: &[u8], secret_key_hex: &str) -> Result<Vec<u8>> {
    let key = hex::decode(secret_key_hex).map_err(|_| {
        DoudouError::new("MEDIA_PROCESSING_FAILED", "invalid AES-GCM image key hex")
    })?;
    if key.len() != 32 || encrypted.len() < 29 {
        return Err(DoudouError::new(
            "MEDIA_PROCESSING_FAILED",
            "invalid AES-GCM image envelope",
        ));
    }
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|_| DoudouError::new("MEDIA_PROCESSING_FAILED", "invalid AES-GCM image key"))?;
    let nonce = Nonce::from_slice(&encrypted[..12]);
    cipher
        .decrypt(nonce, &encrypted[12..])
        .map_err(|_| DoudouError::new("MEDIA_PROCESSING_FAILED", "AES-GCM image decryption failed"))
}

fn md5_hex(data: &[u8]) -> String {
    let mut hasher = Md5::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Verify and save an already-downloaded encrypted image payload for `message`.
pub fn process_downloaded_image(
    message: &Value,
    encrypted: &[u8],
    url: &str,
    directory: &Path,
) -> Result<Value> {
    let content = message.get("content").and_then(Value::as_object);
    let resource = content
        .and_then(|c| c.get("resource_url"))
        .and_then(Value::as_object)
        .ok_or_else(|| DoudouError::new("MEDIA_RESOURCE_MISSING", "Missing image resource."))?;
    let secret = resource
        .get("skey")
        .and_then(Value::as_str)
        .ok_or_else(|| DoudouError::new("MEDIA_RESOURCE_MISSING", "Missing image skey."))?;

    let raw_transport_md5 = resource.get("md5");
    let declared_transport_md5 = exact_md5(raw_transport_md5);
    if raw_transport_md5.is_some_and(|v| !v.is_null() && v.as_str() != Some(""))
        && declared_transport_md5.is_none()
    {
        return Err(DoudouError::new(
            "MEDIA_MD5_INVALID",
            "Invalid transport MD5.",
        ));
    }
    let encrypted_md5 = md5_hex(encrypted);
    let clear = decrypt_image(encrypted, secret)?;
    let (extension, mime) = detect_image(&clear)?;
    let actual_md5 = md5_hex(&clear);
    let resource_md5_scope = match_image_md5(
        declared_transport_md5.as_deref(),
        &encrypted_md5,
        &actual_md5,
    )?;
    let declared_content_md5 = exact_md5(content.and_then(|c| c.get("md5")));
    if let Some(expected) = declared_content_md5
        && expected != actual_md5
    {
        return Err(DoudouError::new(
            "MEDIA_MD5_MISMATCH",
            "Content MD5 mismatch.",
        ));
    }
    if let Some(declared_size) = resource.get("data_size").and_then(Value::as_u64)
        && declared_size != clear.len() as u64
    {
        return Err(DoudouError::new(
            "MEDIA_SIZE_MISMATCH",
            "Image size mismatch.",
        ));
    }
    let mid = message
        .get("message_id")
        .and_then(Value::as_str)
        .unwrap_or("image");
    let target = directory.join(format!("{mid}.{extension}"));
    atomic_write_bytes(&target, &clear, None)?;
    Ok(json!({
        "status": "downloaded",
        "retryable": false,
        "local_path": target.display().to_string(),
        "format": extension,
        "mime": mime,
        "bytes": clear.len(),
        "md5": actual_md5,
        "sha256": sha256_bytes(&clear),
        "encrypted_bytes": encrypted.len(),
        "encrypted_md5": encrypted_md5,
        "resource_md5_scope": resource_md5_scope,
        "resource_url_sha256": sha256_str(url),
    }))
}

/// Download, decrypt, verify, and persist image attachments for one IM message (`message_type == 27`).
pub async fn download_message_images(message: &Value, directory: &Path) -> Vec<Value> {
    let msg_type = message
        .get("message_type")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let resource = message
        .get("content")
        .and_then(Value::as_object)
        .and_then(|c| c.get("resource_url"))
        .and_then(Value::as_object);
    let Some(resource) = resource else {
        return Vec::new();
    };
    if msg_type != 27 {
        return Vec::new();
    }
    let Some(url) = first_image_url(resource) else {
        return vec![json!({
            "status": "failed",
            "retryable": true,
            "error_code": "MEDIA_RESOURCE_MISSING",
        })];
    };
    if resource.get("skey").and_then(Value::as_str).is_none() {
        return vec![json!({
            "status": "failed",
            "retryable": true,
            "error_code": "MEDIA_RESOURCE_MISSING",
        })];
    }
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(45))
        .user_agent("Mozilla/5.0")
        .build()
    else {
        return vec![json!({
            "status": "failed",
            "retryable": true,
            "error_code": "MEDIA_PROCESSING_FAILED",
            "resource_url_sha256": sha256_str(&url),
        })];
    };
    let encrypted = match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.bytes().await {
            Ok(b) => b.to_vec(),
            Err(_) => {
                return vec![json!({
                    "status": "failed",
                    "retryable": true,
                    "error_code": "MEDIA_PROCESSING_FAILED",
                    "resource_url_sha256": sha256_str(&url),
                })];
            }
        },
        _ => {
            return vec![json!({
                "status": "failed",
                "retryable": true,
                "error_code": "MEDIA_PROCESSING_FAILED",
                "resource_url_sha256": sha256_str(&url),
            })];
        }
    };
    match process_downloaded_image(message, &encrypted, &url, directory) {
        Ok(record) => vec![record],
        Err(err) => {
            let code = if err.code.starts_with("MEDIA_") {
                err.code
            } else {
                "MEDIA_PROCESSING_FAILED".to_owned()
            };
            vec![json!({
                "status": "failed",
                "retryable": true,
                "error_code": code,
                "resource_url_sha256": sha256_str(&url),
            })]
        }
    }
}
