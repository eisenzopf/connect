//! Current-authorized per-recipient/channel evidence; never proof of human action.
//!
//! ```no_run
//! # async fn receipts(config: connect_client::sdk::ClientConfig,
//! # cid: connect_client::sdk::ConversationId, message: connect_client::sdk::MessageId)
//! # -> connect_client::sdk::Result<()> {
//! use connect_client::sdk::{Client, RequestOptions};
//! use std::time::Duration;
//! let client = Client::connect_configured(config).await?;
//! let wait = RequestOptions::new(Duration::from_secs(5))?;
//! let result = client.conversation_by_id(cid).receipts(message, wait.clone()).await;
//! client.close(wait).await?;
//! for fact in result? {
//!     // Preserve channel, epoch and provenance when interpreting the state.
//!     let _ = (fact.state, fact.channel, fact.membership_epoch, fact.provenance);
//! }
//! # Ok(()) }
//! ```
use super::*;
pub use connect_protocol::receipts::DeliveryState;
#[derive(Clone)]
pub struct DeliveryObservation {
    pub participant_id: ParticipantId,
    pub membership_epoch: u64,
    pub channel: String,
    pub delivery_id: String,
    pub state: DeliveryState,
    pub provenance: String,
    pub observed_at: i64,
}
impl std::fmt::Debug for DeliveryObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeliveryObservation")
            .field("state", &self.state)
            .field("channel", &self.channel)
            .field("provenance", &self.provenance)
            .finish_non_exhaustive()
    }
}
fn text(value: &str, bound: usize) -> bool {
    !value.is_empty() && value.len() <= bound && value.bytes().all(|b| b.is_ascii_graphic())
}
impl Conversation {
    /// Requires explicitly configured application read services. All pages share
    /// one deadline; any refusal/malformed page discards the accumulated result.
    /// Facts reflect current per-page authority/preferences, not an atomic snapshot.
    pub async fn receipts(
        &self,
        message_id: MessageId,
        options: RequestOptions,
    ) -> Result<Vec<DeliveryObservation>> {
        let fail = |kind| Error::request(kind, None, false);
        options.validate().map_err(|e| fail(e.kind))?;
        let application = self
            .client()
            .application
            .as_ref()
            .ok_or_else(|| fail(ErrorKind::Unsupported))?;
        let expires = tokio::time::Instant::now() + options.timeout;
        let mut after = None;
        let mut seen_cursors = std::collections::BTreeSet::new();
        let mut groups = std::collections::BTreeSet::new();
        let mut observations = Vec::new();
        let mut bytes = 0usize;
        for _ in 0..256 {
            let remaining = expires.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(fail(ErrorKind::Deadline));
            }
            let page = application
                .receipts(
                    self.id(),
                    &message_id,
                    after,
                    RequestOptions {
                        timeout: remaining,
                        signal: options.signal.clone(),
                    },
                )
                .await?;
            if page.receipts.len() > 100 {
                return Err(fail(ErrorKind::Protocol));
            }
            for receipt in page.receipts {
                bytes += serde_json::to_vec(&receipt)
                    .map_err(|_| fail(ErrorKind::Protocol))?
                    .len();
                if bytes > 2 * 1024 * 1024 || observations.len() >= 4096 {
                    return Err(fail(ErrorKind::ResyncRequired));
                }
                if receipt.conversation_id != self.id().as_str()
                    || receipt.message_id != message_id.as_str()
                    || receipt.membership_epoch == 0
                    || receipt.membership_epoch > 9_007_199_254_740_991
                    || !text(&receipt.channel, 64)
                    || !text(&receipt.delivery_id, 512)
                    || !matches!(
                        receipt.provenance.as_str(),
                        "journal" | "client_reported" | "provider_observation" | "local_attempt"
                    )
                    || receipt.observed_at < 0
                    || receipt.observed_at > 9_007_199_254_740_991
                {
                    return Err(fail(ErrorKind::Protocol));
                }
                let participant_id = receipt
                    .participant_id
                    .parse()
                    .map_err(|_| fail(ErrorKind::Protocol))?;
                if !groups.insert((
                    receipt.participant_id,
                    receipt.membership_epoch,
                    receipt.channel.clone(),
                    receipt.delivery_id.clone(),
                )) {
                    return Err(fail(ErrorKind::ResyncRequired));
                }
                observations.push(DeliveryObservation {
                    participant_id,
                    membership_epoch: receipt.membership_epoch,
                    channel: receipt.channel,
                    delivery_id: receipt.delivery_id,
                    state: receipt.state,
                    provenance: receipt.provenance,
                    observed_at: receipt.observed_at,
                });
            }
            match page.next {
                Some(cursor) if text(&cursor, 2048) && seen_cursors.insert(cursor.clone()) => {
                    after = Some(cursor)
                }
                Some(_) => return Err(fail(ErrorKind::ResyncRequired)),
                None => return Ok(observations),
            }
        }
        Err(fail(ErrorKind::ResyncRequired))
    }
}
