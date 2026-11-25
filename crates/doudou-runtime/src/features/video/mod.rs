//! Video feature services (identity, videos, video detail/comments, comment interactions).

pub mod service;

pub use service::{
    execute_comment, execute_comment_like, execute_interaction, execute_interaction_with,
    execute_reply, identity, plan_comment, plan_comment_like, plan_reply, preview_interaction,
    verify_interaction_readback, video, videos,
};
