//! Explicit bearer-authenticated HTTP adapter over the canonical service router.
use crate::{CanonicalHandler, Service, StoreCredentialBinding};
use connect_protocol::{canonical_http::*, Error, Result};
use rvoip_uctp::v2::{scope_policy, Envelope, Scope};
use serde_json::json;
use std::sync::Arc;

impl Service {
    /// Binding facts must come from this installation's canonical bearer
    /// validator. They are not a signed physical-peer grant and never enter the
    /// peer-only Store entry points.
    pub async fn execute_canonical_http(
        self: &Arc<Self>,
        binding: StoreCredentialBinding,
        request: Envelope,
    ) -> Result<(u16, ReplyBatch)> {
        let credential = self
            .authorize_local_principal(
                binding.principal(),
                Some(&binding),
                Some(connect_protocol::REQUIRED_SCOPE),
            )
            .await?;
        let execute = async {
            if self
                .assurance_policy
                .get()
                .is_some_and(|policy| !policy.permits(&request, binding.principal()))
            {
                return Err(Error::forbidden());
            }
            if request.original().get("signature").is_some()
                || request.scope(Scope::Reply).is_some()
            {
                return Err(Error::new(
                    "invalid-request",
                    "HTTP command contains a transport-only assertion",
                ));
            }
            if scope_policy::required_scope(request.payload())
                .is_some_and(|scope| !binding.principal().has_scope(scope))
            {
                return Err(Error::forbidden());
            }
            CanonicalHandler::new(self.clone())
                .execute_command(&request, credential, None, None)
                .await
        };
        let mut frames = match execute.await {
            Ok(frames) => frames,
            Err(error) => {
                vec![super::canonical::problem(&request, error).map_err(|_| Error::storage())?]
            }
        };
        // Recheck the current credential/floor before producing the HTTP reply.
        // A committed effect remains discoverable by its original logical ID if
        // authority changes before this response can be returned.
        if let Err(error) = self
            .authorize_local_principal(
                binding.principal(),
                Some(&binding),
                Some(connect_protocol::REQUIRED_SCOPE),
            )
            .await
        {
            frames =
                vec![super::canonical::problem(&request, error).map_err(|_| Error::storage())?];
        }
        if frames
            .last()
            .is_some_and(|f| !matches!(f.kind(), "ack" | "error"))
        {
            frames.push(super::canonical::reply(
                &request,
                "ack",
                json!({"details":{"reply_complete":true}}),
            )?);
        }
        let status = frames
            .last()
            .filter(|f| f.kind() == "error")
            .and_then(|f| f.original()["payload"]["code"].as_u64())
            .unwrap_or(200) as u16;
        let response = ReplyBatch {
            profile: PROFILE.into(),
            request_id: request.id().into(),
            envelopes: frames.into_iter().map(|f| f.original().clone()).collect(),
        };
        if response.envelopes.is_empty()
            || response.envelopes.len() > MAX_REPLY_ENVELOPES
            || serde_json::to_vec(&response)
                .map_err(|_| Error::storage())?
                .len()
                > MAX_RESPONSE_BYTES
        {
            let error = super::canonical::problem(
                &request,
                Error::new(
                    "capacity-exceeded",
                    "Canonical HTTP reply exceeds its bounded batch",
                ),
            )
            .map_err(|_| Error::storage())?;
            return Ok((
                429,
                ReplyBatch {
                    profile: PROFILE.into(),
                    request_id: request.id().into(),
                    envelopes: vec![error.original().clone()],
                },
            ));
        }
        Ok((status, response))
    }
}
