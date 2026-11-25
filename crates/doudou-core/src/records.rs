//! Core domain records for conversations and user identities.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Conversation classification derived strictly from Douyin's `conversation_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConversationKind {
    Direct,
    Group,
    Unknown,
}

impl ConversationKind {
    /// Map numeric Douyin `conversation_type` (`1` = direct, `2` = group).
    pub const fn from_type_code(conversation_type: i64) -> Self {
        match conversation_type {
            1 => Self::Direct,
            2 => Self::Group,
            _ => Self::Unknown,
        }
    }

    /// Lowercase string form matching the Doudou JSON schema.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Group => "group",
            Self::Unknown => "unknown",
        }
    }
}

/// Map numeric `conversation_type` to its string label (`direct`, `group`, or `unknown`).
pub const fn conversation_kind(conversation_type: i64) -> &'static str {
    ConversationKind::from_type_code(conversation_type).as_str()
}

/// Structured conversation summary record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conversation {
    pub conversation_id: String,
    pub conversation_short_id: String,
    pub conversation_type: i64,
    pub kind: String,
    pub display_name: String,
    #[serde(default)]
    pub members: Vec<Value>,
    #[serde(default)]
    pub ext: serde_json::Map<String, Value>,
}

/// Core 6-field Douyin user identity tuple.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub unique_id: Option<String>,
    pub sec_uid: Option<String>,
    pub uid: Option<String>,
    pub nickname: Option<String>,
    pub remark_name: Option<String>,
    pub display_name: Option<String>,
}
