//! Administrator-owned callback policy. This contains references, never secrets.
use super::*;
mod runtime;
pub use runtime::{
    TelephoneAudioResources, TelephoneConnectionRuntimeObservation, TelephoneConnectionSnapshot,
    TelephoneMediaOrigin, TelephoneMediaRead, TelephoneNativeMediaRead,
    TelephoneReplacementAllocated, TelephoneReplacementAttempt,
    TelephoneReplacementAudienceCapture, TelephoneReplacementCommitAttempt,
    TelephoneReplacementCommitUncertain, TelephoneReplacementCommitted,
    TelephoneReplacementMediaOrigin, TelephoneReplacementMediaRead, TelephoneReplacementPhase,
    TelephoneReplacementPhaseAttempt, TelephoneReplacementPhaseCapture, TelephoneReplacementRead,
    TelephoneReplacementResourcesAttempt, TelephoneReplacementResourcesCapture,
    TelephoneReplacementSceneRead, TelephoneReplacementSubmitted, TelephoneReplacementUncertain,
    TelephoneReplacementWriteRead,
};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelephoneReplacementPolicy {
    pub route_reference: String,
    pub sip_authority: String,
    pub confirmation_digits: String,
    pub timeout_seconds: u32,
}
impl std::fmt::Debug for TelephoneReplacementPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TelephoneReplacementPolicy(redacted)")
    }
}
impl TelephoneReplacementPolicy {
    pub fn validate(&self) -> Result<()> {
        id(&self.route_reference)?;
        let authority = &self.sip_authority;
        let valid = if authority.starts_with('[') {
            authority
                .parse::<std::net::SocketAddr>()
                .is_ok_and(|a| a.port() != 0)
        } else {
            let (host, port) = match authority.split_once(':') {
                Some((host, port)) => (
                    host,
                    port.bytes().all(|b| b.is_ascii_digit())
                        && port.parse::<u16>().is_ok_and(|p| p != 0),
                ),
                None => (authority.as_str(), true),
            };
            port && !host.is_empty()
                && host.len() <= 253
                && host.split('.').all(|label| {
                    !label.is_empty()
                        && label.len() <= 63
                        && !label.starts_with('-')
                        && !label.ends_with('-')
                        && label
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                })
        };
        if !valid {
            return Err(invalid());
        }
        validate_request(&self.request("validation-only".into(), 0))
    }

    /// The host supplies policy, while the destination is resolved separately
    /// from the exact verified Endpoint under the current operation authority.
    pub fn request(
        &self,
        candidate: String,
        participant_revision: u64,
    ) -> ReplacementPreparationRequest {
        ReplacementPreparationRequest {
            candidate_connection_id: candidate,
            participant_revision,
            timeout_seconds: self.timeout_seconds,
            confirmation: ReplacementConfirmation::AnsweredDtmf {
                digits: self.confirmation_digits.clone(),
            },
            telephone_policy: Some(self.clone()),
        }
    }
}

pub(super) fn route(
    store: &Store,
    c: &Connection,
    selection: &NativeReplacementSelection,
    ctx: &crate::NativeReplacementContext,
    request: &ReplacementPreparationRequest,
) -> Result<Option<serde_json::Value>> {
    if ctx.target_transport != connect_protocol::provisioning::EndpointTransport::Telephone {
        return if request.telephone_policy.is_none() {
            Ok(None)
        } else {
            Err(invalid())
        };
    }
    let policy = request.telephone_policy.as_ref().ok_or_else(|| {
        Error::new(
            "invalid-request",
            "Telephone replacement requires administrator callback policy",
        )
    })?;
    policy.validate()?;
    if request.confirmation
        != (ReplacementConfirmation::AnsweredDtmf {
            digits: policy.confirmation_digits.clone(),
        })
        || request.timeout_seconds != policy.timeout_seconds
        || ctx.target_secret_ref.as_deref() != Some(policy.route_reference.as_str())
    {
        return Err(Error::forbidden());
    }
    let actor = selection.origin.require(store, c)?;
    actor.require("sip_dial")?;
    crate::conversations::authorized_conversation(c, &actor, &ctx.conversation_id)?;
    let address = &ctx.target_address;
    if !(9..=16).contains(&address.len())
        || !address.starts_with('+')
        || address.as_bytes()[1] == b'0'
        || !address.as_bytes()[1..].iter().all(u8::is_ascii_digit)
    {
        return Err(invalid());
    }
    let owner_current: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM identities WHERE id=?1 AND revoked=0)",
            [&ctx.target_route_owner_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if !owner_current {
        return Err(Error::forbidden());
    }
    Ok(Some(serde_json::json!({
        "endpoint_id":ctx.target_endpoint_id,"endpoint_revision":ctx.target_endpoint_revision,
        "route_owner_id":ctx.target_route_owner_id,"route_reference":policy.route_reference,
        "destination":format!("sip:{address}@{}",policy.sip_authority),
    })))
}
