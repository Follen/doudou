//! Douyin UI page components covering auth, chat, and video workflows.

pub mod auth;
pub mod chat;
pub mod video;

pub use auth::AuthChallengeComponent;
pub use chat::{
    ChatComposerComponent, ConversationListComponent, ImagePreviewComponent, MessageEvidence,
    MessageItemComponent, message_semantics_match,
};
pub use video::{
    CommentItemComponent, ProfileVideoListComponent, TutorialDialogComponent,
    VideoCommentComposerComponent, VideoCommentsPanelComponent, VideoDetailComponent,
    visible_comment_text,
};
