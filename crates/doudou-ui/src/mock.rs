//! In-memory `MockUiPage` and `MockUiSession` test doubles for deterministic UI testing.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use doudou_core::{DoudouError, Result};
use serde_json::{Value, json};

use crate::page::{
    BoundingBox, CaptureFilter, CapturedRequest, CapturedResponse, ElementId, MouseButton,
    NetworkCaptureHandle, NetworkCaptureSnapshot, PointHit, SelectorQuery, TextMatchMode, UiPage,
    UiSession,
};

type EvalHook = Arc<dyn Fn(&ElementId, &str, Option<&Value>) -> Result<Value> + Send + Sync>;
type ClickHook = Arc<dyn Fn(&mut MockState, &ElementId, MouseButton) -> Result<()> + Send + Sync>;
type PointClickHook =
    Arc<dyn Fn(&mut MockState, f64, f64, MouseButton) -> Result<()> + Send + Sync>;
type QueryOverride = Arc<
    dyn Fn(&mut MockState, Option<&ElementId>, &SelectorQuery) -> Option<Vec<ElementId>>
        + Send
        + Sync,
>;

/// Simulated DOM element stored inside [`MockUiPage`].
#[derive(Debug, Clone)]
pub struct MockElement {
    pub id: ElementId,
    pub tag: String,
    pub classes: Vec<String>,
    pub attributes: BTreeMap<String, String>,
    pub parent: Option<ElementId>,
    pub visible: bool,
    pub enabled: bool,
    pub text: String,
    pub boxes: VecDeque<Option<BoundingBox>>,
    pub last_box: Option<BoundingBox>,
    pub after_scroll_box: Option<BoundingBox>,
    pub media_labels: Vec<String>,
    pub scroll_calls: usize,
    pub click_calls: usize,
    pub key_presses: Vec<String>,
    pub typed_texts: Vec<String>,
    pub click_errors: VecDeque<DoudouError>,
    pub decorative: bool,
    pub focused: bool,
}

impl MockElement {
    /// Construct a visible, enabled mock element with a single bounding box.
    pub fn new(id: impl Into<String>, tag: impl Into<String>, bbox: Option<BoundingBox>) -> Self {
        Self {
            id: ElementId::new(id),
            tag: tag.into().to_uppercase(),
            classes: Vec::new(),
            attributes: BTreeMap::new(),
            parent: None,
            visible: true,
            enabled: true,
            text: String::new(),
            boxes: VecDeque::new(),
            last_box: bbox,
            after_scroll_box: None,
            media_labels: Vec::new(),
            scroll_calls: 0,
            click_calls: 0,
            key_presses: Vec::new(),
            typed_texts: Vec::new(),
            click_errors: VecDeque::new(),
            decorative: false,
            focused: false,
        }
    }

    #[must_use]
    pub fn with_parent(mut self, parent: impl Into<ElementId>) -> Self {
        self.parent = Some(parent.into());
        self
    }

    #[must_use]
    pub fn with_class(mut self, class_name: impl Into<String>) -> Self {
        for token in class_name.into().split_whitespace() {
            self.classes.push(token.to_owned());
        }
        self
    }

    #[must_use]
    pub fn with_attr(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }

    #[must_use]
    pub const fn with_visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    #[must_use]
    pub const fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    #[must_use]
    pub const fn with_decorative(mut self, decorative: bool) -> Self {
        self.decorative = decorative;
        self
    }

    #[must_use]
    pub fn with_box_sequence(
        mut self,
        boxes: impl IntoIterator<Item = Option<BoundingBox>>,
    ) -> Self {
        self.boxes = boxes.into_iter().collect();
        if let Some(last) = self.boxes.back() {
            self.last_box = *last;
        }
        self
    }

    #[must_use]
    pub const fn with_after_scroll_box(mut self, bbox: BoundingBox) -> Self {
        self.after_scroll_box = Some(bbox);
        self
    }

    #[must_use]
    pub fn with_media_labels(
        mut self,
        labels: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.media_labels = labels.into_iter().map(Into::into).collect();
        self
    }

    /// Return the combined class attribute string.
    pub fn class_string(&self) -> String {
        if let Some(explicit) = self.attributes.get("class") {
            return explicit.clone();
        }
        self.classes.join(" ")
    }

    fn next_box(&mut self) -> Option<BoundingBox> {
        if let Some(next) = self.boxes.pop_front() {
            self.last_box = next;
            next
        } else {
            self.last_box
        }
    }
}

/// Recorded pointer click coordinates and button.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecordedClick {
    pub x: f64,
    pub y: f64,
    pub button: MouseButton,
}

/// Shared mutable state backing [`MockUiPage`].
pub struct MockState {
    pub url: String,
    pub now_ms: u64,
    pub waits_ms: Vec<u64>,
    pub elements: BTreeMap<ElementId, MockElement>,
    pub element_order: Vec<ElementId>,
    pub css_bindings: BTreeMap<(Option<ElementId>, String), Vec<ElementId>>,
    pub xpath_bindings: BTreeMap<(Option<ElementId>, String), Vec<ElementId>>,
    pub mouse_clicks: Vec<RecordedClick>,
    pub inserted_texts: Vec<String>,
    pub key_presses: Vec<String>,
    pub chosen_files: Vec<(ElementId, Vec<PathBuf>)>,
    pub wheel_events: Vec<(f64, f64)>,
    pub events: Vec<String>,
    pub hit_override: Option<Option<PointHit>>,
    pub click_xy_error: Option<DoudouError>,
    pub capture_snapshot_error: Option<DoudouError>,
    pub capture_stop_error: Option<DoudouError>,
    pub eval_hook: Option<EvalHook>,
    pub click_hook: Option<ClickHook>,
    pub point_click_hook: Option<PointClickHook>,
    pub query_override: Option<QueryOverride>,
    pub network_captures: Vec<(CaptureFilter, Arc<Mutex<NetworkCaptureSnapshot>>)>,
    pub staged_network: Vec<(Option<CapturedRequest>, Option<CapturedResponse>)>,
    pub recorded_scripts: Vec<String>,
}

impl Default for MockState {
    fn default() -> Self {
        let mut state = Self {
            url: "https://www.douyin.com/chat".to_owned(),
            now_ms: 1_000,
            waits_ms: Vec::new(),
            elements: BTreeMap::new(),
            element_order: Vec::new(),
            css_bindings: BTreeMap::new(),
            xpath_bindings: BTreeMap::new(),
            mouse_clicks: Vec::new(),
            inserted_texts: Vec::new(),
            key_presses: Vec::new(),
            chosen_files: Vec::new(),
            wheel_events: Vec::new(),
            events: Vec::new(),
            hit_override: None,
            click_xy_error: None,
            capture_snapshot_error: None,
            capture_stop_error: None,
            eval_hook: None,
            click_hook: None,
            point_click_hook: None,
            query_override: None,
            network_captures: Vec::new(),
            staged_network: Vec::new(),
            recorded_scripts: Vec::new(),
        };
        state.add_element(MockElement::new(
            "body",
            "BODY",
            Some(BoundingBox::new(0.0, 0.0, 1280.0, 800.0)),
        ));
        state.bind_css(None, "body", ["body"]);
        state
    }
}

impl MockState {
    /// Insert or replace a mock element while preserving insertion order.
    pub fn add_element(&mut self, element: MockElement) -> ElementId {
        let id = element.id.clone();
        if !self.elements.contains_key(&id) {
            self.element_order.push(id.clone());
        }
        self.elements.insert(id.clone(), element);
        id
    }

    /// Bind a CSS selector (optionally scoped to `scope`) to an explicit list of element IDs.
    pub fn bind_css<I, S>(&mut self, scope: Option<&str>, selector: impl Into<String>, ids: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<ElementId>,
    {
        self.css_bindings.insert(
            (scope.map(ElementId::from), selector.into()),
            ids.into_iter().map(Into::into).collect(),
        );
    }

    /// Bind an `XPath` expression (optionally scoped to `scope`) to an explicit list of element IDs.
    pub fn bind_xpath<I, S>(&mut self, scope: Option<&str>, xpath: impl Into<String>, ids: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<ElementId>,
    {
        self.xpath_bindings.insert(
            (scope.map(ElementId::from), xpath.into()),
            ids.into_iter().map(Into::into).collect(),
        );
    }

    /// Build the ancestor chain `[id, parent, grandparent, ...]` for `id`.
    pub fn ancestor_chain(&self, id: &ElementId) -> Vec<ElementId> {
        let mut chain = Vec::new();
        let mut current = Some(id.clone());
        let mut guard = 0usize;
        while let Some(node_id) = current {
            if guard > 64 {
                break;
            }
            guard += 1;
            chain.push(node_id.clone());
            current = self.elements.get(&node_id).and_then(|e| e.parent.clone());
        }
        chain
    }

    /// Return `true` when `descendant` equals `ancestor` or has `ancestor` in its parent chain.
    pub fn is_descendant_or_self(&self, ancestor: &ElementId, descendant: &ElementId) -> bool {
        self.ancestor_chain(descendant)
            .iter()
            .any(|id| id == ancestor)
    }

    /// Push a request/response pair into all matching active network captures.
    pub fn emit_network(
        &self,
        request: Option<&CapturedRequest>,
        response: Option<&CapturedResponse>,
    ) {
        for (filter, sink) in &self.network_captures {
            let mut guard = sink.lock().expect("mock capture lock poisoned");
            if let Some(req) = request
                && filter.matches(&req.url)
            {
                guard.requests.push(req.clone());
            }
            if let Some(resp) = response
                && filter.matches(&resp.url)
            {
                guard.responses.push(resp.clone());
            }
        }
    }

    /// Emit any staged network traffic into active matching captures.
    pub fn flush_staged_network(&mut self) {
        let items = std::mem::take(&mut self.staged_network);
        for (req, resp) in items {
            self.emit_network(req.as_ref(), resp.as_ref());
        }
    }

    fn resolve_query(
        &mut self,
        scope: Option<&ElementId>,
        query: &SelectorQuery,
        limit: usize,
    ) -> Vec<ElementId> {
        if let Some(hook) = self.query_override.clone()
            && let Some(found) = hook(self, scope, query)
        {
            return found.into_iter().take(limit).collect();
        }
        match query {
            SelectorQuery::Css(selector) => {
                if let Some(bound) = self.css_bindings.get(&(scope.cloned(), selector.clone())) {
                    return bound.iter().take(limit).cloned().collect();
                }
                if scope.is_some()
                    && let Some(bound) = self.css_bindings.get(&(None, selector.clone()))
                {
                    let scoped: Vec<ElementId> = bound
                        .iter()
                        .filter(|id| scope.is_none_or(|s| self.is_descendant_or_self(s, id)))
                        .take(limit)
                        .cloned()
                        .collect();
                    if !scoped.is_empty() {
                        return scoped;
                    }
                }
                Vec::new()
            }
            SelectorQuery::XPath(expr) => self
                .xpath_bindings
                .get(&(scope.cloned(), expr.clone()))
                .map(|ids| ids.iter().take(limit).cloned().collect())
                .unwrap_or_default(),
            SelectorQuery::Text { text, mode } => {
                let mut matches = Vec::new();
                for id in &self.element_order {
                    if let Some(s) = scope
                        && !self.is_descendant_or_self(s, id)
                    {
                        continue;
                    }
                    let Some(elem) = self.elements.get(id) else {
                        continue;
                    };
                    let hit = match mode {
                        TextMatchMode::Exact => elem.text == *text,
                        TextMatchMode::Substring => elem.text.contains(text.as_str()),
                    };
                    if hit {
                        matches.push(id.clone());
                        if matches.len() >= limit {
                            break;
                        }
                    }
                }
                matches
            }
        }
    }

    fn hit_test_at(&self, x: f64, y: f64) -> Option<PointHit> {
        if let Some(ref override_hit) = self.hit_override {
            return override_hit.clone();
        }
        // Last inserted visible element containing (x, y) is topmost in paint order.
        for id in self.element_order.iter().rev() {
            let Some(elem) = self.elements.get(id) else {
                continue;
            };
            if !elem.visible {
                continue;
            }
            if let Some(bbox) = elem.last_box
                && bbox.contains_point(x, y)
            {
                return Some(PointHit {
                    element_id: Some(id.clone()),
                    tag: elem.tag.clone(),
                    class_name: elem.class_string(),
                    ancestor_ids: self.ancestor_chain(id),
                    non_interactive: elem.decorative,
                });
            }
        }
        None
    }
}

struct MockNetworkCaptureHandle {
    state: Arc<Mutex<MockState>>,
    sink: Arc<Mutex<NetworkCaptureSnapshot>>,
}

#[async_trait]
impl NetworkCaptureHandle for MockNetworkCaptureHandle {
    async fn snapshot(&self) -> Result<NetworkCaptureSnapshot> {
        if let Some(err) = self
            .state
            .lock()
            .expect("mock state lock poisoned")
            .capture_snapshot_error
            .clone()
        {
            return Err(err);
        }
        Ok(self
            .sink
            .lock()
            .expect("mock capture lock poisoned")
            .clone())
    }

    async fn stop(self: Box<Self>) -> Result<NetworkCaptureSnapshot> {
        if let Some(err) = self
            .state
            .lock()
            .expect("mock state lock poisoned")
            .capture_stop_error
            .clone()
        {
            return Err(err);
        }
        Ok(self
            .sink
            .lock()
            .expect("mock capture lock poisoned")
            .clone())
    }
}

/// Deterministic in-memory [`UiPage`] test double.
#[derive(Clone, Default)]
pub struct MockUiPage {
    state: Arc<Mutex<MockState>>,
}

impl MockUiPage {
    /// Create a fresh mock page initialized with a `1280x800` `<body>` element.
    pub fn new() -> Self {
        Self::default()
    }

    /// Mutate the underlying [`MockState`] under its lock.
    pub fn with_state<R>(&self, f: impl FnOnce(&mut MockState) -> R) -> R {
        let mut guard = self.state.lock().expect("mock state lock poisoned");
        f(&mut guard)
    }

    /// Register a custom handler for `eval_on_element`.
    pub fn set_eval_hook<F>(&self, f: F)
    where
        F: Fn(&ElementId, &str, Option<&Value>) -> Result<Value> + Send + Sync + 'static,
    {
        self.with_state(|s| s.eval_hook = Some(Arc::new(f)));
    }

    /// Register a custom handler invoked on `click_element`.
    pub fn set_click_hook<F>(&self, f: F)
    where
        F: Fn(&mut MockState, &ElementId, MouseButton) -> Result<()> + Send + Sync + 'static,
    {
        self.with_state(|s| s.click_hook = Some(Arc::new(f)));
    }

    /// Register a custom handler invoked on `click_xy`.
    pub fn set_point_click_hook<F>(&self, f: F)
    where
        F: Fn(&mut MockState, f64, f64, MouseButton) -> Result<()> + Send + Sync + 'static,
    {
        self.with_state(|s| s.point_click_hook = Some(Arc::new(f)));
    }

    /// Register a custom override for `query_all`.
    pub fn set_query_override<F>(&self, f: F)
    where
        F: Fn(&mut MockState, Option<&ElementId>, &SelectorQuery) -> Option<Vec<ElementId>>
            + Send
            + Sync
            + 'static,
    {
        self.with_state(|s| s.query_override = Some(Arc::new(f)));
    }
}

#[async_trait]
impl UiPage for MockUiPage {
    async fn url(&self) -> Result<String> {
        Ok(self.with_state(|s| s.url.clone()))
    }

    async fn goto(&self, url: &str) -> Result<()> {
        self.with_state(|s| {
            url.clone_into(&mut s.url);
            s.events.push(format!("goto:{url}"));
        });
        Ok(())
    }

    async fn sleep_ms(&self, ms: u64) -> Result<()> {
        self.with_state(|s| {
            s.now_ms = s.now_ms.saturating_add(ms);
            s.waits_ms.push(ms);
        });
        Ok(())
    }

    fn now_ms(&self) -> u64 {
        self.with_state(|s| s.now_ms)
    }

    async fn query_all(
        &self,
        scope: Option<&ElementId>,
        query: &SelectorQuery,
        limit: usize,
    ) -> Result<Vec<ElementId>> {
        Ok(self.with_state(|s| s.resolve_query(scope, query, limit)))
    }

    async fn is_visible(&self, element: &ElementId) -> Result<bool> {
        Ok(self.with_state(|s| s.elements.get(element).is_some_and(|e| e.visible)))
    }

    async fn is_enabled(&self, element: &ElementId) -> Result<bool> {
        Ok(self.with_state(|s| s.elements.get(element).is_some_and(|e| e.enabled)))
    }

    async fn is_decorative(&self, element: &ElementId) -> Result<bool> {
        let explicitly_decorative =
            self.with_state(|s| s.elements.get(element).is_some_and(|e| e.decorative));
        if explicitly_decorative {
            return Ok(true);
        }
        let aria_hidden = self.get_attribute(element, "aria-hidden").await?;
        if aria_hidden.as_deref() == Some("true") {
            return Ok(true);
        }
        let role = self.get_attribute(element, "role").await?;
        Ok(matches!(role.as_deref(), Some("presentation" | "none")))
    }

    async fn bounding_box(&self, element: &ElementId) -> Result<Option<BoundingBox>> {
        Ok(self.with_state(|s| {
            s.events.push(format!("box:{}", element.as_str()));
            s.elements.get_mut(element).and_then(MockElement::next_box)
        }))
    }

    async fn scroll_into_view(&self, element: &ElementId, _timeout_ms: u64) -> Result<()> {
        self.with_state(|s| {
            s.events.push(format!("scroll:{}", element.as_str()));
            if let Some(elem) = s.elements.get_mut(element) {
                elem.scroll_calls += 1;
                if let Some(after) = elem.after_scroll_box {
                    elem.last_box = Some(after);
                }
            }
        });
        Ok(())
    }

    async fn inner_text(&self, element: &ElementId, _timeout_ms: u64) -> Result<String> {
        Ok(self.with_state(|s| {
            s.elements
                .get(element)
                .map(|e| e.text.clone())
                .unwrap_or_default()
        }))
    }

    async fn text_content(&self, element: &ElementId, _timeout_ms: u64) -> Result<Option<String>> {
        Ok(self.with_state(|s| s.elements.get(element).map(|e| e.text.clone())))
    }

    async fn get_attribute(&self, element: &ElementId, name: &str) -> Result<Option<String>> {
        Ok(self.with_state(|s| {
            let elem = s.elements.get(element)?;
            if name == "class" {
                return Some(elem.class_string());
            }
            elem.attributes.get(name).cloned()
        }))
    }

    async fn eval_json(&self, script: &str, _arg: Option<&Value>) -> Result<Value> {
        self.with_state(|s| s.recorded_scripts.push(script.to_owned()));
        Ok(Value::Null)
    }

    async fn eval_on_element(
        &self,
        element: &ElementId,
        script: &str,
        arg: Option<&Value>,
    ) -> Result<Value> {
        let hook = self.with_state(|s| {
            s.recorded_scripts.push(script.to_owned());
            s.eval_hook.clone()
        });
        if let Some(hook) = hook {
            return hook(element, script, arg);
        }
        Ok(self.with_state(|s| {
            let Some(elem) = s.elements.get(element) else {
                return Value::Null;
            };
            if script.contains("document.activeElement") {
                return json!(elem.focused);
            }
            if script.contains("media_labels") {
                return json!({
                    "text": elem.text,
                    "media_labels": elem.media_labels,
                });
            }
            if script.contains("'value' in element") {
                return json!(elem.text);
            }
            if script.contains("element.tagName") {
                return json!(elem.tag);
            }
            Value::Null
        }))
    }

    async fn element_from_point(&self, x: f64, y: f64) -> Result<Option<PointHit>> {
        Ok(self.with_state(|s| s.hit_test_at(x, y)))
    }

    async fn click_xy(&self, x: f64, y: f64, button: MouseButton, _timeout_ms: u64) -> Result<()> {
        let (err, hook) = self.with_state(|s| {
            s.mouse_clicks.push(RecordedClick { x, y, button });
            (s.click_xy_error.clone(), s.point_click_hook.clone())
        });
        if let Some(error) = err {
            return Err(error);
        }
        if let Some(hook) = hook {
            return self.with_state(|s| hook(s, x, y, button));
        }
        self.with_state(|s| {
            if let Some(hit) = s.hit_test_at(x, y)
                && let Some(id) = hit.element_id
                && let Some(elem) = s.elements.get_mut(&id)
            {
                elem.click_calls += 1;
                elem.focused = true;
            }
        });
        Ok(())
    }

    async fn click_element(
        &self,
        element: &ElementId,
        button: MouseButton,
        _timeout_ms: u64,
    ) -> Result<()> {
        let (queued_err, hook) = self.with_state(|s| {
            let queued = s
                .elements
                .get_mut(element)
                .and_then(|e| e.click_errors.pop_front());
            if queued.is_none()
                && let Some(elem) = s.elements.get_mut(element)
            {
                elem.click_calls += 1;
            }
            (queued, s.click_hook.clone())
        });
        if let Some(error) = queued_err {
            return Err(error);
        }
        if let Some(hook) = hook {
            return self.with_state(|s| hook(s, element, button));
        }
        Ok(())
    }

    async fn insert_text(&self, text: &str) -> Result<()> {
        self.with_state(|s| {
            s.inserted_texts.push(text.to_owned());
            for elem in s.elements.values_mut() {
                if elem.focused {
                    elem.text.push_str(text);
                }
            }
        });
        Ok(())
    }

    async fn type_text_sequentially(
        &self,
        element: &ElementId,
        text: &str,
        _delay_ms: u64,
        _timeout_ms: u64,
    ) -> Result<()> {
        self.with_state(|s| {
            if let Some(elem) = s.elements.get_mut(element) {
                elem.typed_texts.push(text.to_owned());
                elem.text.push_str(text);
            }
        });
        Ok(())
    }

    async fn press_key(&self, key: &str, _timeout_ms: u64) -> Result<()> {
        self.with_state(|s| s.key_presses.push(key.to_owned()));
        Ok(())
    }

    async fn press_element_key(
        &self,
        element: &ElementId,
        key: &str,
        _timeout_ms: u64,
    ) -> Result<()> {
        self.with_state(|s| {
            if let Some(elem) = s.elements.get_mut(element) {
                elem.key_presses.push(key.to_owned());
                if key == "Backspace" {
                    elem.text.clear();
                }
            }
            if key == "Enter" {
                s.flush_staged_network();
            }
        });
        Ok(())
    }

    async fn set_files(&self, element: &ElementId, paths: &[&Path]) -> Result<()> {
        self.with_state(|s| {
            s.chosen_files.push((
                element.clone(),
                paths.iter().map(|p| p.to_path_buf()).collect(),
            ));
        });
        Ok(())
    }

    async fn choose_files_via_click(
        &self,
        trigger: &ElementId,
        paths: &[&Path],
        _timeout_ms: u64,
    ) -> Result<()> {
        self.with_state(|s| {
            if let Some(elem) = s.elements.get_mut(trigger) {
                elem.click_calls += 1;
            }
            s.chosen_files.push((
                trigger.clone(),
                paths.iter().map(|p| p.to_path_buf()).collect(),
            ));
        });
        Ok(())
    }

    async fn wheel(&self, delta_x: f64, delta_y: f64) -> Result<()> {
        self.with_state(|s| s.wheel_events.push((delta_x, delta_y)));
        Ok(())
    }

    async fn start_network_capture(
        &self,
        filter: CaptureFilter,
    ) -> Result<Box<dyn NetworkCaptureHandle>> {
        let sink = Arc::new(Mutex::new(NetworkCaptureSnapshot::default()));
        self.with_state(|s| s.network_captures.push((filter, Arc::clone(&sink))));
        Ok(Box::new(MockNetworkCaptureHandle {
            state: Arc::clone(&self.state),
            sink,
        }))
    }
}

/// Deterministic in-memory [`UiSession`] wrapping [`MockUiPage`].
pub struct MockUiSession {
    pub page: MockUiPage,
    pub verification_override: Mutex<Option<Option<String>>>,
}

impl MockUiSession {
    /// Wrap a [`MockUiPage`] in a ready [`MockUiSession`].
    pub fn new(page: MockUiPage) -> Self {
        Self {
            page,
            verification_override: Mutex::new(None),
        }
    }

    /// Force `verification_kind()` to return a specific value instead of running DOM detection.
    pub fn set_verification_kind(&self, kind: Option<String>) {
        *self
            .verification_override
            .lock()
            .expect("mock verification lock poisoned") = Some(kind);
    }
}

#[async_trait]
impl UiSession for MockUiSession {
    fn page(&self) -> &dyn UiPage {
        &self.page
    }

    async fn ensure_ready(&self) -> Result<()> {
        Ok(())
    }

    async fn open(&self, url: &str) -> Result<()> {
        self.page.goto(url).await
    }

    async fn verification_kind(&self) -> Result<Option<String>> {
        if let Some(overridden) = self
            .verification_override
            .lock()
            .expect("mock verification lock poisoned")
            .clone()
        {
            return Ok(overridden);
        }
        crate::components::AuthChallengeComponent::new(&self.page)
            .detect()
            .await
    }
}
