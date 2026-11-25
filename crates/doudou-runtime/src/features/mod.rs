//! Feature-layer services for chat, video, diagnostics, and broker dispatch.

pub mod chat;
pub mod diagnostics;
pub mod dispatch;
pub mod video;

pub use diagnostics::{
    DoctorOptions, diagnose_chat, diagnose_video, doctor, protocol_capture, ui_probe,
};
pub use dispatch::DefaultBrokerHandler;
