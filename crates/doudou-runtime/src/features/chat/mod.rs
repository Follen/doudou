//! Chat feature services (conversations, collection, single send/like, sequential batch send).

pub mod batch;
pub mod service;

pub use batch::{
    MIN_BATCH_INTERVAL_SECONDS, execute_batch, execute_batch_with, plan_batch,
    plan_batch_from_conversations,
};
pub use service::{
    PlanExecutionGuard, collect, conversations, execute_like, execute_like_with_readback,
    execute_send, execute_send_with, execute_send_with_readback, plan_like,
    plan_like_for_conversation, plan_send, plan_send_for_conversation, verify_like_readback,
    verify_send_readback,
};
