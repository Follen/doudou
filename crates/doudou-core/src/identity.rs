//! User identity normalization, merging, caching, and recursive enrichment.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::error::Result;
use crate::io::{atomic_write_json, read_json, utc_now};
use crate::paths::{CURRENT_IDENTITY_SCHEMA, profile_paths};

pub const IDENTITY_FIELDS: &[&str] = &[
    "uid",
    "sec_uid",
    "unique_id",
    "nickname",
    "remark_name",
    "display_name",
];

const IDENTITY_KEYS: &[&str] = &[
    "sender",
    "author",
    "user",
    "mention",
    "mentions",
    "reference_user",
    "source_users",
    "joined_users",
    "participants",
    "members",
    "mentioned_users",
];

const IDENTITY_CONTAINER_KEYS: &[&str] = &[
    "candidates",
    "comments",
    "conversations",
    "messages",
    "records",
    "replies",
    "reply",
    "videos",
];

fn opt_text(value: Option<&Value>) -> Option<String> {
    match value {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.is_empty() => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::Bool(b)) => Some(b.to_string()),
        Some(other) => Some(other.to_string()),
    }
}

fn first_text(map: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| opt_text(map.get(*k)))
}

/// Normalize a raw user object into a `doudou.user-identity.v1` record.
pub fn normalize_identity(value: Option<&Value>, source: &str) -> Value {
    let empty = Map::new();
    let raw = value.and_then(Value::as_object).unwrap_or(&empty);
    let uid = first_text(raw, &["uid", "user_id", "userId"]);
    let sec_uid = first_text(raw, &["sec_uid", "secUid", "sec_user_id"]);
    let unique_id = first_text(raw, &["unique_id", "uniqueId"]);
    let nickname = first_text(raw, &["nickname", "nick_name", "name"]);
    let remark_name = first_text(raw, &["remark_name", "remarkName", "alias"]);
    let display_name = opt_text(raw.get("display_name"))
        .or_else(|| remark_name.clone())
        .or_else(|| nickname.clone())
        .or_else(|| unique_id.clone());

    let pairs: [(&str, Option<String>); 6] = [
        ("uid", uid),
        ("sec_uid", sec_uid),
        ("unique_id", unique_id),
        ("nickname", nickname),
        ("remark_name", remark_name),
        ("display_name", display_name),
    ];

    let available: Vec<&str> = pairs
        .iter()
        .filter_map(|(k, v)| v.as_ref().map(|_| *k))
        .collect();
    let missing: Vec<&str> = pairs
        .iter()
        .filter_map(|(k, v)| if v.is_none() { Some(*k) } else { None })
        .collect();
    let complete = ["uid", "sec_uid", "unique_id", "nickname"]
        .iter()
        .all(|k| available.contains(k));

    let existing_enrichment = raw.get("enrichment").and_then(Value::as_object);
    let sources: Vec<String> = existing_enrichment
        .and_then(|e| e.get("sources"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| vec![source.to_owned()]);
    let updated_at = existing_enrichment
        .and_then(|e| e.get("updated_at"))
        .and_then(Value::as_str)
        .map_or_else(utc_now, ToOwned::to_owned);

    let mut out = Map::new();
    out.insert("schema".to_owned(), json!(CURRENT_IDENTITY_SCHEMA));
    for (k, v) in pairs {
        out.insert(k.to_owned(), v.map_or(Value::Null, Value::String));
    }
    out.insert(
        "enrichment".to_owned(),
        json!({
            "status": if complete { "complete" } else { "partial" },
            "sources": sources,
            "available_fields": available,
            "missing_fields": missing,
            "updated_at": updated_at,
        }),
    );
    Value::Object(out)
}

/// Merge a candidate identity onto an existing identity, preferring non-null candidate fields.
pub fn merge_identity(current: Option<&Value>, candidate: Option<&Value>) -> Value {
    let left = normalize_identity(current, "existing");
    let right = normalize_identity(candidate, "candidate");
    let mut merged = Map::new();
    for key in IDENTITY_FIELDS {
        let chosen = opt_text(right.get(*key)).or_else(|| opt_text(left.get(*key)));
        merged.insert((*key).to_owned(), chosen.map_or(Value::Null, Value::String));
    }
    let mut seen = HashSet::new();
    let mut sources = Vec::new();
    for src in [current, candidate].into_iter().flatten() {
        if let Some(arr) = src
            .get("enrichment")
            .and_then(|e| e.get("sources"))
            .and_then(Value::as_array)
        {
            for item in arr {
                if let Some(s) = item.as_str()
                    && seen.insert(s.to_owned())
                {
                    sources.push(s.to_owned());
                }
            }
        }
    }
    let primary_source = sources.first().map_or("merged", String::as_str);
    let mut value = normalize_identity(Some(&Value::Object(merged)), primary_source);
    let final_sources = if sources.is_empty() {
        vec!["merged".to_owned()]
    } else {
        sources
    };
    if let Some(enr) = value.get_mut("enrichment").and_then(Value::as_object_mut) {
        enr.insert("sources".to_owned(), json!(final_sources));
    }

    if let Some(curr) = current {
        let same_fields = IDENTITY_FIELDS
            .iter()
            .all(|k| curr.get(*k) == value.get(*k));
        let curr_sources: Vec<String> = curr
            .get("enrichment")
            .and_then(|e| e.get("sources"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        if same_fields
            && curr_sources == final_sources
            && let Some(curr_enr) = curr.get("enrichment")
            && let Some(obj) = value.as_object_mut()
        {
            obj.insert("enrichment".to_owned(), curr_enr.clone());
        }
    }
    value
}

/// Return true if two identity records share any non-empty `uid`, `sec_uid`, or `unique_id`.
pub fn identity_matches(left: &Value, right: &Value) -> bool {
    ["uid", "sec_uid", "unique_id"].iter().any(|key| {
        matches!(
            (opt_text(left.get(*key)), opt_text(right.get(*key))),
            (Some(a), Some(b)) if a == b
        )
    })
}

/// Per-profile persistent identity cache stored at `message/profiles/<profile>/identities/index.json`.
#[derive(Debug, Clone)]
pub struct IdentityCache {
    filename: PathBuf,
    pub records: Vec<Value>,
}

impl IdentityCache {
    /// Load the identity cache for `profile`, creating an empty cache if absent.
    pub fn load(home: &Path, data_root: &Path, profile: &str) -> Result<Self> {
        let filename = profile_paths(home, data_root, profile)?
            .identities
            .join("index.json");
        let existing = read_json(&filename, true)?;
        let records = existing
            .and_then(|v| v.get("identities").cloned())
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        Ok(Self { filename, records })
    }

    /// Merge `value` into the cache and return the updated normalized record.
    pub fn merge(&mut self, value: &Value, source: &str) -> Value {
        let candidate = normalize_identity(Some(value), source);
        for record in &mut self.records {
            if identity_matches(record, &candidate) {
                *record = merge_identity(Some(record), Some(&candidate));
                return record.clone();
            }
        }
        self.records.push(candidate.clone());
        candidate
    }

    /// Enrich `value` from matching cached records without mutating the cache.
    pub fn enrich(&self, value: &Value, source: &str) -> Value {
        let candidate = normalize_identity(Some(value), source);
        for record in &self.records {
            if identity_matches(record, &candidate) {
                return merge_identity(Some(&candidate), Some(record));
            }
        }
        candidate
    }

    /// Look up a unique cached identity matching `unique_id` (case-insensitive) with a `sec_uid`.
    pub fn exact_unique_id(&self, unique_id: &str) -> Option<Value> {
        let needle = unique_id.to_lowercase();
        let matches: Vec<&Value> = self
            .records
            .iter()
            .filter(|rec| {
                opt_text(rec.get("unique_id")).is_some_and(|u| u.to_lowercase() == needle)
                    && opt_text(rec.get("sec_uid")).is_some()
            })
            .collect();
        if matches.len() == 1 {
            Some(matches[0].clone())
        } else {
            None
        }
    }

    /// Atomically persist the sorted cache to disk.
    pub fn save(&self) -> Result<()> {
        let mut sorted = self.records.clone();
        sorted.sort_by(|a, b| {
            let key = |v: &Value| {
                (
                    opt_text(v.get("sec_uid")).unwrap_or_default(),
                    opt_text(v.get("uid")).unwrap_or_default(),
                    opt_text(v.get("unique_id")).unwrap_or_default(),
                )
            };
            key(a).cmp(&key(b))
        });
        let doc = json!({
            "schema": "doudou.identity-cache.v1",
            "updated_at": utc_now(),
            "count": sorted.len(),
            "identities": sorted,
        });
        atomic_write_json(&self.filename, &doc, None)
    }
}

/// Recursively enrich known identity fields in a JSON value using `cache`.
pub fn enrich_known_identities(value: &Value, cache: &IdentityCache, source: &str) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| enrich_known_identities(item, cache, source))
                .collect(),
        ),
        Value::Object(map) => {
            let mut output = map.clone();
            let has_id = ["uid", "sec_uid", "unique_id", "secUid"]
                .iter()
                .any(|k| opt_text(output.get(*k)).is_some());
            if has_id {
                let enriched = cache.enrich(&Value::Object(output.clone()), source);
                if let Value::Object(enriched_map) = enriched {
                    for (k, v) in enriched_map {
                        output.insert(k, v);
                    }
                }
            }
            for key in IDENTITY_KEYS {
                if let Some(child) = output.get(*key).cloned() {
                    output.insert(
                        (*key).to_owned(),
                        enrich_known_identities(&child, cache, source),
                    );
                }
            }
            for key in IDENTITY_CONTAINER_KEYS {
                if let Some(child) = output.get(*key).cloned() {
                    output.insert(
                        (*key).to_owned(),
                        enrich_known_identities(&child, cache, source),
                    );
                }
            }
            if let Some(sys) = output.get("system_event").cloned()
                && sys.is_object()
            {
                output.insert(
                    "system_event".to_owned(),
                    enrich_known_identities(&sys, cache, source),
                );
            }
            if let Some(props) = output.get("property_list").cloned()
                && props.is_object()
            {
                output.insert(
                    "property_list".to_owned(),
                    enrich_known_identities(&props, cache, source),
                );
            }
            Value::Object(output)
        }
        other => other.clone(),
    }
}

/// Collect unique `sec_uid` values in encounter order from all identity fields inside `value`.
pub fn known_identity_sec_uids(value: &Value) -> Vec<String> {
    fn visit(
        current: &Value,
        identity_context: bool,
        seen: &mut HashSet<String>,
        out: &mut Vec<String>,
    ) {
        match current {
            Value::Array(items) => {
                for item in items {
                    visit(item, identity_context, seen, out);
                }
            }
            Value::Object(map) => {
                if identity_context
                    && let Some(sec_uid) = opt_text(map.get("sec_uid"))
                    && seen.insert(sec_uid.clone())
                {
                    out.push(sec_uid);
                }
                for key in IDENTITY_KEYS {
                    if let Some(child) = map.get(*key) {
                        visit(child, true, seen, out);
                    }
                }
                for key in IDENTITY_CONTAINER_KEYS {
                    if let Some(child) = map.get(*key) {
                        visit(child, false, seen, out);
                    }
                }
                if let Some(sys) = map.get("system_event")
                    && sys.is_object()
                {
                    visit(sys, false, seen, out);
                }
                if let Some(props) = map.get("property_list")
                    && props.is_object()
                {
                    visit(props, true, seen, out);
                }
            }
            _ => {}
        }
    }
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    visit(value, false, &mut seen, &mut out);
    out
}

/// Enrich sender, mention, and referenced-message user identities across a batch of messages.
pub fn enrich_message_identities(
    messages: &[Value],
    cache: &IdentityCache,
    source: &str,
    reference_catalog: &[Value],
) -> Vec<Value> {
    let mut catalog: BTreeMap<String, &Value> = BTreeMap::new();
    for item in reference_catalog.iter().chain(messages.iter()) {
        if let Some(mid) = opt_text(item.get("message_id")) {
            catalog.insert(mid, item);
        }
    }
    let mut output = Vec::with_capacity(messages.len());
    for message in messages {
        let mut obj = message.as_object().cloned().unwrap_or_else(Map::new);
        let mut mentions: Vec<Value> = obj
            .get("mentioned_users")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if let Some(rich) = obj
            .get("content")
            .and_then(|c| c.get("richTextInfos"))
            .and_then(Value::as_array)
        {
            for item in rich {
                let info_type = item.get("infoType").and_then(Value::as_i64).unwrap_or(0);
                if matches!(info_type, 1 | 6)
                    && let Some(uid) = item.get("info").and_then(|i| opt_text(i.get("uid")))
                {
                    mentions.push(json!({ "uid": uid }));
                }
            }
        }
        let mut merged_mentions: Vec<Value> = Vec::new();
        for mention in &mentions {
            let candidate = cache.enrich(mention, &format!("{source}-mention"));
            if let Some(existing) = merged_mentions
                .iter_mut()
                .find(|ex| identity_matches(ex, &candidate))
            {
                *existing = merge_identity(Some(existing), Some(&candidate));
            } else {
                merged_mentions.push(candidate);
            }
        }
        if !merged_mentions.is_empty() {
            obj.insert("mentioned_users".to_owned(), Value::Array(merged_mentions));
        }

        let reference_id = opt_text(obj.get("reply_to_message_id")).or_else(|| {
            obj.get("reference_info")
                .and_then(|r| opt_text(r.get("referenced_message_id")))
        });
        if let Some(ref_id) = reference_id {
            let ref_user = catalog
                .get(&ref_id)
                .and_then(|target| target.get("sender"))
                .or_else(|| obj.get("reference_user"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            obj.insert(
                "reference_user".to_owned(),
                cache.enrich(&ref_user, &format!("{source}-reference")),
            );
        }
        output.push(enrich_known_identities(&Value::Object(obj), cache, source));
    }
    output
}
