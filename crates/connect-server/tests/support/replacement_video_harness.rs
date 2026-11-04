//! Focused production-listener feedback cases, independent of unrelated suites.
use super::*;
use connect_client::sdk::{
    self, Client, ConversationPolicy, MutationOptions, NonEmpty, RequestOptions,
};
fn options() -> RequestOptions {
    RequestOptions::new(Duration::from_secs(10)).unwrap()
}
fn mutation(id: &str) -> MutationOptions {
    MutationOptions {
        operation_id: id.parse().unwrap(),
        request: options(),
        extensions: None,
    }
}
#[path = "sdk_handles/replacement_video_recovery.rs"]
mod replacement_video_recovery;
