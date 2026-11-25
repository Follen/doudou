//! Structured protocol read services for chat and video domains.

pub mod chat;
pub mod transport;
pub mod video;

pub use chat::{
    GET_MESSAGES_ENDPOINT, INDEX_MAX_AGE_SECONDS, PARTICIPANTS_ENDPOINT,
    STRANGER_CONVERSATIONS_ENDPOINT, STRANGER_MESSAGES_ENDPOINT, apply_conversation_identities,
    collect_messages, enrich_conversation_identities, filter_conversations, list_conversations,
    refresh_session_index, resolve_members, resolve_user_identities,
};
pub use transport::{fetch_json, parse_result, to_protocol_fetch_response};
pub use video::{collect_user_videos, collect_video, resolve_identity};
