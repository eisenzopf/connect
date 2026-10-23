//! Signed Connect control and explicit local application APIs.
//!
//! Persist logical operation IDs, message IDs and intent before submitting.
//! Reconnect and replay those IDs after uncertainty; never replace them to hide
//! unknown acceptance. `canonical` owns the signed native peer and typed command
//! batches. `application` owns the separately configured HTTP application APIs;
//! it cannot be used as a fallback for canonical communication commands.
//!
//! ```no_run
//! # async fn example() -> connect_client::Result<()> {
//! use connect_client::{canonical::{CanonicalClient, CanonicalClientConfig}, conversations::CreateRequest, MessageDraft};
//! let path=std::path::Path::new(".local/signed/reviews.toml");
//! let peer=CanonicalClientConfig::load(std::path::Path::new(".local/signed/agent.toml"))?;
//! let mut client=CanonicalClient::open(CanonicalClientConfig::load(path)?).await?;
//! let mut input=CreateRequest::persistent(vec![peer.identity_id]);
//! input.tenant_id=Some("tnt_default".into());
//! let conversation=client.create_conversation("saved-create-001", input).await?;
//! let receipt=client.send_message("saved-send-001", &conversation.id, "saved_message_001", MessageDraft::text_all("Review draft-002")).await?;
//! assert!(receipt.receipt.matches("saved-send-001", &conversation.id));
//! client.shutdown().await?;
//! # Ok(()) }
//! ```
//!
//! The earlier `ConnectClient`/`ClientConfig` facade and its media/application
//! consumers still await migration. They do not accept fresh signed configs.
pub mod application;
pub mod canonical;
mod client;
mod config;
mod contacts;
mod discovery;
mod error;
mod event_operations;
mod media;
mod media_end;
mod operations;
mod receipt_operations;
pub mod task_diagnostics;
mod task_operations;
mod transition_operations;
mod video;
pub use video::{
    NativeAvFrame, NativeVideoFrame, NativeVideoPacket, NativeVideoSession,
    VideoAssemblyDiagnostics, VideoFrameAssembler, VideoSequenceRange,
};

pub use client::ConnectClient;
pub use config::ClientConfig;
pub use connect_protocol::attachments::{ArtifactLink, LinkPurpose};
pub use connect_protocol::conversations;
pub use connect_protocol::events;
pub use connect_protocol::membership;
pub use connect_protocol::messages;
pub use connect_protocol::messages::{AttachmentRef, BodyEncoding, MessageAudience, MessageDraft};
pub use connect_protocol::receipts;
pub use connect_protocol::routing;
pub use connect_protocol::session_decisions;
/// Typed task commands/results. Invocation issuance stays in trusted composition.
pub use connect_protocol::tasks;
pub use connect_protocol::transitions;
pub use connect_protocol::{
    Artifact, ArtifactChunk, Availability, AvailabilityState, BrowserBinding, Call, Capabilities,
    CapabilityLimits, ComponentReadiness, ContextDeliveryPage, ContextItem, ContextView,
    Conversation, Event, Identity, Message, Operation, Outcome, Readiness, ReadinessState,
    RouteCapability, Session, SipLeg, PROFILE,
};
pub use error::{ClientError, Result};
pub use media::{NativeAudioFrame, NativeAudioSession};
pub use operations::{
    ArtifactUpload, HistoryPage, InboxPage, RequestState, SendReceipt, ServerHealth,
};

pub use connect_protocol::session_control;
