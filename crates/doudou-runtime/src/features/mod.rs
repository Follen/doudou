//! Feature-layer services for chat, video, diagnostics, and broker dispatch.

pub mod auth;
pub mod chat;
pub mod diagnostics;
pub mod dispatch;
pub mod video;

pub use auth::{
    auth_refresh_qr, auth_sms_send, auth_sms_verify, auth_start, auth_status, auth_wait,
};

pub use diagnostics::{
    DoctorOptions, diagnose_chat, diagnose_video, doctor, protocol_capture, ui_probe,
};
pub use dispatch::DefaultBrokerHandler;
