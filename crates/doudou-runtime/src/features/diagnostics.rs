//! Runtime environment `doctor`, chat/video UI adapter probes (`ui_probe` / `page_doctor`),
//! and headed protocol capture diagnostics.

use std::collections::BTreeSet;
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use doudou_cdp::{
    AuthPolicy, CdpBrowser, CdpPageOps, ChromeLaunchOptions, ChromeSession, EvalWorld, LaunchMode,
    PreserveUrl, SessionMode, discover_all_chromes, discover_chrome,
};
use doudou_core::{
    DoudouError, Result, absolutize_path, atomic_write_bytes, atomic_write_json, is_dir_writable,
    profile_paths, random_id, read_json, sha256_bytes, sha256_str, utc_now,
};
use doudou_storage::list_profiles;
use doudou_ui::{
    AuthChallengeComponent, CaptureFilter, ChatComposerComponent, CommentItemComponent,
    ComponentContext, ConversationListComponent, ImagePreviewComponent, MessageItemComponent,
    TutorialDialogComponent, UiPage, VideoCommentComposerComponent, VideoCommentsPanelComponent,
    not_exercised_probe, visible_candidates,
};
use serde_json::{Value, json};
use tokio::sync::Mutex as AsyncMutex;

use crate::broker::registry::list_brokers;
use crate::cdp_ui_bridge::CdpUiPage;
use crate::lock::lock_is_live;

/// Options controlling [`doctor`] runtime health checks.
#[derive(Debug, Clone, Default)]
pub struct DoctorOptions {
    /// When `true`, launch a real headless Chrome into a temporary profile directory and verify
    /// CDP isolated-world evaluation works without `Runtime.enable`.
    pub launch_probe: bool,
    /// Optional explicit Chrome executable override for discovery and launch probing.
    pub chrome_binary: Option<PathBuf>,
}

impl DoctorOptions {
    /// Construct default options (no live Chrome launch probe).
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable or disable the live headless Chrome launch probe.
    #[must_use]
    pub fn with_launch_probe(mut self, launch_probe: bool) -> Self {
        self.launch_probe = launch_probe;
        self
    }

    /// Override the Chrome binary path used during probing.
    #[must_use]
    pub fn with_chrome_binary(mut self, binary: PathBuf) -> Self {
        self.chrome_binary = Some(binary);
        self
    }
}

async fn run_launch_probe(chrome_binary: Option<&Path>) -> Value {
    let temp_root =
        std::env::temp_dir().join(format!("doudou-launch-probe-{}", random_id("probe")));
    let mut opts = ChromeLaunchOptions::new(temp_root.clone(), LaunchMode::Headless)
        .with_startup_timeout(Duration::from_secs(20));
    if let Some(bin) = chrome_binary {
        opts = opts.with_binary(bin.to_path_buf());
    }

    let result = async {
        let mut browser = CdpBrowser::launch(opts).await?;
        let endpoint = browser.process().endpoint().clone();
        let page = browser.open_page().await?;
        let sum = page.eval_json("1 + 1", EvalWorld::Isolated).await?;
        let webdriver = page
            .eval_json(
                "typeof navigator.webdriver === 'undefined' || navigator.webdriver === false",
                EvalWorld::Main,
            )
            .await?;
        let _ = page.close().await;
        let _ = browser.close().await;
        Ok::<Value, DoudouError>(json!({
            "ok": sum == json!(2),
            "cdp_port": endpoint.port,
            "browser_version": endpoint.browser_version,
            "isolated_eval_ok": sum == json!(2),
            "webdriver_hidden": webdriver == json!(true),
            "runtime_enable_used": false,
        }))
    }
    .await;

    let _ = fs::remove_dir_all(&temp_root);
    match result {
        Ok(report) => report,
        Err(err) => json!({
            "ok": false,
            "error_code": err.code,
            "message": err.message,
        }),
    }
}

/// Inspect Doudou runtime readiness: `doudou_home` writability, `data_root` writability,
/// Chrome discovery/version, optional live headless CDP launch probe, profiles, and active brokers.
pub async fn doctor(home: &Path, data_root: &Path, options: &DoctorOptions) -> Result<Value> {
    let home = absolutize_path(home);
    let data_root = absolutize_path(data_root);
    let home_writable = is_dir_writable(&home);
    let data_root_writable = is_dir_writable(&data_root);

    let discovered_all = discover_all_chromes();
    let primary_chrome = match &options.chrome_binary {
        Some(bin) if bin.is_file() => {
            let version = doudou_cdp::probe_chrome_version(bin);
            Ok(json!({
                "available": true,
                "path": bin.display().to_string(),
                "brand": doudou_cdp::infer_brand(bin),
                "version": version,
                "source": "explicit",
            }))
        }
        Some(bin) => Err(DoudouError::new(
            "CHROME_NOT_FOUND",
            format!("Configured Chrome binary does not exist: {}", bin.display()),
        )),
        None => discover_chrome().map(|bin| {
            json!({
                "available": true,
                "path": bin.path.display().to_string(),
                "brand": bin.brand,
                "version": bin.version,
                "source": bin.source,
            })
        }),
    };

    let (chrome_ok, chrome_report) = match primary_chrome {
        Ok(val) => (true, val),
        Err(err) => (
            false,
            json!({
                "available": false,
                "error_code": err.code,
                "message": err.message,
            }),
        ),
    };

    let launch_probe = if options.launch_probe {
        Some(run_launch_probe(options.chrome_binary.as_deref()).await)
    } else {
        None
    };

    let profiles = list_profiles(&home, &data_root, lock_is_live).unwrap_or_default();
    let brokers = list_brokers(&home).unwrap_or_default();

    let probe_ok = launch_probe
        .as_ref()
        .is_none_or(|p| p.get("ok").and_then(Value::as_bool).unwrap_or(false));
    let overall_ok = home_writable && data_root_writable && chrome_ok && probe_ok;

    Ok(json!({
        "schema": "doudou.doctor.v2",
        "checked_at": utc_now(),
        "status": if overall_ok { "ok" } else { "degraded" },
        "ok": overall_ok,
        "home": {
            "path": home.display().to_string(),
            "writable": home_writable,
        },
        "data_root": {
            "path": data_root.display().to_string(),
            "writable": data_root_writable,
        },
        "chrome": chrome_report,
        "chrome_candidates_count": discovered_all.len(),
        "launch_probe": launch_probe,
        "profiles": {
            "count": profiles.len(),
            "items": profiles,
        },
        "brokers": {
            "count": brokers.len(),
            "items": brokers,
        },
    }))
}

async fn doctor_stage<T, Fut>(name: &str, timeout_secs: u64, fut: Fut) -> Result<T>
where
    Fut: Future<Output = Result<T>>,
{
    match tokio::time::timeout(Duration::from_secs(timeout_secs), fut).await {
        Ok(res) => res,
        Err(_) => Err(DoudouError::new(
            "PAGE_DOCTOR_STAGE_TIMEOUT",
            "A bounded Page Doctor stage exceeded its time limit.",
        )
        .with_details(json!({
            "stage": name,
            "timeout_seconds": timeout_secs,
        }))),
    }
}

/// Run bounded UI component & adapter drift probes (`page.doctor` / `ui_probe`) on the current page.
pub async fn ui_probe(
    session: &Arc<AsyncMutex<ChromeSession>>,
    home: &Path,
    data_root: &Path,
    profile: &str,
    params: &Value,
) -> Result<Value> {
    let paths = profile_paths(home, data_root, profile)?;
    let report_root = paths.diagnostics.join("page-doctor");
    let stage_path = report_root.join("active-stage.json");

    let _ = atomic_write_json(
        &stage_path,
        &json!({ "stage": "ensure_ready", "updated_at": utc_now() }),
        None,
    );
    let ready = doctor_stage("ensure_ready", 60, async {
        let mut guard = session.lock().await;
        let login = guard.ensure_authenticated(AuthPolicy::FailFast).await?;
        let status = guard.status().await;
        Ok(json!({
            "status": "ready",
            "login": login,
            "page_url": status.page_url,
            "initialization_count": status.context_generation,
        }))
    })
    .await?;

    let exercise = params
        .get("conversation")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let mut exercise_result = Value::Null;

    let cdp_page = {
        let mut guard = session.lock().await;
        let page = guard.ensure_page().await?;
        let cur_url = page.current_url().await.unwrap_or_default();
        if exercise.is_some() && !cur_url.starts_with("https://www.douyin.com/chat") {
            let _ = atomic_write_json(
                &stage_path,
                &json!({ "stage": "open_chat", "updated_at": utc_now() }),
                None,
            );
            let _ = doctor_stage("open_chat", 60, async {
                guard.navigate_douyin("https://www.douyin.com/chat").await
            })
            .await?;
        }
        guard.ensure_page().await?
    };

    let page = CdpUiPage::new(cdp_page);
    let _ = atomic_write_json(
        &stage_path,
        &json!({ "stage": "page_shape", "updated_at": utc_now() }),
        None,
    );
    let mut shape = doctor_stage("page_shape", 10, async {
        page.eval_json(
            r"() => ({
              url_path: location.pathname,
              scripts: [...document.scripts].map(item => item.src).filter(Boolean).slice(0, 300).map(value => {
                try { const url = new URL(value); return { origin: url.origin, path: url.pathname }; }
                catch (_) { return { origin: '', path: value }; }
              }),
            })",
            None,
        )
        .await
    })
    .await?;

    let mut scripts_set = BTreeSet::new();
    if let Some(arr) = shape
        .as_object_mut()
        .and_then(|m| m.remove("scripts"))
        .and_then(|v| v.as_array().cloned())
    {
        for item in arr {
            let origin = item.get("origin").and_then(Value::as_str).unwrap_or("");
            let path = item.get("path").and_then(Value::as_str).unwrap_or("");
            scripts_set.insert(format!("{origin}{path}"));
        }
    }
    let scripts: Vec<String> = scripts_set.into_iter().collect();
    let script_set_sha256 = sha256_str(&scripts.join("\n"));

    let previous = latest_previous_report(&report_root);
    let previous_sha = previous
        .as_ref()
        .and_then(|p| p.get("script_set_sha256"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);

    let mut component_reports: Vec<Value> = Vec::new();
    let _ = atomic_write_json(
        &stage_path,
        &json!({ "stage": "auth_probe", "updated_at": utc_now() }),
        None,
    );
    let auth_reports = doctor_stage("auth_probe", 10, async {
        AuthChallengeComponent::new(&page).probe(true).await
    })
    .await?;
    component_reports.extend(auth_reports);

    let url_path = shape
        .get("url_path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();

    if url_path.starts_with("/chat") {
        let conv_list = ConversationListComponent::new(&page);
        let _ = atomic_write_json(
            &stage_path,
            &json!({ "stage": "conversation_list_probe", "updated_at": utc_now() }),
            None,
        );
        component_reports
            .push(doctor_stage("conversation_list_probe", 20, conv_list.probe(true)).await?);
        if let Some(ref conv_name) = exercise {
            let _ = atomic_write_json(
                &stage_path,
                &json!({ "stage": "conversation_select", "updated_at": utc_now() }),
                None,
            );
            let ops = doctor_stage("conversation_select", 40, conv_list.open(conv_name)).await?;
            page.sleep_ms(500).await?;
            exercise_result = json!({
                "capability": "chat.conversation.open",
                "conversation": conv_name,
                "dom_operations": ops,
                "selected": true,
            });
        }
        let composer = ChatComposerComponent::new(&page);
        let comp_cands = composer.candidates().await?;
        let selected = !visible_candidates(&page, &comp_cands, 20).await?.is_empty();
        let _ = atomic_write_json(
            &stage_path,
            &json!({ "stage": "chat_composer_probes", "updated_at": utc_now() }),
            None,
        );
        let comp_probes = doctor_stage(
            "chat_composer_probes",
            15,
            composer.operation_probes(selected),
        )
        .await?;
        component_reports.extend(comp_probes);

        let message_items = MessageItemComponent::new(&page);
        component_reports.push(not_exercised_probe(
            &ComponentContext::new(
                MessageItemComponent::NAME,
                MessageItemComponent::VERSION,
                "chat.message.target",
                "protocol message -> local message item",
            ),
            "protocol_target_not_bound",
        ));
        component_reports.push(message_items.context_menu_probe().await?);
        let image_preview = ImagePreviewComponent::new(&page);
        component_reports.push(image_preview.probe(selected).await?);
        component_reports.push(image_preview.preview_probe().await?);
    } else if url_path.starts_with("/video/") || url_path.starts_with("/note/") {
        component_reports.push(TutorialDialogComponent.probe(&page).await?);
        component_reports.push(VideoCommentsPanelComponent.probe(&page, true).await?);

        let composer = VideoCommentComposerComponent::new(&page);
        let comp_cands = composer.candidates().await?;
        let comp_visible = !visible_candidates(&page, &comp_cands, 20).await?.is_empty();
        let comment_items = CommentItemComponent::new(&page);
        let item_cands = comment_items.candidates().await?;
        let list_visible = !visible_candidates(&page, &item_cands, 20).await?.is_empty();

        component_reports.push(composer.probe(comp_visible).await?);
        component_reports.push(comment_items.probe(list_visible).await?);
        component_reports.push(not_exercised_probe(
            &ComponentContext::new(
                CommentItemComponent::NAME,
                CommentItemComponent::VERSION,
                "video.comment.reply",
                "protocol comment -> local reply action",
            ),
            "protocol_target_not_bound",
        ));
        component_reports.push(not_exercised_probe(
            &ComponentContext::new(
                CommentItemComponent::NAME,
                CommentItemComponent::VERSION,
                "video.comment.like",
                "protocol comment -> local stats/like action",
            ),
            "protocol_target_not_bound",
        ));
    }

    let component_drift: Vec<Value> = component_reports
        .iter()
        .filter(|item| item.get("status").and_then(Value::as_str) == Some("drifted"))
        .cloned()
        .collect();
    let not_exercised: Vec<Value> = component_reports
        .iter()
        .filter(|item| item.get("status").and_then(Value::as_str) == Some("not_exercised"))
        .cloned()
        .collect();
    let adapter_reports: Vec<Value> = component_reports
        .iter()
        .map(|item| {
            json!({
                "name": item.get("component").cloned().unwrap_or(Value::Null),
                "version": item.get("component_version").cloned().unwrap_or(Value::Null),
                "capability": item.get("capability").cloned().unwrap_or(Value::Null),
                "status": item.get("status").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();

    let (init_req_bytes, init_resp_bytes) = {
        let guard = session.lock().await;
        match guard.captured_im_init() {
            Some((req, resp)) => (req.len(), resp.len()),
            None => (0, 0),
        }
    };

    let bundle_changed = previous_sha
        .as_deref()
        .is_some_and(|prev| prev != script_set_sha256);
    let has_drift = !component_drift.is_empty();

    let record = json!({
        "schema": "doudou.page-doctor.v1",
        "completed_at": utc_now(),
        "profile_id": paths.profile_id,
        "ready": ready,
        "page": shape,
        "exercise": exercise,
        "exercise_result": exercise_result,
        "adapters": adapter_reports,
        "components": component_reports,
        "adapter_status": if has_drift { "drifted" } else { "compatible" },
        "adapter_findings": component_drift,
        "not_exercised": not_exercised,
        "adapter_error_code": if has_drift { json!("PAGE_ADAPTER_DRIFTED") } else { Value::Null },
        "script_count": scripts.len(),
        "script_set_sha256": script_set_sha256,
        "previous_script_set_sha256": previous_sha,
        "bundle_changed": bundle_changed,
        "im": {
            "request_bytes": init_req_bytes,
            "response_bytes": init_resp_bytes,
        },
        "react_fiber_traversal": false,
    });

    let report_dir = report_root.join(random_id("doctor"));
    let report_file = report_dir.join("report.json");
    atomic_write_json(&report_file, &record, None)?;
    let _ = fs::remove_file(&stage_path);

    let report_rel = report_file
        .strip_prefix(data_root)
        .unwrap_or(&report_file)
        .display()
        .to_string();

    let mut out = record.as_object().cloned().unwrap_or_default();
    out.insert(
        "status".to_owned(),
        json!(if has_drift { "adapter_drifted" } else { "ok" }),
    );
    out.insert("report".to_owned(), json!(report_rel));
    Ok(Value::Object(out))
}

/// Navigate to `/chat` and run chat component probes (optionally exercising one conversation).
pub async fn diagnose_chat(
    session: &Arc<AsyncMutex<ChromeSession>>,
    home: &Path,
    data_root: &Path,
    profile: &str,
    conversation: Option<&str>,
) -> Result<Value> {
    {
        let mut guard = session.lock().await;
        let _ = guard.navigate_douyin("https://www.douyin.com/chat").await?;
    }
    let params = match conversation {
        Some(name) => json!({ "conversation": name }),
        None => json!({}),
    };
    ui_probe(session, home, data_root, profile, &params).await
}

/// Navigate to `/video/{aweme_id}` and run video component probes.
pub async fn diagnose_video(
    session: &Arc<AsyncMutex<ChromeSession>>,
    home: &Path,
    data_root: &Path,
    profile: &str,
    aweme_id: &str,
) -> Result<Value> {
    {
        let mut guard = session.lock().await;
        let _ = guard
            .navigate_douyin(&format!("https://www.douyin.com/video/{aweme_id}"))
            .await?;
    }
    ui_probe(session, home, data_root, profile, &json!({})).await
}

/// Switch into headed mode, wait for one complete `/v1/message/send` Protobuf exchange,
/// restore headless mode, and persist `request.bin`, `response.bin`, and `manifest.json`.
pub async fn protocol_capture(
    session: &Arc<AsyncMutex<ChromeSession>>,
    home: &Path,
    data_root: &Path,
    profile: &str,
    timeout_seconds: f64,
) -> Result<Value> {
    let page = {
        let mut guard = session.lock().await;
        let _ = guard
            .switch_mode(SessionMode::HeadedTakeover, PreserveUrl::KeepDouyinUrl)
            .await?;
        let _ = guard.ensure_authenticated(AuthPolicy::FailFast).await?;
        guard.ensure_page().await?
    };

    let ui_page = CdpUiPage::new(page);
    let handle = ui_page
        .start_network_capture(CaptureFilter::contains(["/v1/message/send"]))
        .await?;

    let deadline = Instant::now() + Duration::from_secs_f64(timeout_seconds.max(0.1));
    while Instant::now() < deadline {
        let snap = handle.snapshot().await?;
        if !snap.responses.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    let snapshot = handle.stop().await?;
    {
        let mut guard = session.lock().await;
        let _ = guard
            .switch_mode(SessionMode::Headless, PreserveUrl::KeepDouyinUrl)
            .await;
    }

    let send_requests: Vec<&[u8]> = snapshot
        .requests
        .iter()
        .filter(|r| r.path.contains("/v1/message/send"))
        .map(|r| r.body.as_slice())
        .collect();
    let send_responses: Vec<&[u8]> = snapshot
        .responses
        .iter()
        .filter(|r| r.path.contains("/v1/message/send"))
        .map(|r| r.body.as_slice())
        .collect();

    if send_requests.len() != 1 || send_responses.len() != 1 {
        return Err(DoudouError::new(
            "CAPTURE_INCOMPLETE",
            "Capture did not observe exactly one complete normal page send.",
        )
        .with_details(json!({
            "request_count": send_requests.len(),
            "response_count": send_responses.len(),
        })));
    }

    let paths = profile_paths(home, data_root, profile)?;
    let directory = paths
        .diagnostics
        .join("captures")
        .join(random_id("capture"));
    atomic_write_bytes(&directory.join("request.bin"), send_requests[0], None)?;
    atomic_write_bytes(&directory.join("response.bin"), send_responses[0], None)?;
    let manifest = json!({
        "schema": "doudou.protocol-capture.v1",
        "completed_at": utc_now(),
        "complete": true,
        "request": {
            "file": "request.bin",
            "bytes": send_requests[0].len(),
            "sha256": sha256_bytes(send_requests[0]),
        },
        "response": {
            "file": "response.bin",
            "bytes": send_responses[0].len(),
            "sha256": sha256_bytes(send_responses[0]),
        },
        "network": snapshot.to_network_json(),
    });
    atomic_write_json(&directory.join("manifest.json"), &manifest, None)?;
    let rel = directory
        .strip_prefix(data_root)
        .unwrap_or(&directory)
        .display()
        .to_string();

    Ok(json!({
        "status": "captured",
        "directory": rel,
        "manifest": manifest,
    }))
}

fn latest_previous_report(report_root: &Path) -> Option<Value> {
    let entries = fs::read_dir(report_root).ok()?;
    let mut reports: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let report = e.path().join("report.json");
            let meta = fs::metadata(&report).ok()?;
            let modified = meta.modified().ok()?;
            Some((modified, report))
        })
        .collect();
    reports.sort_by_key(|(mtime, _)| std::cmp::Reverse(*mtime));
    let (_, newest) = reports.into_iter().next()?;
    read_json(&newest, true).ok().flatten()
}
