//! Douyin UI page adapters, geometry/occlusion checks, and chat/video/auth components.
//!
//! This crate intentionally has zero dependency on CDP or browser-driver crates. Runtime
//! adapters implement [`UiPage`] and [`UiSession`], while tests use [`mock::MockUiPage`].

pub mod adapters;
pub mod chat;
pub mod components;
pub mod geometry;
pub mod mock;
pub mod page;
pub mod video;

pub use adapters::{
    ADAPTERS, AUTH_ADAPTER, CHAT_ADAPTER, PROFILE_ADAPTER, PageAdapter, VIDEO_ADAPTER,
};
pub use chat::{SEND_PATHS, like_message, open_conversation, send_message, verify_send_capture};
pub use components::{
    AuthChallengeComponent, ChatComposerComponent, CommentItemComponent, ConversationListComponent,
    ImagePreviewComponent, MessageEvidence, MessageItemComponent, ProfileVideoListComponent,
    TutorialDialogComponent, VideoCommentComposerComponent, VideoCommentsPanelComponent,
    VideoDetailComponent, message_semantics_match, visible_comment_text,
};
pub use geometry::{
    AnchorRelation, ClickEvidence, ClickOptions, ClickPoint, ComponentContext,
    DEFAULT_CANDIDATE_LIMIT, DEFAULT_SCOPE_TOLERANCE, DEFAULT_STABILITY_TOLERANCE,
    EnabledRequirement, GeometryFilter, PeerPosition, ScrollBehavior, TargetOptions,
    click_verified_target, component_probe, drift_error, not_exercised_probe, require_target,
    resolution_probe, unique_visible, visible_candidates,
};
pub use page::{
    BlockerInfo, BoundingBox, CaptureFilter, CapturedRequest, CapturedResponse, ElementId,
    EndpointSummary, MouseButton, NetworkCaptureHandle, NetworkCaptureSnapshot, PathMatchMode,
    PointHit, SelectorQuery, TextMatchMode, UiPage, UiSession, endpoint_summary,
};
pub use video::{SENTINELS, captured_write, comment_write, like_comment};
