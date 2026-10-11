//! The optional rvoip dispatch hook preserves staged/default call behavior.
#![allow(clippy::unwrap_used)]
mod gate3_support;
use rvoip_core::{
    adapter::{ConnectionAdapter, EndReason, OriginateRequest},
    connection::Direction,
    ids::{ParticipantId, SessionId},
};
use rvoip_vapi::{VapiAssistant, VapiCallAdmission, VapiCallOptions, VapiError};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct Deny(AtomicUsize);
#[async_trait::async_trait]
impl VapiCallAdmission for Deny {
    async fn admit(&self) -> rvoip_vapi::Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(VapiError::InvalidCallOptions("fixture admission denied"))
    }
}

#[tokio::test]
async fn hook_denial_precedes_post_and_absent_hook_preserves_existing_call_creation() {
    let mut fake = gate3_support::FakeVapi::start().await;
    let deny = Arc::new(Deny(AtomicUsize::new(0)));
    for guarded in [true, false] {
        let options =
            VapiCallOptions::new(VapiAssistant::saved("12345678-1234-1234-1234-123456789012"));
        let options = if guarded {
            options.with_admission(deny.clone())
        } else {
            options
        };
        let handle = fake
            .adapter
            .originate(
                OriginateRequest::new(
                    SessionId::new(),
                    ParticipantId::new(),
                    "fixture",
                    Direction::Outbound,
                    fake.adapter.capabilities(),
                )
                .with_context(options),
            )
            .await
            .unwrap();
        fake.assert_no_message("create-call").await;
        let activated = fake
            .adapter
            .activate_outbound(handle.connection.id.clone())
            .await;
        assert_eq!(activated.is_err(), guarded);
        if guarded {
            fake.assert_no_message("create-call").await;
            assert_eq!(deny.0.load(Ordering::SeqCst), 1);
        } else {
            fake.next("create-call").await;
            assert_eq!(deny.0.load(Ordering::SeqCst), 1);
            fake.adapter
                .end(handle.connection.id, EndReason::Normal)
                .await
                .unwrap();
        }
    }
}
