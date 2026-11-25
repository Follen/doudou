//! Bridge implementing [`doudou_ui::UiPage`] and [`doudou_ui::UiSession`] over [`doudou_cdp`].

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use doudou_cdp::{
    AuthPolicy, CapturedNetworkExchange, CdpPage, CdpPageOps, ChromeSession, EvalWorld,
    KeyEventSpec, KeyModifiers, LoadWait, MouseButton as CdpMouseButton, NetworkCaptureBuffer,
};
use doudou_core::{DoudouError, Result};
use doudou_ui::{
    BoundingBox, CaptureFilter, CapturedRequest, CapturedResponse, ElementId,
    MouseButton as UiMouseButton, NetworkCaptureHandle, NetworkCaptureSnapshot, PointHit,
    SelectorQuery, TextMatchMode, UiPage, UiSession,
};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::Mutex as AsyncMutex;

/// Text filter extracted from Playwright-style pseudo-selectors (`:has-text`, `:text`, `:text-is`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum CssTextFilter {
    /// Case-insensitive substring match (`:has-text(...)`).
    HasText(String),
    /// Case-insensitive substring match (`:text(...)`).
    Text(String),
    /// Trimmed exact text match (`:text-is(...)`).
    TextIs(String),
}

/// One top-level comma-separated CSS selector clause with Playwright pseudo-classes stripped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CssSelectorClause {
    /// Standard CSS selector passed to `querySelectorAll` (`"*"` when empty).
    pub base_css: String,
    /// Ordered text filters (`:has-text`, `:text`, `:text-is`) applied to matched elements.
    pub text_filters: Vec<CssTextFilter>,
    /// Whether `:visible` was present on this clause.
    pub require_visible: bool,
}

fn split_top_level_commas(selector: &str) -> Vec<&str> {
    let mut clauses = Vec::new();
    let mut start = 0usize;
    let mut in_quote: Option<char> = None;
    let mut escaped = false;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;

    for (idx, ch) in selector.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if let Some(q) = in_quote {
            if ch == '\\' {
                escaped = true;
            } else if ch == q {
                in_quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => in_quote = Some(ch),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            ',' if paren_depth == 0 && bracket_depth == 0 => {
                let part = selector[start..idx].trim();
                if !part.is_empty() {
                    clauses.push(part);
                }
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }
    let tail = selector[start..].trim();
    if !tail.is_empty() {
        clauses.push(tail);
    }
    clauses
}

fn consume_paren_arg(after_open: &str) -> (&str, usize) {
    let mut in_quote: Option<char> = None;
    let mut escaped = false;
    let mut depth = 0usize;

    for (idx, ch) in after_open.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if let Some(q) = in_quote {
            if ch == '\\' {
                escaped = true;
            } else if ch == q {
                in_quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => in_quote = Some(ch),
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    return (&after_open[..idx], idx + ch.len_utf8());
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    (after_open, after_open.len())
}

fn unquote_pseudo_arg(raw: &str) -> String {
    let trimmed = raw.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        let inner = &trimmed[1..trimmed.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(ch) = chars.next() {
            if ch == '\\'
                && let Some(next) = chars.next()
            {
                match next {
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    other => out.push(other),
                }
                continue;
            }
            out.push(ch);
        }
        return out;
    }
    trimmed.to_owned()
}

fn starts_with_visible_pseudo(s: &str) -> bool {
    let Some(after) = s.strip_prefix(":visible") else {
        return false;
    };
    !after
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

fn needs_universal_placeholder(prefix: &str, suffix: &str) -> bool {
    let after_combinator = !prefix.is_empty()
        && prefix.ends_with(|c: char| c.is_whitespace() || matches!(c, '>' | '+' | '~'));
    if !after_combinator {
        return false;
    }
    let next_is_valid_simple = suffix
        .starts_with(|c: char| c.is_ascii_alphabetic() || matches!(c, '.' | '#' | '[' | '*'))
        || (suffix.starts_with(':')
            && !suffix.starts_with(":has-text(")
            && !suffix.starts_with(":text-is(")
            && !suffix.starts_with(":text(")
            && !starts_with_visible_pseudo(suffix));
    !next_is_valid_simple
}

fn parse_single_css_clause(clause: &str) -> CssSelectorClause {
    let mut base_css = String::with_capacity(clause.len());
    let mut text_filters = Vec::new();
    let mut require_visible = false;

    let mut i = 0usize;
    let mut in_quote: Option<char> = None;
    let mut escaped = false;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;

    while i < clause.len() {
        let rest = &clause[i..];
        if in_quote.is_none() && paren_depth == 0 && bracket_depth == 0 {
            if let Some(after) = rest.strip_prefix(":has-text(") {
                let (arg, consumed) = consume_paren_arg(after);
                text_filters.push(CssTextFilter::HasText(unquote_pseudo_arg(arg)));
                i += ":has-text(".len() + consumed;
                if needs_universal_placeholder(&base_css, &clause[i..]) {
                    base_css.push('*');
                }
                continue;
            }
            if let Some(after) = rest.strip_prefix(":text-is(") {
                let (arg, consumed) = consume_paren_arg(after);
                text_filters.push(CssTextFilter::TextIs(unquote_pseudo_arg(arg)));
                i += ":text-is(".len() + consumed;
                if needs_universal_placeholder(&base_css, &clause[i..]) {
                    base_css.push('*');
                }
                continue;
            }
            if let Some(after) = rest.strip_prefix(":text(") {
                let (arg, consumed) = consume_paren_arg(after);
                text_filters.push(CssTextFilter::Text(unquote_pseudo_arg(arg)));
                i += ":text(".len() + consumed;
                if needs_universal_placeholder(&base_css, &clause[i..]) {
                    base_css.push('*');
                }
                continue;
            }
            if starts_with_visible_pseudo(rest) {
                require_visible = true;
                i += ":visible".len();
                if needs_universal_placeholder(&base_css, &clause[i..]) {
                    base_css.push('*');
                }
                continue;
            }
        }

        let Some(ch) = rest.chars().next() else {
            break;
        };
        if escaped {
            escaped = false;
        } else if let Some(q) = in_quote {
            if ch == '\\' {
                escaped = true;
            } else if ch == q {
                in_quote = None;
            }
        } else {
            match ch {
                '\'' | '"' => in_quote = Some(ch),
                '(' => paren_depth += 1,
                ')' => paren_depth = paren_depth.saturating_sub(1),
                '[' => bracket_depth += 1,
                ']' => bracket_depth = bracket_depth.saturating_sub(1),
                _ => {}
            }
        }
        base_css.push(ch);
        i += ch.len_utf8();
    }

    let trimmed = base_css.trim();
    let final_base = if trimmed.is_empty() {
        "*".to_owned()
    } else {
        trimmed.to_owned()
    };

    CssSelectorClause {
        base_css: final_base,
        text_filters,
        require_visible,
    }
}

/// Split a CSS selector string (supporting Playwright `:has-text`, `:text`, `:text-is`, `:visible`
/// pseudo-classes and comma-separated lists) into structured clauses for browser evaluation.
#[must_use]
pub fn parse_css_clauses(selector: &str) -> Vec<CssSelectorClause> {
    split_top_level_commas(selector)
        .into_iter()
        .map(parse_single_css_clause)
        .collect()
}

/// JS bootstrap that maintains an isolated-world `Map<string, Element>` handle table.
const REGISTRY_PRELUDE: &str = r"
const __dd = (() => {
  if (!globalThis.__doudouRegistry) {
    globalThis.__doudouRegistry = { next: 1, byId: new Map(), byEl: new WeakMap() };
  }
  const reg = globalThis.__doudouRegistry;
  return {
    idOf(el) {
      if (!el || el.nodeType !== 1) return null;
      let id = reg.byEl.get(el);
      if (!id) {
        id = 'el-' + (reg.next++);
        reg.byEl.set(el, id);
        reg.byId.set(id, el);
      }
      return id;
    },
    get(id) {
      if (!id) return null;
      const el = reg.byId.get(id) || null;
      if (el && !el.isConnected) return null;
      return el;
    },
  };
})();
";

fn map_mouse_button(button: UiMouseButton) -> CdpMouseButton {
    match button {
        UiMouseButton::Left => CdpMouseButton::Left,
        UiMouseButton::Right => CdpMouseButton::Right,
    }
}

/// Parse a Playwright-style shortcut string (e.g. `"Enter"`, `"Control+A"`) into a [`KeyEventSpec`].
pub fn parse_key_spec(shortcut: &str) -> KeyEventSpec {
    let mut modifiers = KeyModifiers::default();
    let mut parts: Vec<&str> = shortcut.split('+').collect();
    let base_key = parts.pop().unwrap_or(shortcut).trim();
    for modifier in parts {
        match modifier.trim().to_ascii_lowercase().as_str() {
            "control" | "ctrl" => modifiers.ctrl = true,
            "alt" | "option" => modifiers.alt = true,
            "meta" | "command" | "cmd" | "super" => modifiers.meta = true,
            "shift" => modifiers.shift = true,
            _ => {}
        }
    }

    let mut spec = KeyEventSpec::named(base_key);
    if base_key.len() == 1 {
        let ch = base_key.chars().next().unwrap_or(' ');
        if ch.is_ascii_alphabetic() {
            let upper = ch.to_ascii_uppercase();
            spec.code = Some(format!("Key{upper}"));
            spec.windows_virtual_key_code = Some(i32::from(upper as u8));
        } else if ch.is_ascii_digit() {
            spec.code = Some(format!("Digit{ch}"));
            spec.windows_virtual_key_code = Some(i32::from(ch as u8));
        }
        if !modifiers.ctrl && !modifiers.alt && !modifiers.meta {
            spec.text = Some(base_key.to_owned());
        }
    }
    spec.modifiers = modifiers;
    spec
}

/// Active CDP network capture handle scoped to a [`CaptureFilter`] and starting offset.
pub struct CdpNetworkCaptureHandle {
    buffer: NetworkCaptureBuffer,
    filter: CaptureFilter,
    start_index: usize,
}

impl CdpNetworkCaptureHandle {
    fn collect_snapshot(&self) -> NetworkCaptureSnapshot {
        let all = self.buffer.snapshot();
        let slice = all.get(self.start_index..).unwrap_or(&[]);
        exchanges_to_snapshot(slice, &self.filter)
    }
}

fn exchanges_to_snapshot(
    exchanges: &[CapturedNetworkExchange],
    filter: &CaptureFilter,
) -> NetworkCaptureSnapshot {
    let mut requests = Vec::new();
    let mut responses = Vec::new();
    for exchange in exchanges {
        if !filter.matches(&exchange.url) || exchange.method.eq_ignore_ascii_case("OPTIONS") {
            continue;
        }
        requests.push(CapturedRequest::new(
            exchange.method.clone(),
            exchange.url.clone(),
            exchange.request_body.clone().unwrap_or_default(),
        ));
        if let (Some(status), Some(body)) = (exchange.status, exchange.response_body.as_ref()) {
            responses.push(CapturedResponse::new(
                exchange.url.clone(),
                status,
                body.clone(),
            ));
        }
    }
    NetworkCaptureSnapshot {
        requests,
        responses,
    }
}

#[async_trait]
impl NetworkCaptureHandle for CdpNetworkCaptureHandle {
    async fn snapshot(&self) -> Result<NetworkCaptureSnapshot> {
        Ok(self.collect_snapshot())
    }

    async fn stop(self: Box<Self>) -> Result<NetworkCaptureSnapshot> {
        Ok(self.collect_snapshot())
    }
}

/// Adapter exposing any [`CdpPageOps`] (including live [`CdpPage`] and [`doudou_cdp::FakeCdpPage`])
/// as a [`doudou_ui::UiPage`].
#[derive(Clone)]
pub struct CdpUiPage {
    page: Arc<dyn CdpPageOps>,
    last_pointer: Arc<Mutex<(f64, f64)>>,
}

impl std::fmt::Debug for CdpUiPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CdpUiPage").finish_non_exhaustive()
    }
}

impl CdpUiPage {
    /// Wrap any [`CdpPageOps`] implementation.
    pub fn new(page: Arc<dyn CdpPageOps>) -> Self {
        Self {
            page,
            last_pointer: Arc::new(Mutex::new((640.0, 450.0))),
        }
    }

    /// Wrap a concrete [`CdpPage`].
    pub fn from_cdp_page(page: Arc<CdpPage>) -> Self {
        Self::new(page)
    }

    /// Borrow the underlying [`CdpPageOps`] handle.
    pub fn inner(&self) -> &Arc<dyn CdpPageOps> {
        &self.page
    }

    fn set_pointer(&self, x: f64, y: f64) {
        let mut guard = self.last_pointer.lock().expect("pointer lock poisoned");
        *guard = (x, y);
    }

    fn pointer(&self) -> (f64, f64) {
        *self.last_pointer.lock().expect("pointer lock poisoned")
    }

    async fn eval_isolated(&self, script: &str) -> Result<Value> {
        self.page.eval_json(script, EvalWorld::Isolated).await
    }

    async fn focus_element(&self, element: &ElementId) -> Result<()> {
        let id_json = serde_json::to_string(element.as_str()).unwrap_or_else(|_| "\"\"".to_owned());
        let script = format!(
            "(() => {{ {REGISTRY_PRELUDE} const el = __dd.get({id_json}); if (!el) return false; if (typeof el.focus === 'function') el.focus(); return true; }})()"
        );
        let _ = self.eval_isolated(&script).await?;
        Ok(())
    }
}

#[async_trait]
impl UiPage for CdpUiPage {
    async fn url(&self) -> Result<String> {
        self.page.current_url().await
    }

    async fn goto(&self, url: &str) -> Result<()> {
        let _ = self
            .page
            .goto(url, LoadWait::DomContentLoaded, Duration::from_secs(60))
            .await?;
        Ok(())
    }

    async fn sleep_ms(&self, ms: u64) -> Result<()> {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        Ok(())
    }

    async fn query_all(
        &self,
        scope: Option<&ElementId>,
        query: &SelectorQuery,
        limit: usize,
    ) -> Result<Vec<ElementId>> {
        let scope_json = match scope {
            Some(id) => serde_json::to_string(id.as_str()).unwrap_or_else(|_| "null".to_owned()),
            None => "null".to_owned(),
        };
        let (kind, pattern, exact, clauses_json) = match query {
            SelectorQuery::Css(sel) => {
                let clauses = parse_css_clauses(sel);
                let json = serde_json::to_string(&clauses).unwrap_or_else(|_| "[]".to_owned());
                ("css", sel.as_str(), false, json)
            }
            SelectorQuery::XPath(expr) => ("xpath", expr.as_str(), false, "[]".to_owned()),
            SelectorQuery::Text { text, mode } => (
                "text",
                text.as_str(),
                *mode == TextMatchMode::Exact,
                "[]".to_owned(),
            ),
        };
        let pattern_json = serde_json::to_string(pattern).unwrap_or_else(|_| "\"\"".to_owned());
        let script = format!(
            r#"(() => {{
  {REGISTRY_PRELUDE}
  const scopeId = {scope_json};
  const root = scopeId ? __dd.get(scopeId) : document;
  if (!root) return [];
  const kind = "{kind}";
  const pattern = {pattern_json};
  const exact = {exact};
  const limit = {limit};
  const out = [];
  const push = (el) => {{
    if (out.length >= limit) return;
    const id = __dd.idOf(el);
    if (id && !out.includes(id)) out.push(id);
  }};
  const norm = (s) => (s || '').replace(/\s+/g, ' ').trim();
  const isVisible = (el) => {{
    if (!el || el.nodeType !== 1) return false;
    const style = getComputedStyle(el);
    if (style.display === 'none' || style.visibility === 'hidden' || parseFloat(style.opacity || '1') === 0) return false;
    const rect = el.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  }};
  if (kind === 'css') {{
    const clauses = {clauses_json};
    for (const clause of clauses) {{
      let list;
      try {{
        list = root.querySelectorAll(clause.base_css || '*');
      }} catch (_) {{
        continue;
      }}
      for (const el of list) {{
        if (clause.require_visible && !isVisible(el)) continue;
        if (clause.text_filters && clause.text_filters.length > 0) {{
          const txt = norm(el.innerText || el.textContent || '');
          const txtLower = txt.toLowerCase();
          let ok = true;
          for (const f of clause.text_filters) {{
            const want = norm(f.value || '');
            if (f.kind === 'text_is') {{
              if (txt !== want) {{ ok = false; break; }}
            }} else {{
              if (!txtLower.includes(want.toLowerCase())) {{ ok = false; break; }}
            }}
          }}
          if (!ok) continue;
        }}
        push(el);
        if (out.length >= limit) break;
      }}
      if (out.length >= limit) break;
    }}
  }} else if (kind === 'xpath') {{
    const doc = root.ownerDocument || document;
    const snapshot = doc.evaluate(pattern, root, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null);
    for (let i = 0; i < snapshot.snapshotLength && out.length < limit; i += 1) {{
      push(snapshot.snapshotItem(i));
    }}
  }} else if (kind === 'text') {{
    const target = norm(pattern);
    const elements = root.nodeType === 1 ? [root, ...root.querySelectorAll('*')] : [...document.querySelectorAll('*')];
    for (const el of elements) {{
      const txt = norm(el.innerText || el.textContent || '');
      const matched = exact ? txt === target : (target.length > 0 && txt.includes(target));
      if (!matched) continue;
      const childMatched = [...el.children].some(ch => {{
        const ctxt = norm(ch.innerText || ch.textContent || '');
        return exact ? ctxt === target : (target.length > 0 && ctxt.includes(target));
      }});
      if (!childMatched) {{
        push(el);
        if (out.length >= limit) break;
      }}
    }}
  }}
  return out;
}})()"#
        );

        let value = self.eval_isolated(&script).await?;
        let ids = value
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ElementId::new)
                    .collect()
            })
            .unwrap_or_default();
        Ok(ids)
    }

    async fn is_visible(&self, element: &ElementId) -> Result<bool> {
        let id_json = serde_json::to_string(element.as_str()).unwrap_or_else(|_| "\"\"".to_owned());
        let script = format!(
            r"(() => {{
  {REGISTRY_PRELUDE}
  const el = __dd.get({id_json});
  if (!el) return false;
  const style = getComputedStyle(el);
  if (style.display === 'none' || style.visibility === 'hidden' || parseFloat(style.opacity || '1') === 0) return false;
  const rect = el.getBoundingClientRect();
  return rect.width > 0 && rect.height > 0;
}})()"
        );
        Ok(self
            .eval_isolated(&script)
            .await?
            .as_bool()
            .unwrap_or(false))
    }

    async fn is_enabled(&self, element: &ElementId) -> Result<bool> {
        let id_json = serde_json::to_string(element.as_str()).unwrap_or_else(|_| "\"\"".to_owned());
        let script = format!(
            r"(() => {{
  {REGISTRY_PRELUDE}
  const el = __dd.get({id_json});
  if (!el) return false;
  if (el.disabled === true) return false;
  if (el.getAttribute('aria-disabled') === 'true') return false;
  return true;
}})()"
        );
        Ok(self
            .eval_isolated(&script)
            .await?
            .as_bool()
            .unwrap_or(false))
    }

    async fn is_decorative(&self, element: &ElementId) -> Result<bool> {
        let id_json = serde_json::to_string(element.as_str()).unwrap_or_else(|_| "\"\"".to_owned());
        let script = format!(
            r"(() => {{
  {REGISTRY_PRELUDE}
  const el = __dd.get({id_json});
  if (!el) return false;
  if (el.getAttribute('aria-hidden') === 'true') return true;
  const role = el.getAttribute('role');
  if (role === 'presentation' || role === 'none') return true;
  const style = getComputedStyle(el);
  return style.pointerEvents === 'none';
}})()"
        );
        Ok(self
            .eval_isolated(&script)
            .await?
            .as_bool()
            .unwrap_or(false))
    }

    async fn bounding_box(&self, element: &ElementId) -> Result<Option<BoundingBox>> {
        let id_json = serde_json::to_string(element.as_str()).unwrap_or_else(|_| "\"\"".to_owned());
        let script = format!(
            r"(() => {{
  {REGISTRY_PRELUDE}
  const el = __dd.get({id_json});
  if (!el) return null;
  const r = el.getBoundingClientRect();
  return {{ x: r.x, y: r.y, width: r.width, height: r.height }};
}})()"
        );
        let value = self.eval_isolated(&script).await?;
        if value.is_null() {
            return Ok(None);
        }
        let x = value.get("x").and_then(Value::as_f64);
        let y = value.get("y").and_then(Value::as_f64);
        let width = value.get("width").and_then(Value::as_f64);
        let height = value.get("height").and_then(Value::as_f64);
        match (x, y, width, height) {
            (Some(x), Some(y), Some(width), Some(height)) => {
                Ok(Some(BoundingBox::new(x, y, width, height)))
            }
            _ => Ok(None),
        }
    }

    async fn scroll_into_view(&self, element: &ElementId, _timeout_ms: u64) -> Result<()> {
        let id_json = serde_json::to_string(element.as_str()).unwrap_or_else(|_| "\"\"".to_owned());
        let script = format!(
            r"(() => {{
  {REGISTRY_PRELUDE}
  const el = __dd.get({id_json});
  if (el && typeof el.scrollIntoView === 'function') {{
    el.scrollIntoView({{ block: 'center', inline: 'nearest' }});
  }}
  return true;
}})()"
        );
        let _ = self.eval_isolated(&script).await?;
        Ok(())
    }

    async fn inner_text(&self, element: &ElementId, _timeout_ms: u64) -> Result<String> {
        let id_json = serde_json::to_string(element.as_str()).unwrap_or_else(|_| "\"\"".to_owned());
        let script = format!(
            r"(() => {{
  {REGISTRY_PRELUDE}
  const el = __dd.get({id_json});
  return el ? (el.innerText ?? el.textContent ?? '') : '';
}})()"
        );
        Ok(self
            .eval_isolated(&script)
            .await?
            .as_str()
            .unwrap_or_default()
            .to_owned())
    }

    async fn text_content(&self, element: &ElementId, _timeout_ms: u64) -> Result<Option<String>> {
        let id_json = serde_json::to_string(element.as_str()).unwrap_or_else(|_| "\"\"".to_owned());
        let script = format!(
            r"(() => {{
  {REGISTRY_PRELUDE}
  const el = __dd.get({id_json});
  return el ? el.textContent : null;
}})()"
        );
        let val = self.eval_isolated(&script).await?;
        Ok(val.as_str().map(ToOwned::to_owned))
    }

    async fn get_attribute(&self, element: &ElementId, name: &str) -> Result<Option<String>> {
        let id_json = serde_json::to_string(element.as_str()).unwrap_or_else(|_| "\"\"".to_owned());
        let name_json = serde_json::to_string(name).unwrap_or_else(|_| "\"\"".to_owned());
        let script = format!(
            r"(() => {{
  {REGISTRY_PRELUDE}
  const el = __dd.get({id_json});
  return el ? el.getAttribute({name_json}) : null;
}})()"
        );
        let val = self.eval_isolated(&script).await?;
        Ok(val.as_str().map(ToOwned::to_owned))
    }

    async fn eval_json(&self, script: &str, arg: Option<&Value>) -> Result<Value> {
        let expr = match arg {
            Some(a) => {
                let arg_json = serde_json::to_string(a).unwrap_or_else(|_| "null".to_owned());
                format!("(({script})({arg_json}))")
            }
            None => format!("(({script})())"),
        };
        self.page.eval_json(&expr, EvalWorld::Main).await
    }

    async fn eval_on_element(
        &self,
        element: &ElementId,
        script: &str,
        arg: Option<&Value>,
    ) -> Result<Value> {
        let id_json = serde_json::to_string(element.as_str()).unwrap_or_else(|_| "\"\"".to_owned());
        let arg_json = arg
            .and_then(|a| serde_json::to_string(a).ok())
            .unwrap_or_else(|| "undefined".to_owned());
        let expr = format!(
            r"(() => {{
  {REGISTRY_PRELUDE}
  const el = __dd.get({id_json});
  if (!el) return null;
  const fn = ({script});
  return fn(el, {arg_json});
}})()"
        );
        self.eval_isolated(&expr).await
    }

    async fn element_from_point(&self, x: f64, y: f64) -> Result<Option<PointHit>> {
        let script = format!(
            r"(() => {{
  {REGISTRY_PRELUDE}
  const hit = document.elementFromPoint({x}, {y});
  if (!hit) return null;
  const ancestors = [];
  for (let cur = hit; cur && cur.nodeType === 1; cur = cur.parentElement) {{
    const id = __dd.idOf(cur);
    if (id) ancestors.push(id);
  }}
  const style = getComputedStyle(hit);
  const nonInteractive = hit.getAttribute('aria-hidden') === 'true'
    || hit.getAttribute('role') === 'presentation'
    || hit.getAttribute('role') === 'none'
    || style.pointerEvents === 'none';
  return {{
    element_id: __dd.idOf(hit),
    tag: String(hit.tagName || '').toUpperCase(),
    class_name: String(typeof hit.className === 'string' ? hit.className : (hit.getAttribute('class') || '')).slice(0, 160),
    ancestor_ids: ancestors,
    non_interactive: nonInteractive,
  }};
}})()"
        );
        let value = self.eval_isolated(&script).await?;
        if value.is_null() {
            return Ok(None);
        }
        let hit: PointHit = serde_json::from_value(value).map_err(|err| {
            DoudouError::new(
                "CDP_EVAL_ERROR",
                format!("Failed decoding elementFromPoint hit: {err}"),
            )
        })?;
        Ok(Some(hit))
    }

    async fn click_xy(&self, x: f64, y: f64, button: UiMouseButton, timeout_ms: u64) -> Result<()> {
        self.set_pointer(x, y);
        let timeout = Duration::from_millis(timeout_ms.max(100));
        match tokio::time::timeout(
            timeout,
            self.page.click_xy(x, y, map_mouse_button(button), 1),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(DoudouError::new(
                "CLICK_TIMEOUT",
                "Timed out dispatching pointer click.",
            )
            .with_details(json!({ "x": x, "y": y, "timeout_ms": timeout_ms }))),
        }
    }

    async fn click_element(
        &self,
        element: &ElementId,
        button: UiMouseButton,
        timeout_ms: u64,
    ) -> Result<()> {
        let Some(bbox) = self.bounding_box(element).await? else {
            return Err(DoudouError::new(
                "PAGE_ADAPTER_DRIFTED",
                "Target element has no bounding box for click.",
            ));
        };
        let (cx, cy) = bbox.center();
        self.click_xy(cx, cy, button, timeout_ms).await
    }

    async fn insert_text(&self, text: &str) -> Result<()> {
        self.page.insert_text(text).await
    }

    async fn type_text_sequentially(
        &self,
        element: &ElementId,
        text: &str,
        delay_ms: u64,
        _timeout_ms: u64,
    ) -> Result<()> {
        self.focus_element(element).await?;
        for ch in text.chars() {
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            self.page.insert_text(s).await?;
            if delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
        }
        Ok(())
    }

    async fn press_key(&self, key: &str, timeout_ms: u64) -> Result<()> {
        let spec = parse_key_spec(key);
        let timeout = Duration::from_millis(timeout_ms.max(100));
        match tokio::time::timeout(timeout, self.page.dispatch_key(&spec)).await {
            Ok(res) => res,
            Err(_) => Err(DoudouError::new(
                "CDP_COMMAND_TIMEOUT",
                format!("Timed out pressing key '{key}'."),
            )),
        }
    }

    async fn press_element_key(
        &self,
        element: &ElementId,
        key: &str,
        timeout_ms: u64,
    ) -> Result<()> {
        self.focus_element(element).await?;
        self.press_key(key, timeout_ms).await
    }

    async fn set_files(&self, element: &ElementId, paths: &[&Path]) -> Result<()> {
        let id_json = serde_json::to_string(element.as_str()).unwrap_or_else(|_| "\"\"".to_owned());
        let marker = format!("dd-upload-{}", element.as_str());
        let marker_json = serde_json::to_string(&marker).unwrap_or_else(|_| "\"\"".to_owned());
        let script = format!(
            r"(() => {{
  {REGISTRY_PRELUDE}
  const el = __dd.get({id_json});
  if (!el) return false;
  el.setAttribute('data-doudou-upload', {marker_json});
  return true;
}})()"
        );
        let tagged = self
            .eval_isolated(&script)
            .await?
            .as_bool()
            .unwrap_or(false);
        if !tagged {
            return Err(DoudouError::new(
                "PAGE_ADAPTER_DRIFTED",
                "Target file input element is no longer connected.",
            ));
        }
        let owned_paths: Vec<PathBuf> = paths.iter().map(|p| p.to_path_buf()).collect();
        let selector = format!(r#"[data-doudou-upload="{marker}"]"#);
        let result = self
            .page
            .set_file_input_files(&selector, &owned_paths)
            .await;
        let cleanup = format!(
            r"(() => {{ {REGISTRY_PRELUDE} const el = __dd.get({id_json}); if (el) el.removeAttribute('data-doudou-upload'); return true; }})()"
        );
        let _ = self.eval_isolated(&cleanup).await;
        result
    }

    async fn choose_files_via_click(
        &self,
        trigger: &ElementId,
        paths: &[&Path],
        timeout_ms: u64,
    ) -> Result<()> {
        // Prefer setting files directly on an <input type="file"> within or adjacent to the trigger
        // to avoid native OS file-chooser dialogs hanging headless Chrome.
        let id_json = serde_json::to_string(trigger.as_str()).unwrap_or_else(|_| "\"\"".to_owned());
        let find_input_script = format!(
            r"(() => {{
  {REGISTRY_PRELUDE}
  const el = __dd.get({id_json});
  if (!el) return null;
  if (el.tagName === 'INPUT' && el.type === 'file') return __dd.idOf(el);
  const inside = el.querySelector('input[type=file]');
  if (inside) return __dd.idOf(inside);
  const parent = el.parentElement;
  if (parent) {{
    const sibling = parent.querySelector('input[type=file]');
    if (sibling) return __dd.idOf(sibling);
  }}
  const globalInput = document.querySelector('input[type=file]');
  return globalInput ? __dd.idOf(globalInput) : null;
}})()"
        );
        if let Some(input_id) = self
            .eval_isolated(&find_input_script)
            .await?
            .as_str()
            .map(ElementId::new)
        {
            return self.set_files(&input_id, paths).await;
        }
        // Fallback: click the trigger to let React mount the input, then bind files.
        self.click_element(trigger, UiMouseButton::Left, timeout_ms)
            .await?;
        tokio::time::sleep(Duration::from_millis(150)).await;
        if let Some(input_id) = self
            .eval_isolated(&find_input_script)
            .await?
            .as_str()
            .map(ElementId::new)
        {
            return self.set_files(&input_id, paths).await;
        }
        Err(DoudouError::new(
            "PAGE_ADAPTER_DRIFTED",
            "No file input element became available after clicking the upload trigger.",
        ))
    }

    async fn wheel(&self, delta_x: f64, delta_y: f64) -> Result<()> {
        let (x, y) = self.pointer();
        self.page.mouse_wheel(x, y, delta_x, delta_y).await
    }

    async fn start_network_capture(
        &self,
        filter: CaptureFilter,
    ) -> Result<Box<dyn NetworkCaptureHandle>> {
        let buffer = self
            .page
            .enable_network_capture(filter.paths.clone())
            .await?;
        let start_index = buffer.snapshot().len();
        Ok(Box::new(CdpNetworkCaptureHandle {
            buffer,
            filter,
            start_index,
        }))
    }
}

/// [`doudou_ui::UiSession`] bridge backed by a shared [`ChromeSession`] and its active [`CdpUiPage`].
pub struct CdpUiSession {
    session: Arc<AsyncMutex<ChromeSession>>,
    page: CdpUiPage,
    auth_policy: AuthPolicy,
}

impl std::fmt::Debug for CdpUiSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CdpUiSession")
            .field("auth_policy", &self.auth_policy)
            .finish_non_exhaustive()
    }
}

impl CdpUiSession {
    /// Ensure `session` has a live page and wrap both into a [`CdpUiSession`].
    pub async fn from_session(
        session: Arc<AsyncMutex<ChromeSession>>,
        auth_policy: AuthPolicy,
    ) -> Result<Self> {
        let cdp_page = {
            let mut guard = session.lock().await;
            guard.ensure_page().await?
        };
        Ok(Self {
            session,
            page: CdpUiPage::new(cdp_page),
            auth_policy,
        })
    }

    /// Wrap an already-resolved page handle together with `session`.
    pub fn with_page(
        session: Arc<AsyncMutex<ChromeSession>>,
        cdp_page: Arc<dyn CdpPageOps>,
        auth_policy: AuthPolicy,
    ) -> Self {
        Self {
            session,
            page: CdpUiPage::new(cdp_page),
            auth_policy,
        }
    }

    /// Access the wrapped [`CdpUiPage`].
    pub fn cdp_ui_page(&self) -> &CdpUiPage {
        &self.page
    }
}

#[async_trait]
impl UiSession for CdpUiSession {
    fn page(&self) -> &dyn UiPage {
        &self.page
    }

    async fn ensure_ready(&self) -> Result<()> {
        let mut guard = self.session.lock().await;
        let _ = guard.ensure_authenticated(self.auth_policy).await?;
        Ok(())
    }

    async fn open(&self, url: &str) -> Result<()> {
        let mut guard = self.session.lock().await;
        let _ = guard.navigate_douyin(url).await?;
        Ok(())
    }
}

const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<CdpUiPage>();
    assert_send_sync::<CdpUiSession>();
    assert_send_sync::<CdpNetworkCaptureHandle>();
};

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use doudou_cdp::FakeCdpPage;

    use super::*;

    #[test]
    fn parse_css_clauses_handles_playwright_pseudos_and_comma_lists() {
        let auth_passkey = r#"[role="dialog"] button:has-text("Passkey"), [role="dialog"] button:has-text("通行密钥"), [class*="login" i] button:has-text("Passkey"), [class*="login" i] button:has-text("通行密钥")"#;
        let clauses = parse_css_clauses(auth_passkey);
        assert_eq!(
            clauses,
            vec![
                CssSelectorClause {
                    base_css: r#"[role="dialog"] button"#.to_owned(),
                    text_filters: vec![CssTextFilter::HasText("Passkey".to_owned())],
                    require_visible: false,
                },
                CssSelectorClause {
                    base_css: r#"[role="dialog"] button"#.to_owned(),
                    text_filters: vec![CssTextFilter::HasText("通行密钥".to_owned())],
                    require_visible: false,
                },
                CssSelectorClause {
                    base_css: r#"[class*="login" i] button"#.to_owned(),
                    text_filters: vec![CssTextFilter::HasText("Passkey".to_owned())],
                    require_visible: false,
                },
                CssSelectorClause {
                    base_css: r#"[class*="login" i] button"#.to_owned(),
                    text_filters: vec![CssTextFilter::HasText("通行密钥".to_owned())],
                    require_visible: false,
                },
            ]
        );

        let mixed = r#"button:has-text("Hello, world"), div:is(.a, .b):text-is('Exact, Text'):visible, :text("登录"), div > :visible, div.list:has([data-e2e="item"])"#;
        let parsed = parse_css_clauses(mixed);
        assert_eq!(
            parsed,
            vec![
                CssSelectorClause {
                    base_css: "button".to_owned(),
                    text_filters: vec![CssTextFilter::HasText("Hello, world".to_owned())],
                    require_visible: false,
                },
                CssSelectorClause {
                    base_css: "div:is(.a, .b)".to_owned(),
                    text_filters: vec![CssTextFilter::TextIs("Exact, Text".to_owned())],
                    require_visible: true,
                },
                CssSelectorClause {
                    base_css: "*".to_owned(),
                    text_filters: vec![CssTextFilter::Text("登录".to_owned())],
                    require_visible: false,
                },
                CssSelectorClause {
                    base_css: "div > *".to_owned(),
                    text_filters: vec![],
                    require_visible: true,
                },
                CssSelectorClause {
                    base_css: r#"div.list:has([data-e2e="item"])"#.to_owned(),
                    text_filters: vec![],
                    require_visible: false,
                },
            ]
        );
    }

    #[tokio::test]
    async fn query_all_embeds_parsed_clauses_without_raw_has_text_selector() {
        let fake = Arc::new(FakeCdpPage::new());
        fake.push_eval_result("querySelectorAll", json!(["el-1"]));
        let ui = CdpUiPage::new(Arc::clone(&fake) as Arc<dyn CdpPageOps>);

        let found = ui
            .query_all(
                None,
                &SelectorQuery::css(r#"[role="dialog"] button:has-text("Passkey"):visible"#),
                5,
            )
            .await
            .expect("query_all");
        assert_eq!(found, vec![ElementId::new("el-1")]);

        let evals = fake.evaluations();
        assert_eq!(evals.len(), 1);
        let expr = &evals[0].expression;
        assert!(expr.contains(r#""base_css":"[role=\"dialog\"] button""#));
        assert!(expr.contains(r#""kind":"has_text","value":"Passkey""#));
        assert!(expr.contains(r#""require_visible":true"#));
    }

    #[test]
    fn exchanges_to_snapshot_waits_for_response_body_some_while_allowing_empty_vec() {
        let filter = CaptureFilter::contains(["/v1/message/send"]);
        let make_exchange =
            |id: &str, status: Option<u16>, body: Option<Vec<u8>>| CapturedNetworkExchange {
                request_id: id.to_owned(),
                url: "https://imapi.douyin.com/v1/message/send".to_owned(),
                method: "POST".to_owned(),
                status,
                mime_type: Some("application/x-protobuf".to_owned()),
                request_headers: BTreeMap::new(),
                response_headers: BTreeMap::new(),
                request_body: Some(b"req".to_vec()),
                response_body: body,
                resource_type: Some("Fetch".to_owned()),
                timestamp: doudou_core::utc_now(),
            };

        // In-flight exchange between responseReceived and loadingFinished/getResponseBody:
        // status is Some(200) but response_body is None -> must NOT emit CapturedResponse yet.
        let inflight = vec![make_exchange("r-inflight", Some(200), None)];
        let snap = exchanges_to_snapshot(&inflight, &filter);
        assert_eq!(snap.requests.len(), 1);
        assert!(
            snap.responses.is_empty(),
            "in-flight exchange without response_body must not emit CapturedResponse"
        );

        // Completed exchange with empty body Some(vec![]) (e.g. fake/test or empty HTTP body):
        // MUST still emit CapturedResponse, while CORS OPTIONS preflights are skipped completely.
        let mut options_preflight = make_exchange("r-options", Some(200), Some(Vec::new()));
        options_preflight.method = "OPTIONS".to_owned();
        let completed_empty = vec![
            options_preflight,
            make_exchange("r-empty", Some(200), Some(Vec::new())),
            make_exchange("r-full", Some(200), Some(b"ok".to_vec())),
        ];
        let snap2 = exchanges_to_snapshot(&completed_empty, &filter);
        assert_eq!(snap2.requests.len(), 2);
        assert_eq!(snap2.responses.len(), 2);
        assert_eq!(snap2.responses[0].body, b"");
        assert_eq!(snap2.responses[1].body, b"ok");
    }
}
