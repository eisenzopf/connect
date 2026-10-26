//! Store-backed policy for canonical authentication and installed data commands.
use crate::{StoreCredentialBinding, StoreInvocationAuthority};
use async_trait::async_trait;
use connect_store::{
    AuthorityWait, PeerAuthorityGuard, PeerAuthorityRequest, PeerCommand, PeerConversationCommand,
    PeerDataCommand, PeerDeviceClaim, PeerOperationClaim, RuntimeRetirementSignal, Store,
};
use rvoip_auth_core::bearer::ValidatedBearer;
use rvoip_uctp::v2::{
    auth_dispatch::{AuthorityLease, DispatchError},
    auth_peer::{AuthorityLifetime, PeerAuthority, RetirementReason},
    payloads::{Device, Payload},
    Envelope, Identifier, Scope,
};
use std::{
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
const MAX_BINDINGS: usize = 1024;
struct RuntimeSignal(CancellationToken);
impl RuntimeRetirementSignal for RuntimeSignal {
    fn retire(&self) {
        self.0.cancel();
    }
}
struct Watch {
    credential: ValidatedBearer,
    request: PeerAuthorityRequest,
    signal: CancellationToken,
}
struct Registry {
    watches: Mutex<Vec<Weak<Watch>>>,
    capacity: usize,
}
pub(crate) struct Grant {
    policy: String,
    invocation_id: String,
    pub(crate) proof: PeerAuthorityGuard,
    request: PeerAuthorityRequest,
    credential: ValidatedBearer,
    native_stream_plan: Mutex<Option<native_streams::Planned>>,
    webrtc_answer: Mutex<Option<serde_json::Value>>,
    native_data: Mutex<Weak<crate::canonical_media::data::DataInvocation>>,
}
mod credential_refresh;
mod invitations;
mod native_data;
pub(crate) use native_data::{bind as bind_native_data, invocation as native_data_invocation};
mod native_streams;
mod webrtc;
pub(crate) use native_streams::bind_plan as bind_native_stream_plan;
pub(crate) use native_streams::codec as native_stream_codec;
pub(crate) use webrtc::bind_answer as bind_webrtc_answer;
/// Explicit composition only. Unsupported commands receive only Identity/key
/// proof for a bounded rejection, never a resource grant or stock Core fallback.
pub struct StorePeerAuthority {
    runtime: Weak<crate::canonical_runtime::Runtime>,
    assurance: Arc<crate::canonical_assurance::Policy>,
    store: Arc<Store>,
    id: String,
    stop: CancellationToken,
    _runtime_signal: Arc<dyn RuntimeRetirementSignal>,
    registry: Arc<Registry>,
    record_references: Option<Arc<rvoip_uctp::v2::recording::RecordReferenceRegistry>>,
}
fn device(d: &Device) -> PeerDeviceClaim {
    PeerDeviceClaim {
        id: d.id.as_str().to_owned(),
        kind: d.kind.clone(),
        platform: d.platform.clone(),
    }
}
impl StorePeerAuthority {
    pub async fn new(
        store: Arc<Store>,
        capacity: usize,
        record_references: Option<Arc<rvoip_uctp::v2::recording::RecordReferenceRegistry>>,
    ) -> Result<Arc<Self>, DispatchError> {
        Self::with_assurance(
            store,
            capacity,
            record_references,
            Arc::new(Default::default()),
            Weak::new(),
        )
        .await
    }
    pub(crate) async fn with_assurance(
        store: Arc<Store>,
        capacity: usize,
        record_references: Option<Arc<rvoip_uctp::v2::recording::RecordReferenceRegistry>>,
        assurance: Arc<crate::canonical_assurance::Policy>,
        runtime: Weak<crate::canonical_runtime::Runtime>,
    ) -> Result<Arc<Self>, DispatchError> {
        if capacity == 0 || capacity > MAX_BINDINGS {
            return Err(DispatchError::Capacity);
        }
        let stop = CancellationToken::new();
        let signal: Arc<dyn RuntimeRetirementSignal> = Arc::new(RuntimeSignal(stop.clone()));
        let setup_store = store.clone();
        let setup_signal = signal.clone();
        tokio::task::spawn_blocking(move || setup_store.register_runtime_retirement(&setup_signal))
            .await
            .map_err(|_| DispatchError::Authority)?
            .map_err(|_| DispatchError::Authority)?;
        if stop.is_cancelled() {
            return Err(DispatchError::Authority);
        }
        let registry = Arc::new(Registry {
            watches: Mutex::new(vec![]),
            capacity,
        });
        let weak = Arc::downgrade(&registry);
        let shutdown = stop.clone();
        let observed_store = store.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {biased; _=shutdown.cancelled()=>break,_=tokio::time::sleep(Duration::from_millis(100))=>{}}
                let Some(registry) = weak.upgrade() else {
                    break;
                };
                let watches = match registry.watches.lock() {
                    Ok(mut rows) => {
                        rows.retain(|w| w.strong_count() > 0);
                        rows.iter().filter_map(Weak::upgrade).collect::<Vec<_>>()
                    }
                    Err(_) => {
                        shutdown.cancel();
                        break;
                    }
                };
                drop(registry);
                if watches.is_empty() {
                    continue;
                }
                let store = observed_store.clone();
                let check = tokio::task::spawn_blocking(move || {
                    for watch in watches {
                        if StoreCredentialBinding::from_validated(&store, &watch.credential)
                            .is_err()
                            || store.check_peer_authority(&watch.request).is_err()
                        {
                            watch.signal.cancel();
                        }
                    }
                })
                .await;
                if check.is_err() {
                    shutdown.cancel();
                    break;
                }
            }
        });
        Ok(Arc::new(Self {
            runtime,
            assurance,
            store,
            id: uuid::Uuid::new_v4().to_string(),
            stop,
            _runtime_signal: signal,
            registry,
            record_references,
        }))
    }
    fn request(
        &self,
        credentials: &[&ValidatedBearer],
        d: &Device,
        e: &Envelope,
    ) -> Result<PeerAuthorityRequest, DispatchError> {
        if self.stop.is_cancelled() {
            return Err(DispatchError::Authority);
        }
        let command = match e.payload() {
            Payload::IdentityStepUpResponse(_) if e.scope(Scope::Reply).is_some() => {
                PeerCommand::Binding
            }
            Payload::AuthResponse(_)
            | Payload::AuthRefresh(_)
            | Payload::AuthKeepalive(_)
            | Payload::AuthBye(_) => {
                if [
                    Scope::Conversation,
                    Scope::Session,
                    Scope::Connection,
                    Scope::Operation,
                ]
                .iter()
                .any(|s| e.scope(*s).is_some())
                    || e.expected_revision().is_some()
                {
                    return Err(DispatchError::Unsupported);
                }
                PeerCommand::Binding
            }
            Payload::RecordingVconFetch(_) => {
                let Ok(intent) = crate::canonical_records::native::intent(e) else {
                    return self.rejection_request(credentials, d, e);
                };
                PeerCommand::RecordFetch(crate::canonical_records::resolve_selector(
                    intent,
                    self.record_references.as_deref(),
                ))
            }
            Payload::MessageSend(_)
                if e.scope(Scope::Session).is_some() || e.scope(Scope::Connection).is_some() =>
            {
                let Ok(intent) = crate::canonical_native_streams::intent(e) else {
                    return self.rejection_request(credentials, d, e);
                };
                PeerCommand::NativeStream(intent)
            }
            Payload::MessageSend(_) | Payload::MessageHistory(_) => {
                if [Scope::Session, Scope::Connection, Scope::Reply]
                    .iter()
                    .any(|s| e.scope(*s).is_some())
                    || e.expected_revision().is_some()
                {
                    return self.rejection_request(credentials, d, e);
                }
                let cid = e
                    .scope(Scope::Conversation)
                    .ok_or(DispatchError::ScopeDenied)?
                    .to_owned();
                let command = match e.payload() {
                    Payload::MessageSend(_) => PeerDataCommand::MessageSend,
                    Payload::MessageHistory(_) => PeerDataCommand::MessageHistory,
                    _ => unreachable!("message-only branch"),
                };
                PeerCommand::Data {
                    conversation_id: cid,
                    command,
                }
            }
            Payload::StreamSubscribe(_)
            | Payload::StreamUnsubscribe(_)
            | Payload::ConnectionEnd(_)
            | Payload::ConnectionUpdate(_)
            | Payload::CapabilityAdvertise(_)
            | Payload::DtmfSend(_) => {
                let Ok(intent) = crate::canonical_native_streams::intent(e) else {
                    return self.rejection_request(credentials, d, e);
                };
                PeerCommand::NativeStream(intent)
            }
            Payload::ConnectionOffer(_) => {
                let Ok(intent) = crate::canonical_native::intent(e) else {
                    return self.rejection_request(credentials, d, e);
                };
                PeerCommand::NativeConnectionOffer(intent)
            }
            Payload::SessionInvite(_)
            | Payload::SessionAccept(_)
            | Payload::SessionUpdate(_)
            | Payload::SessionEnd(_)
            | Payload::SessionReject(_)
            | Payload::SessionCancel(_) => {
                let Ok(Some(request)) = crate::canonical_sessions::command(e) else {
                    return self.rejection_request(credentials, d, e);
                };
                PeerCommand::SessionControl(request)
            }
            Payload::MessageRead(_) => {
                let Ok(Some(command)) = crate::canonical_receipts::command(e) else {
                    return self.rejection_request(credentials, d, e);
                };
                PeerCommand::ReadReceipt(command)
            }
            Payload::ConversationGet(input) => {
                if crate::canonical_subscriptions::assert_read_scopes(e).is_err() {
                    return self.rejection_request(credentials, d, e);
                }
                let Ok(query) = crate::canonical_subscriptions::snapshot_query(input) else {
                    return self.rejection_request(credentials, d, e);
                };
                PeerCommand::Snapshot {
                    conversation_id: e
                        .scope(Scope::Conversation)
                        .ok_or(DispatchError::ScopeDenied)?
                        .into(),
                    query,
                }
            }
            Payload::ConversationSubscribe(_) => {
                let Ok(command) = crate::canonical_subscriptions::command(e) else {
                    return self.rejection_request(credentials, d, e);
                };
                PeerCommand::Subscription(command)
            }
            Payload::ConversationCreate(_)
            | Payload::ConversationClose(_)
            | Payload::ConversationList(_) => {
                let Ok(Some(operation)) = crate::canonical_conversations::operation(e) else {
                    return self.rejection_request(credentials, d, e);
                };
                let candidate_cid = matches!(operation, connect_protocol::Operation::Create { .. })
                    .then(|| format!("conv_{}", uuid::Uuid::new_v4().simple()));
                PeerCommand::Conversation(PeerConversationCommand {
                    operation,
                    candidate_cid,
                })
            }
            Payload::ConversationParticipantAdd(_)
            | Payload::ConversationParticipantUpdate(_)
            | Payload::ConversationParticipantRemove(_) => {
                let Ok(Some(request)) = crate::canonical_membership::request(e) else {
                    return self.rejection_request(credentials, d, e);
                };
                PeerCommand::Membership(request)
            }
            Payload::OperationGet(_)
            | Payload::OperationCancel(_)
            | Payload::OperationReconcile(_) => {
                if e.scope(Scope::Reply).is_some() {
                    return self.rejection_request(credentials, d, e);
                }
                let Ok(Some(action)) = crate::canonical_operations::action(e) else {
                    return self.rejection_request(credentials, d, e);
                };
                PeerCommand::Operation(PeerOperationClaim {
                    action,
                    conversation_id: e.scope(Scope::Conversation).map(str::to_owned),
                    session_id: e.scope(Scope::Session).map(str::to_owned),
                    connection_id: e.scope(Scope::Connection).map(str::to_owned),
                })
            }
            Payload::SessionConsult(_)
            | Payload::SessionConsultReturn(_)
            | Payload::SessionHandoff(_)
            | Payload::ConnectionReplace(_) => {
                let Ok(Some(command)) = crate::canonical_transitions::command(e) else {
                    return self.rejection_request(credentials, d, e);
                };
                PeerCommand::Transition(command)
            }
            _ => return self.rejection_request(credentials, d, e),
        };
        let sig = &e.original()["signature"];
        Ok(PeerAuthorityRequest {
            credentials: credentials
                .iter()
                .map(|c| c.token_id.clone().ok_or(DispatchError::Authority))
                .collect::<Result<_, _>>()?,
            device: device(d),
            key_id: sig["keyid"]
                .as_str()
                .ok_or(DispatchError::Signature)?
                .into(),
            algorithm: sig["alg"].as_str().ok_or(DispatchError::Signature)?.into(),
            command,
        })
    }
    fn rejection_request(
        &self,
        credentials: &[&ValidatedBearer],
        d: &Device,
        e: &Envelope,
    ) -> Result<PeerAuthorityRequest, DispatchError> {
        let sig = &e.original()["signature"];
        Ok(PeerAuthorityRequest {
            credentials: credentials
                .iter()
                .map(|c| c.token_id.clone().ok_or(DispatchError::Authentication))
                .collect::<Result<_, _>>()?,
            device: device(d),
            key_id: sig["keyid"]
                .as_str()
                .ok_or(DispatchError::Signature)?
                .into(),
            algorithm: sig["alg"].as_str().ok_or(DispatchError::Signature)?.into(),
            command: PeerCommand::Binding,
        })
    }
    /// Protected standalone-dispatch composition; it does not create a physical
    /// peer binding or bypass normal canonical authentication on a listener.
    pub fn invocation_authority(
        self: &Arc<Self>,
        credential: ValidatedBearer,
        device: Device,
    ) -> Result<Arc<dyn rvoip_uctp::v2::auth_dispatch::CurrentAuthority>, DispatchError> {
        let identity = Identifier::try_from(credential.principal.subject.clone())?;
        Ok(Arc::new(Bound {
            policy: self.clone(),
            credential,
            device,
            identity,
        }))
    }
    async fn admit(
        &self,
        credentials: Vec<ValidatedBearer>,
        d: &Device,
        identity: &Identifier,
        e: &Envelope,
    ) -> Result<AuthorityLease, DispatchError> {
        let record_principal = credentials
            .last()
            .ok_or(DispatchError::Authority)?
            .principal
            .clone();
        let refs = credentials.iter().collect::<Vec<_>>();
        let request = self.request(&refs, d, e)?;
        if credentials
            .iter()
            .any(|c| c.principal.subject != identity.as_str())
        {
            return Err(DispatchError::Authority);
        }
        let invocation_id = e.id().to_owned();
        let policy = self.id.clone();
        let store = self.store.clone();
        let lifetime = self.stop.clone();
        let wait = AuthorityWait::new(Duration::from_secs(2));
        let _cancel = wait.cancel_on_drop();
        let lease = tokio::task::spawn_blocking(move || {
            let mut proof = store
                .admit_peer_authority(&request, &wait)
                .map_err(store_error)?;
            let mut bindings = credentials
                .iter()
                .map(|c| {
                    StoreCredentialBinding::from_validated(&store, c)
                        .map_err(|_| DispatchError::Authority)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let source_expiry = credentials
                .iter()
                .map(|c| {
                    c.principal
                        .expires_at
                        .map(|time| time.timestamp())
                        .ok_or(DispatchError::Authority)
                })
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .min()
                .ok_or(DispatchError::Authority)?;
            proof
                .limit_expiry(source_expiry)
                .map_err(|_| DispatchError::Authority)?;
            let binding = bindings.pop().ok_or(DispatchError::Authority)?;
            if bindings
                .iter()
                .any(|b| !b.principal().has_same_owner(binding.principal()))
                || !proof.current()
            {
                return Err(DispatchError::Authority);
            }
            let credential = credentials
                .last()
                .cloned()
                .ok_or(DispatchError::Authority)?;
            let end = chrono::DateTime::from_timestamp(proof.expires_at(), 0)
                .ok_or(DispatchError::Authority)?
                .min(
                    credential
                        .principal
                        .expires_at
                        .ok_or(DispatchError::Authority)?,
                );
            let duration = (end - chrono::Utc::now())
                .to_std()
                .map_err(|_| DispatchError::Authority)?;
            let grant = Arc::new(Grant {
                policy,
                invocation_id,
                proof,
                request,
                credential,
                native_stream_plan: Mutex::new(None),
                webrtc_answer: Mutex::new(None),
                native_data: Mutex::new(Weak::new()),
            });
            let current = grant.clone();
            Ok(AuthorityLease::checking(
                StoreInvocationAuthority::holding(binding, grant),
                tokio::time::Instant::now() + duration,
                move || current.proof.current() && !lifetime.is_cancelled(),
            ))
        })
        .await
        .map_err(|_| DispatchError::Authority)??;
        if matches!(e.payload(), Payload::RecordingVconFetch(_)) {
            crate::canonical_records::native::authorize(
                self.store.clone(),
                e,
                &record_principal,
                lease,
                self.stop.clone(),
                self.record_references.clone(),
            )
            .await
        } else {
            Ok(lease)
        }
    }
}
impl Drop for StorePeerAuthority {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
#[async_trait]
impl PeerAuthority for StorePeerAuthority {
    fn assurance_requirement(
        &self,
        _c: &ValidatedBearer,
        _d: &Device,
        _i: &Identifier,
        e: &Envelope,
    ) -> Result<Option<rvoip_uctp::v2::auth_peer::AssuranceRequirement>, DispatchError> {
        if e.scope(Scope::Reply).is_some() && !crate::canonical_sessions::correlated_cancel(e) {
            return Ok(None);
        }
        let requirement = self.assurance.requirement(e);
        if requirement.is_some() && e.scope(Scope::Operation).is_none() {
            return Err(DispatchError::ScopeDenied);
        }
        Ok(requirement)
    }
    async fn step_up(
        &self,
        old: &ValidatedBearer,
        next: &ValidatedBearer,
        d: &Device,
        i: &Identifier,
        requirement: &rvoip_uctp::v2::auth_peer::AssuranceRequirement,
        blocked: &Envelope,
        response: &Envelope,
    ) -> Result<AuthorityLease, DispatchError> {
        if !matches!(response.payload(), Payload::IdentityStepUpResponse(_))
            || response.scope(Scope::Reply).is_none()
            || [
                Scope::Conversation,
                Scope::Session,
                Scope::Connection,
                Scope::Operation,
            ]
            .iter()
            .any(|scope| response.scope(*scope) != blocked.scope(*scope))
            || response
                .expected_revision()
                .is_some_and(|revision| Some(revision) != blocked.expected_revision())
            || self.assurance.requirement(blocked).as_ref() != Some(requirement)
            || !self.assurance.permits(blocked, &next.principal)
        {
            return Err(DispatchError::Authority);
        }
        // Validate both enrolled credentials without projecting the successor
        // as the old native source. The original command gets its own current
        // grant after the exact physical credential transition.
        if let PeerCommand::NativeStream(intent) = self.request(&[old, next], d, blocked)?.command {
            if intent.command != connect_protocol::native_streams::Command::Dtmf
                && !next.principal.has_scope("connection.offer")
            {
                return Err(DispatchError::ScopeDenied);
            }
        }
        self.admit(vec![old.clone(), next.clone()], d, i, response)
            .await
    }
    async fn step_up_media(
        &self,
        transition: Arc<rvoip_uctp::v2::auth_dispatch::VerifiedCredentialRefresh>,
        d: &Device,
        i: &Identifier,
        blocked: &Envelope,
        held: AuthorityLease,
    ) -> Result<AuthorityLease, DispatchError> {
        let media = credential_refresh::refresh(self, transition.clone(), held).await?;
        // Do not nest new readers behind a queued writer while retaining the
        // larger transition union. New media already requires the observed
        // successor's current authority, independently of this pending command.
        drop(media);
        self.admit(vec![transition.next().clone()], d, i, blocked)
            .await
    }
    async fn identity(&self, c: &ValidatedBearer, d: &Device) -> Result<Identifier, DispatchError> {
        if self.stop.is_cancelled() {
            return Err(DispatchError::Authority);
        }
        let store = self.store.clone();
        let c = c.clone();
        let d = device(d);
        tokio::task::spawn_blocking(move || {
            let binding = StoreCredentialBinding::from_validated(&store, &c)
                .map_err(|_| DispatchError::Authority)?;
            store
                .inspect_peer_device(binding.credential_id(), &d)
                .map_err(|_| DispatchError::Authority)?;
            Identifier::try_from(c.principal.subject).map_err(DispatchError::from)
        })
        .await
        .map_err(|_| DispatchError::Authority)?
    }
    async fn acquire(
        &self,
        c: &ValidatedBearer,
        d: &Device,
        i: &Identifier,
        e: &Envelope,
    ) -> Result<AuthorityLease, DispatchError> {
        self.admit(vec![c.clone()], d, i, e).await
    }
    async fn refresh(
        &self,
        old: &ValidatedBearer,
        next: &ValidatedBearer,
        d: &Device,
        i: &Identifier,
        e: &Envelope,
    ) -> Result<AuthorityLease, DispatchError> {
        self.admit(vec![old.clone(), next.clone()], d, i, e).await
    }
    async fn refresh_media(
        &self,
        refresh: Arc<rvoip_uctp::v2::auth_dispatch::VerifiedCredentialRefresh>,
        held: AuthorityLease,
    ) -> Result<AuthorityLease, DispatchError> {
        credential_refresh::refresh(self, refresh, held).await
    }
    fn negotiation_answer(
        &self,
        c: &ValidatedBearer,
        d: &Device,
        i: &Identifier,
        offer: &Envelope,
        answer: &Envelope,
        h: &AuthorityLease,
    ) -> Result<AuthorityLease, DispatchError> {
        let grant = self.native_held(c, d, i, h)?;
        if offer.original()["payload"]["substrate"] == "websocket+webrtc" {
            self.webrtc_outgoing(grant, offer, answer)?;
        } else {
            self.native_outgoing(grant, offer, answer, true)?;
        }
        Ok(h.retained())
    }
    fn exchange_request(
        &self,
        c: &ValidatedBearer,
        d: &Device,
        i: &Identifier,
        offer: &Envelope,
        request: &Envelope,
        h: &AuthorityLease,
    ) -> Result<AuthorityLease, DispatchError> {
        let grant = self.native_held(c, d, i, h)?;
        self.native_outgoing(grant, offer, request, false)?;
        Ok(h.retained())
    }
    fn exchange_reply(
        &self,
        c: &ValidatedBearer,
        d: &Device,
        i: &Identifier,
        request: &Envelope,
        reply: &Envelope,
        h: &AuthorityLease,
    ) -> Result<rvoip_uctp::v2::auth_dispatch::ExchangeReplyAuthority, DispatchError> {
        if let Some(result) = crate::canonical_native_subscriptions::reply(
            &self.store,
            &self.stop,
            c,
            d,
            i,
            request,
            reply,
            h,
        ) {
            return result;
        }
        use rvoip_auth_core::sig9421::{PrincipalVerificationKey, ResolvedVerificationKey};
        let grant = self.native_held(c, d, i, h)?;
        self.native_scopes(&grant.request.command, request)?;
        self.native_scopes(&grant.request.command, reply)?;
        let p = &reply.original()["payload"];
        let setup = &request.original()["payload"]["substrate_setup"];
        if request.kind() != "connection.offer"
            || reply.kind() != "connection.answer"
            || reply.scope(Scope::Reply) != Some(request.id())
            || p["substrate_setup"] != *setup
            || p["substrate"] != request.original()["payload"]["substrate"]
            || p["by_participant"].as_str() != grant.proof.native_member().map(|v| v.0)
            || setup["offer_id"] != grant.invocation_id
            || reply.original()["signature"]["keyid"] != grant.request.key_id
            || reply.original()["signature"]["alg"] != grant.request.algorithm
        {
            return Err(DispatchError::Authority);
        }
        let expected=setup["streams"].as_array().ok_or(DispatchError::Authority)?.iter().map(|s|serde_json::json!({"id":s["strm_id"],"kind":s["kind"],"direction":match s["direction"].as_str(){Some("sendonly")=>"recvonly",Some("recvonly")=>"sendonly",Some("sendrecv")=>"sendrecv",_=>"inactive"},"codec":s["codec"]})).collect::<Vec<_>>();
        if p["streams_answered"] != serde_json::json!(expected) {
            return Err(DispatchError::Authority);
        }
        let current = &grant.proof.key;
        let key = PrincipalVerificationKey::from_current(
            &c.principal,
            grant.request.key_id.clone(),
            ResolvedVerificationKey {
                public_key: current.public_key.clone(),
                algorithm: current.algorithm.as_str().into(),
                not_before: chrono::DateTime::from_timestamp(current.not_before, 0),
                expires_at: chrono::DateTime::from_timestamp(current.expires_at, 0),
                revoked: false,
            },
        )
        .map_err(|_| DispatchError::Authority)?;
        Ok(rvoip_uctp::v2::auth_dispatch::ExchangeReplyAuthority {
            lease: h.retained(),
            key,
        })
    }
    fn authority_lifetime(
        &self,
        c: &ValidatedBearer,
        d: &Device,
        i: &Identifier,
        held: &AuthorityLease,
    ) -> Result<AuthorityLifetime, DispatchError> {
        let authority = held
            .authority::<StoreInvocationAuthority>()
            .ok_or(DispatchError::Authority)?;
        let grant = authority
            .guard::<Arc<Grant>>()
            .ok_or(DispatchError::Authority)?;
        if grant.policy != self.id
            || !grant.proof.current()
            || self.stop.is_cancelled()
            || !authority.binding().matches(&c.principal)
            || c.token_id.as_deref() != Some(authority.binding().credential_id())
            || c.issued_at != grant.credential.issued_at
            || grant.request.device != device(d)
            || i.as_str() != c.principal.subject
        {
            return Err(DispatchError::Authority);
        }
        let mut request = grant.request.clone();
        request.credentials = vec![authority.binding().credential_id().into()];
        request.command = PeerCommand::Binding;
        let watch = Arc::new(Watch {
            credential: c.clone(),
            request,
            signal: CancellationToken::new(),
        });
        let mut records = self
            .registry
            .watches
            .lock()
            .map_err(|_| DispatchError::Authority)?;
        records.retain(|r| r.strong_count() > 0);
        if records.len() >= self.registry.capacity {
            return Err(DispatchError::Capacity);
        }
        records.push(Arc::downgrade(&watch));
        drop(records);
        // The union lease protects refresh verification and delivery under both
        // credentials. The new binding's lifetime belongs to its own current
        // credential and key, so it does not inherit the predecessor's expiry.
        let expiry = chrono::DateTime::from_timestamp(
            grant
                .proof
                .metadata
                .actor
                .expires_at
                .min(grant.proof.key.expires_at),
            0,
        )
        .ok_or(DispatchError::Authority)?;
        let principal_expiry = c.principal.expires_at.ok_or(DispatchError::Authority)?;
        let end = expiry.min(principal_expiry);
        let duration = (end - chrono::Utc::now())
            .to_std()
            .map_err(|_| DispatchError::Authority)?;
        let reason = if end == principal_expiry
            || grant.proof.metadata.actor.expires_at < grant.proof.key.expires_at
        {
            RetirementReason::CredentialExpired
        } else {
            RetirementReason::AuthorityRetired
        };
        AuthorityLifetime::new(watch.signal.clone(), RetirementReason::AuthorityRetired)
            .with_signal(self.stop.child_token(), RetirementReason::AuthorityRetired)
            .map(|l| {
                l.with_deadline(tokio::time::Instant::now() + duration, reason)
                    .with_custody(watch)
            })
    }
}

fn store_error(error: connect_protocol::Error) -> DispatchError {
    match error.code.as_str() {
        "unauthorized" => DispatchError::Authentication,
        "capacity-exceeded" => DispatchError::Capacity,
        "authority-busy" | "authority-unavailable" => DispatchError::AuthorityUnavailable,
        "storage-unavailable" => DispatchError::StorageUnavailable,
        _ => DispatchError::Authority,
    }
}
struct Bound {
    policy: Arc<StorePeerAuthority>,
    credential: ValidatedBearer,
    device: Device,
    identity: Identifier,
}
#[async_trait]
impl rvoip_uctp::v2::auth_dispatch::CurrentAuthority for Bound {
    async fn acquire(
        &self,
        p: &rvoip_core::AuthenticatedPrincipal,
        e: &Envelope,
    ) -> Result<AuthorityLease, DispatchError> {
        let original = &self.credential.principal;
        if !p.has_same_owner(original)
            || p.scopes != original.scopes
            || p.expires_at != original.expires_at
            || p.method != original.method
        {
            return Err(DispatchError::Authority);
        }
        self.policy
            .acquire(&self.credential, &self.device, &self.identity, e)
            .await
    }
}

impl StorePeerAuthority {
    fn native_held<'a>(
        &self,
        c: &ValidatedBearer,
        d: &Device,
        i: &Identifier,
        h: &'a AuthorityLease,
    ) -> Result<&'a Grant, DispatchError> {
        let authority = h
            .authority::<StoreInvocationAuthority>()
            .ok_or(DispatchError::Authority)?;
        let grant = authority
            .guard::<Arc<Grant>>()
            .ok_or(DispatchError::Authority)?;
        if !h.current()
            || !grant.proof.current()
            || grant.policy != self.id
            || self.stop.is_cancelled()
            || !authority.binding().matches(&c.principal)
            || c.token_id.as_deref() != Some(authority.binding().credential_id())
            || c.issued_at != grant.credential.issued_at
            || grant.request.device != device(d)
            || i.as_str() != c.principal.subject
            || !matches!(
                grant.request.command,
                PeerCommand::NativeConnectionOffer(_) | PeerCommand::NativeStream(_)
            )
        {
            return Err(DispatchError::Authority);
        }
        if matches!(grant.request.command, PeerCommand::NativeStream(_))
            && grant
                .native_stream_plan
                .try_lock()
                .map_err(|_| DispatchError::Authority)?
                .is_none()
        {
            return Err(DispatchError::Authority);
        }
        Ok(grant)
    }
    fn native_scopes(&self, command: &PeerCommand, e: &Envelope) -> Result<(), DispatchError> {
        let (cid, sid, conn, op) = match command {
            PeerCommand::NativeConnectionOffer(i) => (
                i.conversation_id.as_str(),
                i.session_id.as_str(),
                i.connection_id.as_str(),
                i.operation_id.as_str(),
            ),
            PeerCommand::NativeStream(i) => (
                i.conversation_id.as_str(),
                i.session_id.as_str(),
                i.connection_id.as_str(),
                i.operation_id.as_str(),
            ),
            _ => return Err(DispatchError::Authority),
        };
        if e.scope(Scope::Conversation) != Some(cid)
            || e.scope(Scope::Session) != Some(sid)
            || e.scope(Scope::Connection) != Some(conn)
            || e.scope(Scope::Operation) != Some(op)
        {
            return Err(DispatchError::ScopeDenied);
        }
        Ok(())
    }
    fn native_outgoing(
        &self,
        grant: &Grant,
        offer: &Envelope,
        frame: &Envelope,
        answer: bool,
    ) -> Result<(), DispatchError> {
        if matches!(grant.request.command, PeerCommand::NativeStream(_)) {
            return self.native_stream_outgoing(grant, offer, frame, answer);
        }
        let PeerCommand::NativeConnectionOffer(intent) = &grant.request.command else {
            return Err(DispatchError::Authority);
        };
        if crate::canonical_native::intent(offer).as_ref() != Ok(intent)
            || offer.id() != grant.invocation_id
        {
            return Err(DispatchError::Authority);
        }
        self.native_scopes(&grant.request.command, frame)?;
        let p = &frame.original()["payload"];
        let setup = &p["substrate_setup"];
        if frame.kind()
            != if answer {
                "connection.answer"
            } else {
                "connection.offer"
            }
            || frame.scope(Scope::Reply) != if answer { Some(offer.id()) } else { None }
            || p["substrate"] != intent.offer["substrate"]
            || p["by_participant"].as_str() != grant.proof.native_member().map(|v| v.0)
            || setup["kind"] != "native-prepared/1"
            || setup["offer_id"] != offer.id()
            || setup["revision"].as_u64() != Some(intent.expected_revision.unwrap_or(0) + 1)
        {
            return Err(DispatchError::Authority);
        }
        let streams = setup["streams"]
            .as_array()
            .ok_or(DispatchError::Authority)?;
        let offered = intent.offer["streams_offered"]
            .as_array()
            .ok_or(DispatchError::Authority)?;
        if streams.is_empty() || streams.len() > 32 || streams.len() > offered.len() {
            return Err(DispatchError::Capacity);
        }
        let normal = |value: &str| match value.to_ascii_lowercase().as_str() {
            "g.711-mu" => "pcmu".to_owned(),
            "g.711-a" => "pcma".to_owned(),
            v => v.to_owned(),
        };
        let mut ids = std::collections::BTreeSet::new();
        for stream in streams {
            let id = stream["strm_id"].as_str().ok_or(DispatchError::Authority)?;
            let source = offered
                .iter()
                .find(|s| s["id"].as_str() == Some(id))
                .ok_or(DispatchError::Authority)?;
            let codec = stream["codec"]["name"]
                .as_str()
                .ok_or(DispatchError::Authority)?;
            let handle = stream["stream_local_id"]
                .as_u64()
                .ok_or(DispatchError::Authority)?;
            let peer_direction = match stream["direction"].as_str() {
                Some("sendonly") => "recvonly",
                Some("recvonly") => "sendonly",
                Some("sendrecv") => "sendrecv",
                _ => return Err(DispatchError::Authority),
            };
            if !ids.insert(id)
                || !(1..=65535).contains(&handle)
                || stream["kind"] != source["kind"]
                || source["direction"] != peer_direction
                || !source["codec_preferences"].as_array().is_some_and(|cs| {
                    cs.iter()
                        .any(|v| v.as_str().is_some_and(|v| normal(v) == normal(codec)))
                })
            {
                return Err(DispatchError::Authority);
            }
        }
        let expected = streams.iter().map(|s| {
            if answer {
                serde_json::json!({"id":s["strm_id"],"kind":s["kind"],"direction":s["direction"],"codec":s["codec"]})
            } else {
                serde_json::json!({"id":s["strm_id"],"kind":s["kind"],"direction":s["direction"],"codec_preferences":[s["codec"]["name"]]})
            }
        }).collect::<Vec<_>>();
        if p[if answer {
            "streams_answered"
        } else {
            "streams_offered"
        }] != serde_json::json!(expected)
        {
            return Err(DispatchError::Authority);
        }
        Ok(())
    }
}
