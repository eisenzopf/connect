//! Native Connect SDK with mandatory TLS trust and caller-owned logical IDs.
//!
//! The caller persists a mutation's request ID and inputs before sending. A
//! timeout, cancellation or disconnect may leave acceptance unknown. Reconnect
//! and retry the same operation/ID, or look up its saved receipt; never invent a
//! new logical ID to resolve uncertainty. Inbox events are delivered at least
//! once: store app effects, processed event IDs and cursor atomically locally.
//!
//! ```no_run
//! # async fn example() -> connect_client::Result<()> {
//! use connect_client::{ClientConfig,ConnectClient};
//! let config=ClientConfig::load(std::path::Path::new(".local/demo/reviews.toml"))?;
//! let mut client=ConnectClient::open(config).await?;
//! let agent=client.resolve("connect:agent@local").await?;
//! let conversation=client.create("saved-create-001",vec![agent.id]).await?;
//! let receipt=client.send("saved-send-001",&conversation.id,"Review draft-002").await?;
//! let page=client.history(&conversation.id,None,100).await?;
//! assert!(page.messages.iter().any(|m|m.id==receipt.message_id));
//! client.close().await;
//! # Ok(()) }
//! ```
mod client;
mod config;
mod contacts;
mod discovery;
mod error;
mod media;
mod media_end;
mod operations;
pub mod task_diagnostics;
mod task_operations;
mod video;
pub use video::{
    NativeAvFrame, NativeVideoFrame, NativeVideoPacket, NativeVideoSession,
    VideoAssemblyDiagnostics, VideoFrameAssembler, VideoSequenceRange,
};

pub use client::ConnectClient;
pub use config::ClientConfig;
/// Typed task commands/results. Invocation issuance stays in trusted composition.
pub use connect_protocol::tasks;
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
