//! Default [`BrokerHandler`] routing all Douyin chat, video, and diagnostic commands.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use doudou_cdp::{AuthPolicy, ChromeSession, SessionMode};
use doudou_core::{DoudouError, Result};
use serde_json::{Value, json};
use tokio::sync::Mutex as AsyncMutex;

use crate::broker::server::BrokerHandler;
use crate::features::{auth, chat, diagnostics, video};
use crate::reads::chat::refresh_session_index;

/// Production broker command router matching Python's `features/broker.py::dispatch`.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultBrokerHandler;

#[async_trait]
impl BrokerHandler for DefaultBrokerHandler {
    async fn handle(
        &self,
        command: &str,
        params: &Value,
        session: &Arc<AsyncMutex<ChromeSession>>,
        home: &Path,
        data_root: &Path,
        profile_name: &str,
    ) -> Result<Option<Value>> {
        let result = match command {
            "session.auth.start" => {
                auth::auth_start(session, home, data_root, profile_name, params).await?
            }
            "session.auth.status" => {
                auth::auth_status(session, home, data_root, profile_name).await?
            }
            "session.auth.refresh_qr" => {
                auth::auth_refresh_qr(session, home, data_root, profile_name).await?
            }
            "session.auth.sms_send" => {
                auth::auth_sms_send(session, home, data_root, profile_name, params).await?
            }
            "session.auth.sms_verify" => {
                auth::auth_sms_verify(session, home, data_root, profile_name, params).await?
            }
            "session.auth.wait" => {
                auth::auth_wait(session, home, data_root, profile_name, params).await?
            }
            "session.refresh" => {
                let is_fake = {
                    let guard = session.lock().await;
                    guard.mode() == SessionMode::Fake
                };
                if is_fake {
                    let mut guard = session.lock().await;
                    let login = guard.ensure_authenticated(AuthPolicy::FailFast).await?;
                    let status = guard.status().await;
                    json!({
                        "status": "ready",
                        "initialization_count": status.context_generation,
                        "page_url": status.page_url,
                        "login": login,
                    })
                } else {
                    let mut guard = session.lock().await;
                    refresh_session_index(&mut guard, home, data_root, profile_name, true).await?
                }
            }
            "conversations.refresh" => {
                let force = params
                    .get("force")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let kind = params.get("kind").and_then(Value::as_str).unwrap_or("all");
                let inbox = params.get("inbox").and_then(Value::as_str).unwrap_or("all");
                chat::conversations(session, home, data_root, profile_name, force, kind, inbox)
                    .await?
            }
            "messages.collect" => {
                chat::collect(session, home, data_root, profile_name, params).await?
            }
            "chat.send.preview" => {
                chat::plan_send(session, home, data_root, profile_name, params).await?
            }
            "chat.send.execute" => {
                chat::execute_send(session, home, data_root, profile_name, params).await?
            }
            "chat.send.batch.preview" => {
                chat::plan_batch(session, home, data_root, profile_name, params).await?
            }
            "chat.send.batch.execute" => {
                chat::execute_batch(session, home, data_root, profile_name, params).await?
            }
            "chat.like.preview" => {
                chat::plan_like(session, home, data_root, profile_name, params).await?
            }
            "chat.like.execute" => {
                chat::execute_like(session, home, data_root, profile_name, params).await?
            }
            "identity.resolve" => {
                let douyin_id = params
                    .get("douyin_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| DoudouError::new("INVALID_PARAMS", "Missing douyin_id."))?;
                video::identity(session, home, data_root, profile_name, douyin_id).await?
            }
            "videos.collect" => {
                video::videos(session, home, data_root, profile_name, params).await?
            }
            "video.collect" => video::video(session, home, data_root, profile_name, params).await?,
            "video.interaction.preview" => {
                let operation = params
                    .get("operation")
                    .and_then(Value::as_str)
                    .ok_or_else(|| DoudouError::new("INVALID_PARAMS", "Missing operation."))?;
                let payload = params.get("payload").unwrap_or(&Value::Null);
                video::preview_interaction(home, data_root, profile_name, operation, payload)?
            }
            "video.interaction.execute" => {
                let operation = params
                    .get("operation")
                    .and_then(Value::as_str)
                    .ok_or_else(|| DoudouError::new("INVALID_PARAMS", "Missing operation."))?;
                video::execute_interaction(
                    session,
                    home,
                    data_root,
                    profile_name,
                    operation,
                    params,
                )
                .await?
            }
            "page.doctor" => {
                diagnostics::ui_probe(session, home, data_root, profile_name, params).await?
            }
            "protocol.capture" => {
                let timeout = params
                    .get("timeout_seconds")
                    .and_then(Value::as_f64)
                    .unwrap_or(600.0);
                diagnostics::protocol_capture(session, home, data_root, profile_name, timeout)
                    .await?
            }
            _ => return Ok(None),
        };
        Ok(Some(result))
    }
}
