//! Doudou runtime layer: cross-platform process identity, atomic profile locks,
//! loopback TCP broker server/client/registry, CDP-to-UI bridge, protocol reads,
//! and chat/video/diagnostic feature services.

pub mod broker;
pub mod cdp_ui_bridge;
pub mod features;
pub mod lock;
pub mod process;
pub mod reads;

pub use broker::{
    BROKER_DESCRIPTOR_SCHEMA, BROKER_REGISTRATION_SCHEMA, BROKER_RESPONSE_LIMIT_BYTES,
    BrokerClient, BrokerDescriptor, BrokerEndpoint, BrokerHandler, BrokerLauncher,
    BrokerRegistration, BrokerRequest, BrokerServer, BrokerServerConfig, CommandSemantics,
    DEFAULT_COMMAND_TIMEOUT_SECONDS, DETERMINISTIC_WRITE_ERRORS, FnBrokerHandler,
    MAX_COMMAND_TIMEOUT_SECONDS, MAX_FRAME_BYTES, MIN_COMMAND_TIMEOUT_SECONDS, broker_status,
    command_timeout_seconds, constant_time_eq, descriptor_is_live, list_brokers, read_bounded_line,
    register_broker, registration_path, stop_all_brokers, stop_broker, unregister_broker,
};
pub use cdp_ui_bridge::{CdpNetworkCaptureHandle, CdpUiPage, CdpUiSession, parse_key_spec};
pub use features::{
    DefaultBrokerHandler, DoctorOptions, diagnose_chat, diagnose_video, doctor, protocol_capture,
    ui_probe,
};
pub use lock::{LockOwner, PROFILE_LOCK_SCHEMA, ProfileLock, lock_is_live};
pub use process::{
    ProcessIdentity, current_process_identity, is_recorded_process_live,
    parse_windows_process_json, proc_stat_is_zombie, process_identity, same_process,
    windows_process_query_script,
};
pub use reads::{
    GET_MESSAGES_ENDPOINT, INDEX_MAX_AGE_SECONDS, PARTICIPANTS_ENDPOINT,
    STRANGER_CONVERSATIONS_ENDPOINT, STRANGER_MESSAGES_ENDPOINT, apply_conversation_identities,
    collect_messages, collect_user_videos, collect_video, enrich_conversation_identities,
    fetch_json, filter_conversations, list_conversations, parse_result, refresh_session_index,
    resolve_identity, resolve_members, resolve_user_identities, to_protocol_fetch_response,
};
