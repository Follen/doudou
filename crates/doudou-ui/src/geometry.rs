//! Geometry validation, visibility filtering, occlusion-checked clicks, and probes.

use std::future::Future;

use doudou_core::{DoudouError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::page::{BoundingBox, ElementId, MouseButton, UiPage};

/// Default maximum candidate count inspected during visibility filtering.
pub const DEFAULT_CANDIDATE_LIMIT: usize = 40;

/// Default bounding-box scope containment tolerance in CSS pixels.
pub const DEFAULT_SCOPE_TOLERANCE: f64 = 2.0;

/// Default bounding-box stability tolerance before clicking in CSS pixels.
pub const DEFAULT_STABILITY_TOLERANCE: f64 = 1.0;

/// Relative direction between a candidate target and an anchor element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorRelation {
    Above,
    Below,
    LeftOf,
    RightOf,
}

impl AnchorRelation {
    fn satisfied(self, target_center: (f64, f64), anchor_center: (f64, f64)) -> bool {
        match self {
            Self::Above => target_center.1 < anchor_center.1,
            Self::Below => target_center.1 > anchor_center.1,
            Self::LeftOf => target_center.0 < anchor_center.0,
            Self::RightOf => target_center.0 > anchor_center.0,
        }
    }
}

/// Expected horizontal extreme among sibling peer controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerPosition {
    Rightmost,
    Leftmost,
}

impl PeerPosition {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Rightmost => "rightmost",
            Self::Leftmost => "leftmost",
        }
    }
}

/// Whether [`require_target`] requires the resolved element to be enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnabledRequirement {
    #[default]
    Any,
    EnabledOnly,
}

/// Whether [`require_target`] pre-filters visible candidates by scope/anchor geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GeometryFilter {
    #[default]
    ValidateOnly,
    FilterCandidates,
}

/// Whether [`click_verified_target`] scrolls the target into view before measuring stability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollBehavior {
    #[default]
    IntoView,
    Skip,
}

/// Component identity metadata attached to every drift diagnostic or probe report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentContext<'a> {
    pub component: &'a str,
    pub version: &'a str,
    pub capability: &'a str,
    pub strategy: &'a str,
}

impl<'a> ComponentContext<'a> {
    /// Construct a new component diagnostic context.
    pub const fn new(
        component: &'a str,
        version: &'a str,
        capability: &'a str,
        strategy: &'a str,
    ) -> Self {
        Self {
            component,
            version,
            capability,
            strategy,
        }
    }
}

/// Geometric and semantic constraints enforced by [`require_target`].
#[derive(Debug, Clone)]
pub struct TargetOptions<'a> {
    pub enabled: EnabledRequirement,
    pub scope: Option<&'a ElementId>,
    pub peers: Option<&'a [ElementId]>,
    pub position: Option<PeerPosition>,
    pub minimum_peer_count: usize,
    pub anchor: Option<&'a ElementId>,
    pub relation: Option<AnchorRelation>,
    pub max_anchor_distance: Option<f64>,
    pub filter: GeometryFilter,
    pub scope_tolerance: f64,
    pub candidate_limit: usize,
}

impl Default for TargetOptions<'_> {
    fn default() -> Self {
        Self {
            enabled: EnabledRequirement::Any,
            scope: None,
            peers: None,
            position: None,
            minimum_peer_count: 1,
            anchor: None,
            relation: None,
            max_anchor_distance: None,
            filter: GeometryFilter::ValidateOnly,
            scope_tolerance: DEFAULT_SCOPE_TOLERANCE,
            candidate_limit: DEFAULT_CANDIDATE_LIMIT,
        }
    }
}

impl<'a> TargetOptions<'a> {
    /// Require the resolved target to be enabled.
    #[must_use]
    pub const fn require_enabled(mut self) -> Self {
        self.enabled = EnabledRequirement::EnabledOnly;
        self
    }

    /// Constrain the resolved target to lie within `scope`.
    #[must_use]
    pub const fn scope(mut self, scope: &'a ElementId) -> Self {
        self.scope = Some(scope);
        self
    }

    /// Override the scope containment tolerance in pixels.
    #[must_use]
    pub const fn scope_tolerance(mut self, tolerance: f64) -> Self {
        self.scope_tolerance = tolerance;
        self
    }

    /// Require the target to occupy `position` among `peers` with at least `minimum_peer_count` valid boxes.
    #[must_use]
    pub const fn peers(
        mut self,
        peers: &'a [ElementId],
        position: PeerPosition,
        minimum_peer_count: usize,
    ) -> Self {
        self.peers = Some(peers);
        self.position = Some(position);
        self.minimum_peer_count = minimum_peer_count;
        self
    }

    /// Constrain the target relative to `anchor`.
    #[must_use]
    pub const fn anchor(
        mut self,
        anchor: &'a ElementId,
        relation: Option<AnchorRelation>,
        max_anchor_distance: Option<f64>,
    ) -> Self {
        self.anchor = Some(anchor);
        self.relation = relation;
        self.max_anchor_distance = max_anchor_distance;
        self
    }

    /// Pre-filter visible candidates using scope and anchor geometry before uniqueness checking.
    #[must_use]
    pub const fn filter_by_geometry(mut self) -> Self {
        self.filter = GeometryFilter::FilterCandidates;
        self
    }
}

/// Configuration for [`click_verified_target`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClickOptions {
    pub stability_tolerance: f64,
    pub scroll: ScrollBehavior,
    pub settle_ms: u64,
    pub scroll_timeout_ms: u64,
    pub click_timeout_ms: u64,
}

impl Default for ClickOptions {
    fn default() -> Self {
        Self {
            stability_tolerance: DEFAULT_STABILITY_TOLERANCE,
            scroll: ScrollBehavior::IntoView,
            settle_ms: 100,
            scroll_timeout_ms: 5_000,
            click_timeout_ms: 5_000,
        }
    }
}

impl ClickOptions {
    /// Skip `scroll_into_view` when the caller has already scrolled the target into local scope.
    #[must_use]
    pub const fn without_scroll(mut self) -> Self {
        self.scroll = ScrollBehavior::Skip;
        self
    }
}

/// Measured click coordinates recorded in [`ClickEvidence`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClickPoint {
    pub x: f64,
    pub y: f64,
}

/// Diagnostic evidence produced by [`click_verified_target`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClickEvidence {
    pub stable: bool,
    pub box_delta: f64,
    pub receives_events: bool,
    pub dispatch_completed: bool,
    pub click_point: ClickPoint,
}

/// Build a `PAGE_ADAPTER_DRIFTED` error with standard component context and extra fields.
pub fn drift_error(
    message: impl Into<String>,
    ctx: &ComponentContext<'_>,
    extra: Value,
) -> DoudouError {
    let mut details = Map::new();
    details.insert("component".to_owned(), json!(ctx.component));
    details.insert("component_version".to_owned(), json!(ctx.version));
    details.insert("capability".to_owned(), json!(ctx.capability));
    details.insert("strategy".to_owned(), json!(ctx.strategy));
    if let Value::Object(map) = extra {
        for (k, v) in map {
            details.insert(k, v);
        }
    }
    DoudouError::new("PAGE_ADAPTER_DRIFTED", message).with_details(Value::Object(details))
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn euclidean_distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// Return up to `limit` elements from `candidates` that are currently visible.
pub async fn visible_candidates(
    page: &dyn UiPage,
    candidates: &[ElementId],
    limit: usize,
) -> Result<Vec<ElementId>> {
    let mut visible = Vec::new();
    for item in candidates.iter().take(limit) {
        if page.is_visible(item).await? {
            visible.push(item.clone());
        }
    }
    Ok(visible)
}

/// Click a verified DOM target at its measured center after stability and occlusion checks.
///
/// Never uses fixed coordinates: measures the bounding box twice across `settle_ms`, checks
/// `elementFromPoint` ancestor-chain containment at the center, and dispatches a pointer click.
/// If click dispatch times out (`CLICK_TIMEOUT`), returns `dispatch_completed = false` so the
/// caller can confirm success from the DOM state postcondition.
pub async fn click_verified_target(
    page: &dyn UiPage,
    target: &ElementId,
    ctx: &ComponentContext<'_>,
    options: ClickOptions,
) -> Result<ClickEvidence> {
    let (first_box, second_box) = async {
        if options.scroll == ScrollBehavior::IntoView {
            page.scroll_into_view(target, options.scroll_timeout_ms)
                .await?;
        }
        let first = page.bounding_box(target).await?;
        page.sleep_ms(options.settle_ms).await?;
        let second = page.bounding_box(target).await?;
        Ok::<_, DoudouError>((first, second))
    }
    .await
    .map_err(|error| {
        drift_error(
            "The page target could not be prepared for a normal mouse click.",
            ctx,
            json!({
                "geometry_reason": "target_actionability_failed",
                "cause_type": error.code,
            }),
        )
    })?;

    let (Some(first_box), Some(second_box)) = (
        first_box.filter(BoundingBox::is_valid),
        second_box.filter(BoundingBox::is_valid),
    ) else {
        return Err(drift_error(
            "The page target lost its usable geometry before the click.",
            ctx,
            json!({ "geometry_reason": "target_box_missing_before_click" }),
        ));
    };

    let delta = first_box.max_delta(&second_box);
    if delta > options.stability_tolerance {
        return Err(drift_error(
            "The page target was still moving before the click.",
            ctx,
            json!({
                "geometry_reason": "target_unstable",
                "box_delta": round3(delta),
            }),
        ));
    }

    let (center_x, center_y) = second_box.center();
    let hit = page.element_from_point(center_x, center_y).await?;
    let receives_events = hit.as_ref().is_some_and(|h| h.receives_events_for(target));
    if !receives_events {
        let blocker = hit.as_ref().map(crate::page::PointHit::as_blocker);
        return Err(drift_error(
            "The page target is covered and cannot receive the normal mouse click.",
            ctx,
            json!({
                "geometry_reason": "target_occluded",
                "blocker": blocker,
            }),
        ));
    }

    let dispatch_completed = match page
        .click_xy(
            center_x,
            center_y,
            MouseButton::Left,
            options.click_timeout_ms,
        )
        .await
    {
        Ok(()) => true,
        Err(error) if error.code == "CLICK_TIMEOUT" => false,
        Err(error) => return Err(error),
    };

    Ok(ClickEvidence {
        stable: true,
        box_delta: round3(delta),
        receives_events: true,
        dispatch_completed,
        click_point: ClickPoint {
            x: round3(center_x),
            y: round3(center_y),
        },
    })
}

/// Resolve and validate exactly one visible target matching all semantic and geometric contracts.
pub async fn require_target(
    page: &dyn UiPage,
    raw_candidates: &[ElementId],
    ctx: &ComponentContext<'_>,
    options: &TargetOptions<'_>,
) -> Result<ElementId> {
    let total_count = raw_candidates.len().min(options.candidate_limit);
    let mut candidates = visible_candidates(page, raw_candidates, options.candidate_limit).await?;
    let scope_box = match options.scope {
        Some(scope) => page
            .bounding_box(scope)
            .await?
            .filter(BoundingBox::is_valid),
        None => None,
    };
    let anchor_box = match options.anchor {
        Some(anchor) => page
            .bounding_box(anchor)
            .await?
            .filter(BoundingBox::is_valid),
        None => None,
    };

    if options.filter == GeometryFilter::FilterCandidates {
        let mut filtered = Vec::new();
        for item in &candidates {
            let Some(item_box) = page.bounding_box(item).await?.filter(BoundingBox::is_valid)
            else {
                continue;
            };
            if options.scope.is_some() {
                let Some(sbox) = scope_box else {
                    continue;
                };
                if !sbox.contains(&item_box, options.scope_tolerance) {
                    continue;
                }
            }
            if options.anchor.is_some() {
                let Some(abox) = anchor_box else {
                    continue;
                };
                let item_center = item_box.center();
                let anchor_center = abox.center();
                let relation_ok = options
                    .relation
                    .is_none_or(|rel| rel.satisfied(item_center, anchor_center));
                let distance = euclidean_distance(item_center, anchor_center);
                if !relation_ok
                    || options
                        .max_anchor_distance
                        .is_some_and(|max_dist| distance > max_dist)
                {
                    continue;
                }
            }
            filtered.push(item.clone());
        }
        candidates = filtered;
    }

    let mut enabled_count = 0usize;
    for item in &candidates {
        if page.is_enabled(item).await? {
            enabled_count += 1;
        }
    }

    if candidates.len() != 1 {
        return Err(drift_error(
            "The page component did not resolve to one visible target.",
            ctx,
            json!({
                "candidate_count": total_count,
                "visible_count": candidates.len(),
                "enabled_count": enabled_count,
                "geometry_reason": "target_not_unique",
            }),
        ));
    }

    let target = candidates
        .into_iter()
        .next()
        .expect("candidates length checked to be 1");

    if options.enabled == EnabledRequirement::EnabledOnly && enabled_count != 1 {
        return Err(drift_error(
            "The page component target is not enabled.",
            ctx,
            json!({
                "candidate_count": total_count,
                "visible_count": 1,
                "enabled_count": 0,
                "geometry_reason": "target_disabled",
            }),
        ));
    }

    let Some(target_box) = page
        .bounding_box(&target)
        .await?
        .filter(BoundingBox::is_valid)
    else {
        return Err(drift_error(
            "The page component target has no usable geometry.",
            ctx,
            json!({
                "candidate_count": total_count,
                "visible_count": 1,
                "enabled_count": enabled_count,
                "geometry_reason": "target_box_missing",
            }),
        ));
    };

    if options.scope.is_some()
        && !scope_box.is_some_and(|sbox| sbox.contains(&target_box, options.scope_tolerance))
    {
        return Err(drift_error(
            "The page component target is outside its confirmed local scope.",
            ctx,
            json!({
                "candidate_count": total_count,
                "visible_count": 1,
                "enabled_count": enabled_count,
                "geometry_reason": "outside_local_scope",
            }),
        ));
    }

    if let (Some(peers), Some(position)) = (options.peers, options.position) {
        let visible_peers = visible_candidates(page, peers, DEFAULT_CANDIDATE_LIMIT).await?;
        let mut peer_boxes = Vec::new();
        for peer in &visible_peers {
            if let Some(pbox) = page.bounding_box(peer).await?.filter(BoundingBox::is_valid) {
                peer_boxes.push(pbox);
            }
        }
        if peer_boxes.len() < options.minimum_peer_count {
            return Err(drift_error(
                "The local component actions have no usable geometry.",
                ctx,
                json!({
                    "candidate_count": total_count,
                    "visible_count": 1,
                    "enabled_count": enabled_count,
                    "geometry_reason": "peer_boxes_missing",
                    "peer_count": peer_boxes.len(),
                    "minimum_peer_count": options.minimum_peer_count,
                }),
            ));
        }
        let expected_x = match position {
            PeerPosition::Rightmost => peer_boxes
                .iter()
                .map(|b| b.x)
                .fold(f64::NEG_INFINITY, f64::max),
            PeerPosition::Leftmost => peer_boxes.iter().map(|b| b.x).fold(f64::INFINITY, f64::min),
        };
        if (target_box.x - expected_x).abs() > 2.0 {
            return Err(drift_error(
                format!(
                    "The page component target is not the expected {} action.",
                    position.as_str()
                ),
                ctx,
                json!({
                    "candidate_count": total_count,
                    "visible_count": 1,
                    "enabled_count": enabled_count,
                    "geometry_reason": format!("not_{}", position.as_str()),
                }),
            ));
        }
    }

    if options.anchor.is_some() {
        let Some(abox) = anchor_box else {
            return Err(drift_error(
                "The page component anchor has no usable geometry.",
                ctx,
                json!({ "geometry_reason": "anchor_box_missing" }),
            ));
        };
        let target_center = target_box.center();
        let anchor_center = abox.center();
        let relation_ok = options
            .relation
            .is_none_or(|rel| rel.satisfied(target_center, anchor_center));
        let distance = euclidean_distance(target_center, anchor_center);
        if !relation_ok
            || options
                .max_anchor_distance
                .is_some_and(|max_dist| distance > max_dist)
        {
            return Err(drift_error(
                "The page component target violates its anchor geometry contract.",
                ctx,
                json!({
                    "candidate_count": total_count,
                    "visible_count": 1,
                    "enabled_count": enabled_count,
                    "geometry_reason": "anchor_relation_mismatch",
                }),
            ));
        }
    }

    Ok(target)
}

/// Convenience wrapper requiring a single visible element with default options.
pub async fn unique_visible(
    page: &dyn UiPage,
    candidates: &[ElementId],
    ctx: &ComponentContext<'_>,
) -> Result<ElementId> {
    require_target(page, candidates, ctx, &TargetOptions::default()).await
}

/// Produce a diagnostic probe report for a candidate list.
pub async fn component_probe(
    page: &dyn UiPage,
    raw_candidates: &[ElementId],
    ctx: &ComponentContext<'_>,
    exercised: bool,
) -> Result<Value> {
    if !exercised {
        return Ok(json!({
            "component": ctx.component,
            "component_version": ctx.version,
            "capability": ctx.capability,
            "exercised": false,
            "status": "not_exercised",
            "strategy": ctx.strategy,
        }));
    }
    let total_count = raw_candidates.len().min(DEFAULT_CANDIDATE_LIMIT);
    let candidates = visible_candidates(page, raw_candidates, DEFAULT_CANDIDATE_LIMIT).await?;
    let mut enabled_count = 0usize;
    for item in &candidates {
        if page.is_enabled(item).await? {
            enabled_count += 1;
        }
    }
    let geometry_valid = if candidates.len() == 1 {
        page.bounding_box(&candidates[0])
            .await?
            .is_some_and(|b| b.is_valid())
    } else {
        false
    };
    let reason = if candidates.len() != 1 {
        Some("target_not_unique")
    } else if enabled_count != 1 {
        Some("target_disabled")
    } else if !geometry_valid {
        Some("target_box_missing")
    } else {
        None
    };
    let ok = candidates.len() == 1 && enabled_count == 1 && geometry_valid;
    Ok(json!({
        "component": ctx.component,
        "component_version": ctx.version,
        "capability": ctx.capability,
        "exercised": true,
        "status": if ok { "ok" } else { "drifted" },
        "strategy": ctx.strategy,
        "candidate_count": total_count,
        "visible_count": candidates.len(),
        "enabled_count": enabled_count,
        "unique": candidates.len() == 1,
        "geometry_valid": geometry_valid,
        "geometry_reason": reason,
    }))
}

/// Produce a standard `not_exercised` diagnostic probe entry.
pub fn not_exercised_probe(ctx: &ComponentContext<'_>, reason: &str) -> Value {
    json!({
        "component": ctx.component,
        "component_version": ctx.version,
        "capability": ctx.capability,
        "exercised": false,
        "status": "not_exercised",
        "strategy": ctx.strategy,
        "reason": reason,
    })
}

/// Run an async target resolver and package its success or `PAGE_ADAPTER_DRIFTED` failure as a probe.
pub async fn resolution_probe<F, Fut>(
    page: &dyn UiPage,
    resolver: F,
    ctx: &ComponentContext<'_>,
    exercised: bool,
    not_exercised_reason: &str,
) -> Result<Value>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<ElementId>>,
{
    if !exercised {
        return Ok(not_exercised_probe(ctx, not_exercised_reason));
    }
    match resolver().await {
        Ok(target) => {
            let geometry_valid = page
                .bounding_box(&target)
                .await?
                .is_some_and(|b| b.is_valid());
            let enabled = page.is_enabled(&target).await?;
            Ok(json!({
                "component": ctx.component,
                "component_version": ctx.version,
                "capability": ctx.capability,
                "exercised": true,
                "status": "ok",
                "strategy": ctx.strategy,
                "candidate_count": 1,
                "visible_count": 1,
                "enabled_count": usize::from(enabled),
                "unique": true,
                "geometry_valid": geometry_valid,
                "geometry_reason": Value::Null,
            }))
        }
        Err(error) if error.code == "PAGE_ADAPTER_DRIFTED" => {
            let mut report = Map::new();
            report.insert("component".to_owned(), json!(ctx.component));
            report.insert("component_version".to_owned(), json!(ctx.version));
            report.insert("capability".to_owned(), json!(ctx.capability));
            report.insert("exercised".to_owned(), json!(true));
            report.insert("status".to_owned(), json!("drifted"));
            report.insert("strategy".to_owned(), json!(ctx.strategy));
            if let Some(Value::Object(details)) = error.details {
                for (k, v) in details {
                    report.insert(k, v);
                }
            }
            Ok(Value::Object(report))
        }
        Err(error) => Err(error),
    }
}
