//! Native engine policy derives every grant from current fenced Store state.
#[cfg(feature = "test-support")]
pub(crate) mod continuation_fixture;
pub(crate) mod credential_refresh;
pub(crate) mod data;
pub(crate) mod data_writes;
pub(crate) mod handoff;
pub(crate) mod ice;
mod lifetime;
mod native_snapshots;
mod packet_metadata;
mod packets;
mod published;
use published::Published;
pub(crate) mod preparation;
pub(crate) mod public_continuation;
#[cfg(feature = "test-support")]
mod publication_fixture;
pub(crate) mod reconciliation;
pub(crate) mod replacement;
mod sources;
#[cfg(feature = "test-support")]
pub(crate) mod video_preparation_fixture;
pub(crate) mod weak_identity;
pub(crate) mod webrtc;
use crate::{canonical_runtime::current_member, StoreInvocationAuthority};
use connect_store::{NativeMediaOrigin, NativeMediaRead, PeerConnectionProjection, Store};
use rvoip_core::{
    ids::{ConnectionId, DeviceId, EndpointId, IdentityId, SessionId},
    relationships::{ConnectionBinding, EndpointBinding, IdentityBinding, SessionAppearance},
    staged_streams::{
        ControlResult, MediaActivation, MediaAuthorityLease, MediaControl, MediaControlIntent,
        MediaOwner, MediaPolicy, MediaPurpose, MediaScope, MediaSource, PreparedMediaObservation,
        StreamControlError,
    },
    stream_lifecycle::{EndpointMediaCapabilities, MediaCapability, MediaFormat, Medium},
    AuthenticatedPrincipal, Transport,
};
use rvoip_uctp::v2::{
    auth_dispatch::AuthorizedEnvelope,
    native_negotiation::{NativeMediaProfile, NativeOfferPlan},
    Scope,
};
use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

fn adoption_pending(
    id: &ConnectionId,
    prepared: &ConnectionBinding,
    committed: bool,
    cancelled: bool,
) -> bool {
    prepared.connection() == id && committed && !cancelled
}

pub(crate) struct Connection {
    domain: Weak<Policy>,
    pub origin: Arc<NativeMediaOrigin>,
    pub principal: AuthenticatedPrincipal,
    pub profile: Arc<NativeMediaProfile>,
    pub plan: NativeOfferPlan,
    // Peer support established by the initial offer and actual later commits.
    // Declarations alone never remove rights from an existing usable binding.
    snapshots: Published<native_snapshots::Routes>,
    lifetime: Published<Option<connect_store::NativeConnectionLifetimeSnapshot>>,
    negotiated_remote: Published<EndpointMediaCapabilities>,
    activation: Published<Option<Arc<dyn MediaActivation>>>,
    binding: Published<Option<ConnectionBinding>>,
    preparation: Published<
        Option<Arc<rvoip_core::staged_streams::endpoint_preparation::AudioEndpointPreparation>>,
    >,
}
pub(crate) struct Policy {
    pub store: Arc<Store>,
    entries: Published<BTreeMap<ConnectionId, Arc<Connection>>>,
    replacement_targets:
        Published<BTreeMap<ConnectionId, Weak<crate::canonical_replacement::TargetPolicy>>>,
    telephone_targets:
        Published<BTreeMap<ConnectionId, Weak<crate::telephone_replacement::media::Policy>>>,
    initial_targets: Published<BTreeMap<ConnectionId, Weak<crate::initial_telephone::policy::Leg>>>,
    capacity: usize,
    #[cfg(feature = "test-support")]
    video_preparation_probes: Mutex<BTreeMap<String, Weak<video_preparation_fixture::Probe>>>,
    #[cfg(feature = "test-support")]
    replacement_projection_pauses: Mutex<BTreeMap<String, Arc<continuation_fixture::Pause>>>,
    #[cfg(feature = "test-support")]
    held_registry_probes: Mutex<BTreeMap<String, Arc<std::sync::atomic::AtomicUsize>>>,
    #[cfg(feature = "test-support")]
    preparation_registry_probes: Mutex<BTreeMap<String, Arc<std::sync::atomic::AtomicUsize>>>,
    #[cfg(feature = "test-support")]
    replacement_adoption_refusals: Mutex<BTreeMap<String, Arc<std::sync::atomic::AtomicBool>>>,
    #[cfg(feature = "test-support")]
    replacement_data_observer_refusals: Mutex<BTreeMap<String, Arc<std::sync::atomic::AtomicBool>>>,
    #[cfg(feature = "test-support")]
    replacement_publication_retirements: Mutex<BTreeMap<String, publication_fixture::Retirement>>,
}
/// One bounded original native invocation, never retained by a packet port.
// The protocol dispatcher owns the actual signature/physical exchange proof.
#[derive(Clone)]
pub(crate) struct Invocation {
    pub policy: Arc<Policy>,
    pub connection: Arc<Connection>,
    pub authorized: Arc<AuthorizedEnvelope>,
    // Fresh activation proof after the unique signed resource exchange releases
    // its original reader. Never installed in packet ports or idle workers.
    pub resumed: Option<Arc<connect_store::PeerAuthorityGuard>>,
    // The original owned source readers freeze authorization through this
    // bounded invocation. Engine callbacks must never reacquire SQL or grants.
    pub read: Arc<NativeMediaRead>,
    pub participant_revision: u64,
}
fn denied<T>(_: T) -> StreamControlError {
    StreamControlError::Authority
}
/// A busy metadata registry cannot prove source retirement or withdrawn rights.
/// Refuse this poll without converting transient contention into revocation.
#[track_caller]
fn metadata_contention<T>(error: std::sync::TryLockError<T>) -> StreamControlError {
    match error {
        std::sync::TryLockError::WouldBlock => {
            tracing::debug!(target: "connect_service::packet_refusal",
                caller = %std::panic::Location::caller(), "Current media metadata mutex busy");
            StreamControlError::NotReady
        }
        std::sync::TryLockError::Poisoned(_) => StreamControlError::Authority,
    }
}
/// Only transient Store contention is retryable at a packet boundary.
#[track_caller]
pub(crate) fn packet_authority_error(error: connect_protocol::Error) -> StreamControlError {
    tracing::debug!(target: "connect_service::packet_refusal",
        caller = %std::panic::Location::caller(), code = %error.code, detail = %error.message,
        "Current media Store authority refused");
    if error.code == "authority-busy" {
        StreamControlError::NotReady
    } else {
        StreamControlError::Authority
    }
}
fn codec_name(format: &MediaFormat) -> &str {
    match format {
        MediaFormat::Encoded { name, .. } => match name.as_str() {
            "pcmu" => "g.711-mu",
            "pcma" => "g.711-a",
            other => other,
        },
        MediaFormat::ReliableOrdered(rvoip_core::capability::DataProtocol::Text) => "text",
        MediaFormat::ReliableOrdered(rvoip_core::capability::DataProtocol::Json) => "json",
        MediaFormat::ReliableOrdered(rvoip_core::capability::DataProtocol::Binary) => "binary",
    }
}
fn kind(medium: Medium) -> &'static str {
    match medium {
        Medium::Audio => "audio",
        Medium::Video => "video",
        Medium::Screen => "screen",
        Medium::Data => "data",
    }
}
pub(crate) fn allows(
    view: &PeerConnectionProjection,
    medium: Medium,
    format: &MediaFormat,
) -> bool {
    view.endpoint_capabilities.iter().any(|v| v == kind(medium))
        && session_allows(view, medium, format)
}
fn session_allows(view: &PeerConnectionProjection, medium: Medium, format: &MediaFormat) -> bool {
    session_capabilities_allow(
        &view.session.session,
        &view.accepted_capabilities,
        medium,
        format,
    )
}
pub(crate) fn session_capabilities_allow(
    session: &connect_protocol::session_control::SessionView,
    accepted: &serde_json::Value,
    medium: Medium,
    format: &MediaFormat,
) -> bool {
    let kind = kind(medium);
    // Canonical Session data capability names the delivery class. Native
    // text/json/binary formats remain explicit in the negotiated endpoints.
    let codec = if medium == Medium::Data {
        "reliable-ordered"
    } else {
        codec_name(format)
    };
    let equivalent = |name: &str| {
        name.eq_ignore_ascii_case(codec)
            || (codec == "g.711-mu" && name.eq_ignore_ascii_case("pcmu"))
            || (codec == "g.711-a" && name.eq_ignore_ascii_case("pcma"))
    };
    let contains = |value: &serde_json::Value| {
        value
            .as_array()
            .is_some_and(|v| v.iter().any(|v| v.as_str().is_some_and(equivalent)))
    };
    session.media.iter().any(|v| v == kind)
        && contains(&accepted[kind])
        && session
            .capabilities_offer
            .as_ref()
            .is_some_and(|v| contains(&v[kind]))
}
pub(crate) fn filtered(
    view: &PeerConnectionProjection,
    capabilities: &EndpointMediaCapabilities,
) -> EndpointMediaCapabilities {
    let filter = |caps: &[MediaCapability]| {
        caps.iter()
            .filter_map(|cap| {
                let formats = cap
                    .formats
                    .iter()
                    .filter(|f| allows(view, cap.medium, f))
                    .cloned()
                    .collect::<Vec<_>>();
                (!formats.is_empty()).then_some(MediaCapability {
                    medium: cap.medium,
                    formats,
                })
            })
            .collect()
    };
    EndpointMediaCapabilities {
        send: filter(&capabilities.send),
        receive: filter(&capabilities.receive),
        max_streams: capabilities.max_streams,
        renegotiation: capabilities.renegotiation,
    }
}
/// Select codecs only after applying the current Session acceptance and verified
/// endpoint profile. Otherwise a disallowed preferred codec can hide an allowed
/// fallback, even though the final policy correctly refuses that preferred codec.
pub(crate) fn offer_plan(
    view: &PeerConnectionProjection,
    base: &NativeMediaProfile,
    principal: &AuthenticatedPrincipal,
) -> ControlResult<(Arc<NativeMediaProfile>, NativeOfferPlan)> {
    use rvoip_core::capability::AssuranceLevel;
    let assurance = |field: &str, default: AssuranceLevel| {
        base.capabilities()
            .get(field)
            .map(|value| serde_json::from_value(value.clone()).map_err(denied))
            .unwrap_or(Ok(default))
    };
    let profile = Arc::new(
        NativeMediaProfile::new(
            filtered(view, base.engine_capabilities()),
            assurance("identity_assurance_offered", AssuranceLevel::Anonymous)?,
            assurance("identity_assurance_required", AssuranceLevel::Anonymous)?,
        )
        .map(|profile| {
            // Filtering codecs must retain the host's explicit wire profile.
            // Negotiation still requires the peer's offer and confirmation.
            if base.capabilities()["video_feedback"]
                == rvoip_uctp::substrate::VIDEO_FEEDBACK_PROFILE
            {
                profile.with_video_feedback()
            } else {
                profile
            }
        })
        .and_then(|profile| profile.with_codec_preferences(&view.session.session.codec_preferences))
        .map_err(|error|{tracing::debug!(target:"connect_service::canonical_native",?error,"Current native profile refused");denied(error)})?,
    );
    let offer = serde_json::from_value(view.offer.clone()).map_err(denied)?;
    let plan = NativeOfferPlan::parse(&offer, &profile, principal).map_err(|error|{tracing::debug!(target:"connect_service::canonical_native",?error,"Current native offer plan refused");denied(error)})?;
    Ok((profile, plan))
}
impl Connection {
    /// Historical confirmed formats for existing packet IO. Current Store
    /// ceilings and the broker declaration still authorize future choices.
    pub(crate) fn remember_bindings(
        &self,
        bindings: &[rvoip_core::stream_lifecycle::ReadyMediaBinding],
    ) {
        use rvoip_core::stream_lifecycle::StreamDirection;
        let mut known = self
            .negotiated_remote
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        for binding in bindings {
            let stream = &binding.negotiated;
            let add = |set: &mut Vec<MediaCapability>| {
                if let Some(cap) = set.iter_mut().find(|c| c.medium == stream.requested.medium) {
                    if !cap.formats.contains(&stream.format) {
                        cap.formats.push(stream.format.clone());
                    }
                } else {
                    set.push(MediaCapability {
                        medium: stream.requested.medium,
                        formats: vec![stream.format.clone()],
                    });
                }
            };
            if matches!(
                stream.requested.direction,
                StreamDirection::Send | StreamDirection::Both
            ) {
                add(&mut known.receive);
            }
            if matches!(
                stream.requested.direction,
                StreamDirection::Receive | StreamDirection::Both
            ) {
                add(&mut known.send);
            }
        }
    }
    pub(crate) fn activation(&self) -> ControlResult<Option<Arc<dyn MediaActivation>>> {
        // Immutable metadata publication is independent of control-side writes.
        Ok(self.activation.snapshot().map_err(denied)?.as_ref().clone())
    }

    pub fn set_activation(&self, value: Arc<dyn MediaActivation>) -> ControlResult<()> {
        let mut activation = self.activation.lock().map_err(denied)?;
        if activation
            .as_ref()
            .is_some_and(|a| a.confirmed_generation().is_some())
        {
            return Err(StreamControlError::Authority);
        }
        *activation = Some(value);
        Ok(())
    }
    pub(crate) fn owner(
        &self,
        view: &PeerConnectionProjection,
        principal: &AuthenticatedPrincipal,
        route: bool,
    ) -> ControlResult<MediaOwner> {
        self.owner_with_remote(view, principal, route, None)
    }
    fn owner_with_remote(
        &self,
        view: &PeerConnectionProjection,
        principal: &AuthenticatedPrincipal,
        route: bool,
        negotiation: Option<&EndpointMediaCapabilities>,
    ) -> ControlResult<MediaOwner> {
        if !rvoip_core::staged_streams::reliable_data::same_credential(&self.principal, principal)
            || view.runtime_connection_id != self.origin.runtime_connection_id()
            || view.connection.generation != self.origin.generation()
        {
            return Err(StreamControlError::Authority);
        }
        let member = current_member(principal, &view.session).map_err(denied)?;
        let binding = IdentityBinding::from_authenticated(
            principal,
            IdentityId::from_string(view.session.actor.identity_id.clone()),
            chrono::Utc::now(),
        )
        .map_err(denied)?;
        let mut endpoint = EndpointBinding::provision(
            binding.clone(),
            EndpointId::from_string(view.connection.endpoint_id.clone()),
            DeviceId::from_string(view.device_id.clone()),
            crate::canonical_runtime::core_transport(&view.endpoint_transport).map_err(denied)?,
            view.endpoint_address.clone(),
        )
        .map_err(denied)?;
        // This label records the trusted resolver. The real verified endpoint,
        // Device/key and lifetime were just checked under the retained Store fence.
        endpoint
            .verify(&binding, "connect-store".into(), chrono::Utc::now())
            .map_err(denied)?;
        let presence = SessionAppearance::admit(
            &member,
            SessionId::from_string(view.connection.session_id.clone()),
            Some(crate::canonical_runtime::participant_role(
                &view.presence_role,
            )),
        )
        .map_err(denied)?;
        let saved = self.binding.snapshot().map_err(denied)?;
        let connection = if let Some(binding) = saved.as_ref() {
            binding.clone()
        } else {
            // Initial binding publication is a state change. Established media
            // never enters this writer; later generations adopt their actual
            // Core binding before publication.
            if view.connection.generation != 1 {
                return Err(StreamControlError::Authority);
            }
            let mut saved = self.binding.try_lock().map_err(metadata_contention)?;
            if let Some(binding) = saved.as_ref() {
                binding.clone()
            } else {
                let binding = ConnectionBinding::prepare(
                    &member,
                    &presence,
                    &endpoint,
                    ConnectionId::from_string(view.runtime_connection_id.clone()),
                )
                .map_err(denied)?;
                *saved = Some(binding.clone());
                binding
            }
        };
        connection
            .require_current(&member, &presence)
            .map_err(denied)?;
        if connection.endpoint() != endpoint.endpoint()
            || connection.device() != endpoint.device()
            || connection.generation() != view.connection.generation
        {
            return Err(StreamControlError::Authority);
        }
        let local = filtered(view, self.profile.engine_capabilities());
        let remote = match negotiation {
            Some(remote) => filtered(view, remote),
            None => {
                let remote = self.negotiated_remote.snapshot().map_err(denied)?;
                filtered(view, &remote)
            }
        };
        // Current accepted capabilities authorize media kinds independently
        // of the initial selection. Actual ports and the exact held command
        // still gate every stream; adding a kind requires real negotiation.
        let intersection = |local: &[MediaCapability], remote: &[MediaCapability]| {
            local
                .iter()
                .filter(|cap| {
                    remote.iter().any(|peer| {
                        peer.medium == cap.medium
                            && cap
                                .formats
                                .iter()
                                .any(|format| peer.formats.contains(format))
                    })
                })
                .map(|cap| cap.medium)
                .collect::<BTreeSet<_>>()
        };
        let send_media = intersection(&local.send, &remote.receive);
        let receive_media = intersection(&local.receive, &remote.send);
        Ok(MediaOwner {
            actor: member.clone(),
            member,
            presence,
            endpoint,
            connection,
            may_control: true,
            send_media,
            receive_media,
            local,
            remote,
            may_route_source: route,
        })
    }
}
impl Policy {
    /// One exact pre-Core negative injection; it cannot grant or run media.
    #[cfg(feature = "test-support")]
    pub(crate) fn refuse_replacement_adoption_once_for_test(
        &self,
        record: &str,
    ) -> connect_protocol::Result<Arc<std::sync::atomic::AtomicBool>> {
        let mut refusals = self
            .replacement_adoption_refusals
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?;
        if refusals.contains_key(record) {
            return Err(connect_protocol::Error::forbidden());
        }
        let armed = Arc::new(std::sync::atomic::AtomicBool::new(true));
        refusals.insert(record.into(), armed.clone());
        Ok(armed)
    }
    /// Make the exact pending Data writer fail its real scope validation.
    #[cfg(feature = "test-support")]
    pub(crate) fn refuse_data_observer_once_for_test(
        &self,
        record: &str,
    ) -> connect_protocol::Result<Arc<std::sync::atomic::AtomicBool>> {
        let mut refusals = self
            .replacement_data_observer_refusals
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?;
        if refusals.contains_key(record) {
            return Err(connect_protocol::Error::forbidden());
        }
        let armed = Arc::new(std::sync::atomic::AtomicBool::new(true));
        refusals.insert(record.into(), armed.clone());
        Ok(armed)
    }
    #[cfg(feature = "test-support")]
    pub(crate) fn arm_held_registry_probe_for_test(
        &self,
        operation: &str,
    ) -> connect_protocol::Result<Arc<std::sync::atomic::AtomicUsize>> {
        let mut probes = self
            .held_registry_probes
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?;
        if probes.contains_key(operation) {
            return Err(connect_protocol::Error::forbidden());
        }
        // 0=armed, 1=entered, 2=legacy refusal and held-owner success, 3=failed comparison.
        let state = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        probes.insert(operation.into(), state.clone());
        Ok(state)
    }
    #[cfg(feature = "test-support")]
    pub(crate) fn probe_held_registry_for_test(
        &self,
        operation: &str,
        probe: impl FnOnce() -> (bool, ControlResult<MediaOwner>),
    ) -> Option<ControlResult<MediaOwner>> {
        use std::sync::atomic::Ordering;
        let state = self.held_registry_probes.lock().ok()?.remove(operation)?;
        state.store(1, Ordering::Release);
        let Ok(_registry) = self.entries.lock() else {
            return Some(Err(StreamControlError::Authority));
        };
        let (legacy_refused, current) = probe();
        state.store(
            if legacy_refused && current.is_ok() {
                2
            } else {
                3
            },
            Ordering::Release,
        );
        Some(current)
    }
    /// Exercise the original initial-mute dispatch while all packet target
    /// registries are genuinely excluded. The once-only gate itself is not
    /// replayed; only its original synchronous policy admission is entered.
    #[cfg(feature = "test-support")]
    pub(crate) fn probe_start_dispatch_for_test(
        &self,
        operation: &str,
        connection: &ConnectionId,
        probe: impl FnOnce() -> (bool, bool, ControlResult<MediaControl>),
    ) -> Option<ControlResult<MediaControl>> {
        use std::sync::atomic::Ordering;
        let state = self
            .held_registry_probes
            .lock()
            .ok()?
            .remove(&format!("start-dispatch:{operation}"))?;
        state.store(1, Ordering::Release);
        let _initial = self
            .initial_targets
            .lock()
            .expect("original initial registry");
        let _telephone = self
            .telephone_targets
            .lock()
            .expect("original telephone registry");
        let _replacement = self
            .replacement_targets
            .lock()
            .expect("original replacement registry");
        let original_lookups_busy = self.initial_targets.try_lock().is_err()
            && self.telephone_targets.try_lock().is_err()
            && self.replacement_targets.try_lock().is_err();
        let published_lookups_current = self.initial_target(connection).is_ok()
            && self.telephone_target(connection).is_ok()
            && self
                .replacement_target(connection)
                .is_ok_and(|target| target.is_some());
        let (foreign_refused, expired_refused, current) = probe();
        eprintln!("original start dispatch: lookups_busy={original_lookups_busy} foreign_refused={foreign_refused} expired_refused={expired_refused} held_ok={}", current.is_ok());
        state.store(
            if original_lookups_busy
                && published_lookups_current
                && foreign_refused
                && expired_refused
                && current.is_ok()
            {
                2
            } else {
                3
            },
            Ordering::Release,
        );
        Some(current)
    }
    /// Exercise actual post-publication attachment with this original registry
    /// excluded. No mock grant, media packet or replacement attempt is created.
    #[cfg(feature = "test-support")]
    pub(crate) fn probe_lifecycle_registry_for_test(
        &self,
        operation: &str,
        id: &ConnectionId,
        activate: impl FnOnce() -> connect_protocol::Result<()>,
    ) -> Option<connect_protocol::Result<()>> {
        use std::sync::atomic::Ordering;
        let state = self
            .held_registry_probes
            .lock()
            .ok()?
            .remove(&format!("lifecycle:{operation}"))?;
        state.store(1, Ordering::Release);
        let Ok(registry) = self.entries.lock() else {
            return Some(Err(connect_protocol::Error::forbidden()));
        };
        let original_installed = registry.contains_key(id);
        let writer_busy_and_readable = self.entries.try_lock().is_err() && self.entry(id).is_ok();
        let result = activate();
        state.store(
            if original_installed && writer_busy_and_readable && result.is_ok() {
                2
            } else {
                3
            },
            Ordering::Release,
        );
        Some(result)
    }
    #[cfg(feature = "test-support")]
    pub(crate) fn with_registry_for_test(
        &self,
        hold: impl FnOnce(),
    ) -> connect_protocol::Result<()> {
        let _entries = self
            .entries
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?;
        hold();
        Ok(())
    }
    #[cfg(feature = "test-support")]
    pub(crate) fn with_activation_for_test(
        &self,
        connection_id: &str,
        hold: impl FnOnce(),
    ) -> connect_protocol::Result<()> {
        let entry = self
            .entries
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?
            .values()
            .find(|e| e.origin.connection_id() == connection_id)
            .cloned()
            .ok_or_else(connect_protocol::Error::forbidden)?;
        let _activation = entry
            .activation
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?;
        hold();
        Ok(())
    }
    #[cfg(feature = "test-support")]
    pub(crate) fn with_native_binding_for_test(
        &self,
        connection_id: &str,
        hold: impl FnOnce(),
    ) -> connect_protocol::Result<()> {
        let entry = self
            .entries
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?
            .values()
            .find(|e| e.origin.connection_id() == connection_id)
            .cloned()
            .ok_or_else(connect_protocol::Error::forbidden)?;
        let _binding = entry
            .binding
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?;
        hold();
        Ok(())
    }
    pub(crate) fn register_initial_telephone(
        &self,
        leg: &Arc<crate::initial_telephone::policy::Leg>,
    ) -> ControlResult<()> {
        if !std::ptr::eq(self, Arc::as_ptr(&leg.runtime()?.media))
            || self
                .entries
                .try_lock()
                .map_err(denied)?
                .contains_key(&leg.connection())
            || self.telephone_target(&leg.connection())?.is_some()
            || self.replacement_target(&leg.connection())?.is_some()
        {
            return Err(StreamControlError::Authority);
        }
        let mut targets = self.initial_targets.try_lock().map_err(denied)?;
        targets.retain(|_, t| t.strong_count() > 0);
        if targets.len() >= self.capacity || targets.contains_key(&leg.connection()) {
            return Err(StreamControlError::Authority);
        }
        targets.insert(leg.connection(), Arc::downgrade(leg));
        Ok(())
    }
    pub(crate) fn remove_initial_telephone(&self, id: &ConnectionId) {
        if let Ok(mut targets) = self.initial_targets.lock() {
            targets.remove(id);
        }
    }
    pub(crate) fn initial_dtmf_targets(
        &self,
        source: &connect_store::NativeMediaOrigin,
    ) -> ControlResult<Vec<Arc<crate::initial_telephone::policy::Leg>>> {
        let mut targets = self.initial_targets.try_lock().map_err(denied)?;
        targets.retain(|_, target| target.strong_count() > 0);
        Ok(targets
            .values()
            .filter_map(Weak::upgrade)
            .filter(|leg| {
                let s = leg.signal.origin().source();
                leg.dtmf_registration.get().is_some()
                    && !leg.cancel.is_cancelled()
                    && s.conversation_id == source.conversation_id()
                    && s.session_id == source.session_id()
                    && s.target.participant_id != source.participant_id()
            })
            .collect())
    }
    pub(crate) fn established_dtmf_targets(
        &self,
        source: &connect_store::NativeMediaOrigin,
    ) -> ControlResult<Vec<Arc<crate::telephone_replacement::media::Policy>>> {
        let mut targets = self.telephone_targets.try_lock().map_err(denied)?;
        targets.retain(|_, target| target.strong_count() > 0);
        Ok(targets
            .values()
            .filter_map(Weak::upgrade)
            .filter(|policy| {
                policy.established_signal_current()
                    && policy.established_origin().is_some_and(|origin| {
                        let s = origin.snapshot();
                        s.conversation_id == source.conversation_id()
                            && s.session_id == source.session_id()
                            && s.participant_id != source.participant_id()
                    })
            })
            .collect())
    }
    pub(super) fn initial_target(
        &self,
        id: &ConnectionId,
    ) -> ControlResult<Option<Arc<crate::initial_telephone::policy::Leg>>> {
        Ok(self
            .initial_targets
            .snapshot()
            .map_err(denied)?
            .get(id)
            .and_then(Weak::upgrade))
    }
    pub(crate) fn remove_telephone_target(&self, id: &ConnectionId) {
        if let Ok(mut targets) = self.telephone_targets.lock() {
            targets.remove(id);
        }
    }
    pub(crate) fn register_telephone_target(
        &self,
        target: &Arc<crate::telephone_replacement::media::Policy>,
    ) -> ControlResult<()> {
        if !std::ptr::eq(self, Arc::as_ptr(&target.base()?)) {
            return Err(StreamControlError::Authority);
        }
        let id = target.connection()?;
        if self.entries.try_lock().map_err(denied)?.contains_key(&id)
            || self.replacement_target(&id)?.is_some()
        {
            return Err(StreamControlError::Authority);
        }
        let mut targets = self.telephone_targets.try_lock().map_err(denied)?;
        targets.retain(|_, t| t.strong_count() > 0);
        if targets.len() >= self.capacity || targets.contains_key(&id) {
            return Err(StreamControlError::Authority);
        }
        targets.insert(id, Arc::downgrade(target));
        Ok(())
    }
    fn telephone_target(
        &self,
        id: &ConnectionId,
    ) -> ControlResult<Option<Arc<crate::telephone_replacement::media::Policy>>> {
        Ok(self
            .telephone_targets
            .snapshot()
            .map_err(denied)?
            .get(id)
            .and_then(Weak::upgrade))
    }
    pub(crate) fn register_replacement_target(
        &self,
        target: &Arc<crate::canonical_replacement::TargetPolicy>,
    ) -> ControlResult<()> {
        if !std::ptr::eq(self, Arc::as_ptr(&target.base))
            || self
                .entries
                .try_lock()
                .map_err(denied)?
                .contains_key(target.pending.binding().connection())
        {
            return Err(StreamControlError::Authority);
        }
        let mut targets = self.replacement_targets.try_lock().map_err(denied)?;
        targets.retain(|_, t| t.strong_count() > 0);
        if targets.len() >= self.capacity
            || targets.contains_key(target.pending.binding().connection())
        {
            return Err(StreamControlError::Authority);
        }
        targets.insert(
            target.pending.binding().connection().clone(),
            Arc::downgrade(target),
        );
        Ok(())
    }
    fn replacement_target(
        &self,
        id: &ConnectionId,
    ) -> ControlResult<Option<Arc<crate::canonical_replacement::TargetPolicy>>> {
        Ok(self
            .replacement_targets
            .snapshot()
            .map_err(denied)?
            .get(id)
            .and_then(Weak::upgrade)
            // Dispatch state only; the ordinary path still takes fresh Store
            // and actual resource admission. This flag never grants media.
            .filter(|target| {
                !target
                    .public_continuation
                    .get()
                    .is_some_and(|continuation| continuation.ordinary())
            }))
    }
    pub fn new(store: Arc<Store>, capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            store,
            entries: Published::new(BTreeMap::new()),
            replacement_targets: Published::new(BTreeMap::new()),
            telephone_targets: Published::new(BTreeMap::new()),
            initial_targets: Published::new(BTreeMap::new()),
            capacity,
            #[cfg(feature = "test-support")]
            replacement_projection_pauses: Default::default(),
            #[cfg(feature = "test-support")]
            replacement_adoption_refusals: Default::default(),
            #[cfg(feature = "test-support")]
            held_registry_probes: Default::default(),
            #[cfg(feature = "test-support")]
            video_preparation_probes: Default::default(),
            #[cfg(feature = "test-support")]
            preparation_registry_probes: Default::default(),
            #[cfg(feature = "test-support")]
            replacement_data_observer_refusals: Default::default(),
            #[cfg(feature = "test-support")]
            replacement_publication_retirements: Default::default(),
        })
    }
    pub fn register(
        self: &Arc<Self>,
        origin: Arc<NativeMediaOrigin>,
        principal: AuthenticatedPrincipal,
        base: &NativeMediaProfile,
        read: &NativeMediaRead,
    ) -> ControlResult<Arc<Connection>> {
        if read.target().runtime_connection_id != origin.runtime_connection_id()
            || read.target().connection.connection_id != origin.connection_id()
            || read.target().connection.participant_id != origin.participant_id()
            || read.target().connection.membership_epoch != origin.membership_epoch()
            || read.target().connection.generation != origin.generation()
        {
            return Err(StreamControlError::Authority);
        }
        let (profile, plan) = offer_plan(read.target(), base, &principal)?;
        if !read.current() {
            return Err(StreamControlError::Authority);
        }
        let id = ConnectionId::from_string(origin.runtime_connection_id().to_owned());
        let mut entries = self.entries.lock().map_err(denied)?;
        if let Some(previous) = entries.get(&id) {
            if !rvoip_core::staged_streams::reliable_data::same_credential(
                &previous.principal,
                &principal,
            ) || previous.plan.selected() != plan.selected()
                || previous.plan.rejected() != plan.rejected()
            {
                return Err(StreamControlError::Authority);
            }
            return Ok(previous.clone());
        }
        if entries.len() >= self.capacity {
            return Err(StreamControlError::Authority);
        }
        let negotiated_remote = Published::new(plan.remote_capabilities().clone());
        let connection = Arc::new(Connection {
            domain: Arc::downgrade(self),
            origin,
            principal,
            profile,
            plan,
            snapshots: Default::default(),
            lifetime: Default::default(),
            negotiated_remote,
            activation: Published::new(None),
            binding: Published::new(None),
            preparation: Published::new(None),
        });
        connection
            .prepare_snapshot_route(&self.store, None)
            .map_err(packet_authority_error)?;
        entries.insert(id, connection.clone());
        Ok(connection)
    }
    pub(crate) fn origin(&self, id: &ConnectionId) -> Option<Arc<NativeMediaOrigin>> {
        self.entries
            .snapshot()
            .ok()?
            .get(id)
            .map(|e| e.origin.clone())
    }
    /// Resource-bearing entries and Weak target registries, never permissions.
    /// Each try_lock releases before another registry or serialization is entered.
    pub(crate) fn retention_counts(&self) -> serde_json::Value {
        let entries = self.entries.try_lock().ok().map(|entries| entries.len());
        let replacement = self.replacement_targets.try_lock().ok().map(|entries| {
            (
                entries.len(),
                entries.values().filter(|p| p.strong_count() > 0).count(),
            )
        });
        let telephone = self.telephone_targets.try_lock().ok().map(|entries| {
            (
                entries.len(),
                entries.values().filter(|p| p.strong_count() > 0).count(),
            )
        });
        let initial = self.initial_targets.try_lock().ok().map(|entries| {
            (
                entries.len(),
                entries.values().filter(|p| p.strong_count() > 0).count(),
            )
        });
        serde_json::json!({"connection_strong_entries": entries,
            "replacement_weak_entries_and_with_strong_owner": replacement,
            "telephone_weak_entries_and_with_strong_owner": telephone,
            "initial_weak_entries_and_with_strong_owner": initial})
    }
    pub(crate) fn remove(&self, id: &ConnectionId) {
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
    }
    pub(crate) fn assistant_source(
        &self,
        public_id: &str,
        generation: u64,
    ) -> connect_protocol::Result<Arc<Connection>> {
        let entries = self
            .entries
            .snapshot()
            .map_err(|_| connect_protocol::Error::storage())?;
        let mut candidates = entries.values().filter(|entry| {
            entry.origin.connection_id() == public_id && entry.origin.generation() == generation
        });
        let source = candidates.next().cloned().ok_or_else(|| {
            connect_protocol::Error::new("not-ready", "Assistant native source is not registered")
        })?;
        if candidates.next().is_some() {
            return Err(connect_protocol::Error::forbidden());
        }
        Ok(source)
    }
    pub(crate) fn entry(&self, id: &ConnectionId) -> ControlResult<Arc<Connection>> {
        self.entries
            .snapshot()
            .map_err(denied)?
            .get(id)
            .cloned()
            .ok_or(StreamControlError::Authority)
    }
    /// A known replacement can be journaled before its ordinary media policy
    /// is adopted. Defer metadata preparation for that exact committed target;
    /// the caller retains its original attempt, authorization and deadline.
    /// This lookup never lends provisional packet or control authority.
    fn stream_entry(&self, id: &ConnectionId) -> ControlResult<Arc<Connection>> {
        // Adoption takes these registries in the same order. Keep both views
        // stable so removal of the provisional entry cannot hide an adoption.
        let entries = self.entries.try_lock().map_err(metadata_contention)?;
        if let Some(entry) = entries.get(id) {
            return Ok(entry.clone());
        }
        let targets = self
            .replacement_targets
            .try_lock()
            .map_err(metadata_contention)?;
        let adopting = targets
            .get(id)
            .and_then(Weak::upgrade)
            .is_some_and(|target| {
                target.root.upgrade().is_some_and(|root| {
                    adoption_pending(
                        id,
                        target.pending.binding(),
                        root.handle.is_committed(),
                        root.handle.cancellation().is_cancelled(),
                    )
                })
            });
        if adopting {
            Err(StreamControlError::NotReady)
        } else {
            Err(StreamControlError::Authority)
        }
    }
    fn scope(&self, scope: &MediaScope<'_>, entry: &Connection) -> ControlResult<()> {
        if !entry
            .domain
            .upgrade()
            .is_some_and(|policy| std::ptr::eq(self, Arc::as_ptr(&policy)))
            || scope.conversation.as_str() != entry.origin.conversation_id()
            || scope.session.as_str() != entry.origin.session_id()
            || scope.connection.as_str() != entry.origin.runtime_connection_id()
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                scope.principal,
                &entry.principal,
            )
        {
            return Err(StreamControlError::Authority);
        }
        Ok(())
    }
    fn held_read(
        &self,
        i: &Invocation,
        source: Option<&NativeMediaOrigin>,
    ) -> ControlResult<Arc<NativeMediaRead>> {
        let a = &i.authorized;
        let proof = i
            .resumed
            .as_deref()
            .or_else(|| {
                a.authority::<StoreInvocationAuthority>()
                    .and_then(|a| a.peer_guard())
            })
            .ok_or(StreamControlError::Authority)?;
        if !i
            .connection
            .domain
            .upgrade()
            .is_some_and(|policy| std::ptr::eq(self, Arc::as_ptr(&policy)))
            || !std::ptr::eq(self, Arc::as_ptr(&i.policy))
            || a.envelope().kind() != "connection.offer"
            || a.envelope().scope(Scope::Conversation)
                != Some(i.connection.origin.conversation_id())
            || a.envelope().scope(Scope::Session) != Some(i.connection.origin.session_id())
            || a.envelope().scope(Scope::Connection) != Some(i.connection.origin.connection_id())
        {
            return Err(StreamControlError::Authority);
        }
        if source.is_some() || !proof.current() || !i.read.current() {
            return Err(StreamControlError::Authority);
        }
        // Original source authorization is held by these exact readers. A
        // logical cancellation is rechecked outside engine fences before
        // admission and by the final durable commit; every packet still takes
        // a fresh attempt/source read. This view grants no idle publication.
        let read = i.read.clone();
        Ok(read)
    }
}
type ContinuationPublications =
    [Option<rvoip_core::orchestrator::participant_control::CurrentReplacementPublication>; 2];
struct Fresh {
    owner: MediaOwner,
    _read: Arc<NativeMediaRead>,
    weak: std::sync::Weak<dyn Any + Send + Sync>,
    // Active IO only. Queued packets can retain only the weak reference above.
    _source: Option<Arc<NativeMediaRead>>,
    // Exact target/source slots retain actual IO publications in that order.
    // The ordinary path needs no continuation heap allocation.
    _continuations: ContinuationPublications,
}
impl Fresh {
    fn new(owner: MediaOwner, read: NativeMediaRead, source: Option<Arc<NativeMediaRead>>) -> Self {
        let read = Arc::new(read);
        // Keep one weak anchor while any IO for this original packet is live.
        // A short-lived child must not hide an older, still-owned source read.
        let anchor = source.as_ref().unwrap_or(&read);
        let erased: Arc<dyn Any + Send + Sync> = anchor.clone();
        Self {
            owner,
            _read: read,
            weak: Arc::downgrade(&erased),
            _source: source,
            _continuations: [None, None],
        }
    }
    fn with_continuations(mut self, current: ContinuationPublications) -> Self {
        self._continuations = current;
        self
    }
}
// Queue metadata only. The Fresh lease still retains the actual IO fences.
struct PacketEpoch(connect_store::NativeMediaSnapshotEpoch);
impl rvoip_core::staged_streams::PacketEpoch for PacketEpoch {
    fn current(&self) -> bool {
        self.0.current()
    }
}
impl MediaAuthorityLease for Fresh {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
    fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        Some(
            chrono::DateTime::from_timestamp(self._read.expires_at(), 0)
                .unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC),
        )
    }
    fn packet_epoch(&self) -> Option<Arc<dyn rvoip_core::staged_streams::PacketEpoch>> {
        self._read.packet_epoch().map(|epoch| {
            Arc::new(PacketEpoch(epoch)) as Arc<dyn rvoip_core::staged_streams::PacketEpoch>
        })
    }
    fn packet_context(&self) -> Option<&(dyn Any + Send + Sync)> {
        Some(&self.weak)
    }
}
struct Held {
    invocation: Arc<Invocation>,
    owner: MediaOwner,
}
impl Held {
    fn base(&self) -> ControlResult<MediaOwner> {
        let i = &self.invocation;
        let read = i.policy.held_read(i, None)?;
        i.connection
            .owner(read.target(), i.authorized.principal(), false)
            .map_err(|error| {tracing::debug!(target:"connect_service::canonical_native",?error,"Held native owner refused");error})
    }
}
impl MediaAuthorityLease for Held {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
    fn control_owner(&self) -> ControlResult<Box<MediaOwner>> {
        let owner = self.base()?;
        match self
            .invocation
            .connection
            .activation()?
            .filter(|a| a.confirmed_generation().is_some())
        {
            Some(activation) => Ok(Box::new(activation.current(owner).map_err(|error| {tracing::debug!(target:"connect_service::canonical_native",?error,"Held native activation refused");error})?)),
            None => Ok(Box::new(owner)),
        }
    }
    fn control_prepared(
        &self,
        prepared: &Arc<PreparedMediaObservation>,
    ) -> ControlResult<Box<MediaOwner>> {
        let owner = self.base()?;

        let activation = self
            .invocation
            .connection
            .activation()?
            .ok_or(StreamControlError::NotReady)?;
        Ok(Box::new(activation.prepare(prepared, owner).map_err(|error| {tracing::debug!(target:"connect_service::canonical_native",?error,"Held native resource preparation refused");error})?))
    }
}
impl MediaPolicy for Policy {
    fn current_transport_update(
        &self,
        scope: &MediaScope<'_>,
        expected: u64,
        held: &(dyn Any + Send + Sync),
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        held.downcast_ref::<ice::Invocation>()
            .ok_or(StreamControlError::Authority)?
            .current(self, scope, expected)
    }
    fn current_device_replacement_signal(
        &self,
        scope: &rvoip_core::orchestrator::participant_control::DeviceReplacementSignalScope<'_>,
        held: Option<&(dyn Any + Send + Sync)>,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        if let Some(i) = held.and_then(|h| {
            h.downcast_ref::<crate::telephone_replacement::continuation::Invocation>()
        }) {
            if !std::ptr::eq(self, Arc::as_ptr(&i.runtime.media)) {
                return Err(StreamControlError::Authority);
            }
            return i.signal(scope);
        }
        let i = held
            .and_then(|h| {
                h.downcast_ref::<crate::canonical_replacement::continuation::Invocation>()
            })
            .ok_or(StreamControlError::Authority)?;
        if !std::ptr::eq(self, Arc::as_ptr(&i.policy.base)) {
            return Err(StreamControlError::Authority);
        }
        i.signal(scope)
    }

    fn current_audio_preparation(
        &self,
        scope: &MediaScope<'_>,
        purpose: &rvoip_core::staged_streams::local_audio::LocalAudioPurpose,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        if let Some(target) = self.telephone_target(scope.connection)? {
            return target.current_audio_preparation(scope, purpose);
        }
        if let Some(target) = self.replacement_target(scope.connection)? {
            return target.current_audio_preparation(scope, purpose);
        }
        self.preparation_authority(scope, purpose, false)
    }
    fn current_local_audio(
        &self,
        scope: &MediaScope<'_>,
        purpose: &rvoip_core::staged_streams::local_audio::LocalAudioPurpose,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        if let Some(target) = self.telephone_target(scope.connection)? {
            return target.current_local_audio(scope, purpose);
        }
        if let Some(target) = self.replacement_target(scope.connection)? {
            return target.current_local_audio(scope, purpose);
        }
        self.preparation_authority(scope, purpose, true)
    }

    fn current_data(
        &self,
        scope: &MediaScope<'_>,
        data: &rvoip_core::staged_streams::reliable_data::StagedData,
        held: Option<&(dyn Any + Send + Sync)>,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.data_authority(scope, data, held)
    }
    fn current(&self, scope: &MediaScope<'_>) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.packet_current(scope, None, false)
    }
    fn current_original_ingress(
        &self,
        scope: &MediaScope<'_>,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        if !matches!(scope.purpose, MediaPurpose::Publish) {
            return Err(StreamControlError::Authority);
        }
        self.packet_current_inner(scope, None, true, true)
    }
    fn current_queued(
        &self,
        scope: &MediaScope<'_>,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        if !matches!(
            scope.purpose,
            MediaPurpose::Publish | MediaPurpose::Receive { .. }
        ) {
            return Err(StreamControlError::Authority);
        }
        self.packet_current(scope, None, true)
    }
    fn current_packet(
        &self,
        scope: &MediaScope<'_>,
        context: &(dyn Any + Send + Sync),
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        if !matches!(
            scope.purpose,
            MediaPurpose::Publish | MediaPurpose::Receive { .. }
        ) {
            return Err(StreamControlError::Authority);
        }
        let read = context
            .downcast_ref::<std::sync::Weak<dyn Any + Send + Sync>>()
            .and_then(std::sync::Weak::upgrade)
            .and_then(|read| read.downcast::<NativeMediaRead>().ok())
            .ok_or(StreamControlError::Authority)?;
        self.packet_current(scope, Some(read), true)
    }
    fn current_participant_receive(
        &self,
        scope: &MediaScope<'_>,
        source: &rvoip_core::staged_streams::MediaParticipantSource,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        if let Some(target) = self.initial_target(scope.connection)? {
            return target.current_participant_receive(scope, source);
        }
        self.fresh_participant_receive(scope, source)
    }
    fn current_control(
        &self,
        scope: &MediaScope<'_>,
        intent: &MediaControlIntent<'_>,
        held: &(dyn Any + Send + Sync),
    ) -> ControlResult<MediaControl> {
        #[cfg(feature = "test-support")]
        if let Some(result) = crate::canonical_replacement::probe_held_start_control_for_test(
            self, scope, intent, held,
        ) {
            return result;
        }
        // Startup owns its exact target policy too. Do not consult unrelated
        // packet-routing registries before its unchanged current authority.
        if let Some(result) =
            crate::canonical_replacement::held_start_control(self, scope, intent, held)
        {
            return result;
        }
        // A held replacement already owns exact scene entries and current
        // Store custody. Unrelated packet-registry contention is not a changed
        // command grant; dispatch the sealed invocation before packet routing.
        if let Some(i) =
            held.downcast_ref::<crate::canonical_replacement::continuation::Invocation>()
        {
            if !std::ptr::eq(self, Arc::as_ptr(&i.policy.base)) {
                return Err(StreamControlError::Authority);
            }
            return i.media_control(scope, intent);
        }
        if let Some(target) = self.initial_target(scope.connection)? {
            return target.current_control(scope, intent, held);
        }
        if let Some(target) = self.telephone_target(scope.connection)? {
            return target.current_control(scope, intent, held);
        }
        if let Some(target) = self.replacement_target(scope.connection)? {
            return target.current_control(scope, intent, held);
        }

        if let Some(i) = held.downcast_ref::<webrtc::RtcInvocation>() {
            return i.media_control(self, scope, intent);
        }
        if let Some(i) = held.downcast_ref::<credential_refresh::Invocation>() {
            return i.control(self, scope, intent);
        }
        if let Some(i) =
            held.downcast_ref::<crate::telephone_replacement::continuation::Invocation>()
        {
            if !std::ptr::eq(self, Arc::as_ptr(&i.runtime.media)) {
                return Err(StreamControlError::Authority);
            }
            return i.media_control(scope, intent);
        }
        if let Some(i) = held.downcast_ref::<handoff::HandoffInvocation>() {
            return self.consultation_control(
                scope,
                intent,
                &consultation::AudienceInvocation {
                    policy: i.policy.clone(),
                    plan: i.plan.media().audience().clone(),
                    read: consultation::AudienceRead::Handoff(i.read.clone()),
                    cancel: i.cancel.clone(),
                },
            );
        }
        if let Some(i) = held.downcast_ref::<consultation::AudienceInvocation>() {
            return self.consultation_control(scope, intent, i);
        }
        if let Some(i) = held.downcast_ref::<Arc<rvoip_uctp::v2::native_subscriptions::NativeSubscriptionInvocation>>() {
            return self.reconciliation_control(scope, intent, i.clone());
        }
        if let Some(i)=held.downcast_ref::<Arc<rvoip_uctp::v2::native_session_codecs::NativeSessionCodecInvocation>>() {return self.codec_control(scope,intent,i.clone());}
        if let Some(i) = held.downcast_ref::<streams::StreamInvocation>() {
            return self.stream_control(scope, intent, i).map_err(|error|{tracing::debug!(target:"connect_service::canonical_native_streams",?error,"Native stream control policy refused");error});
        }
        let i = Arc::new(
            held.downcast_ref::<Invocation>()
                .ok_or(StreamControlError::Authority)?
                .clone(),
        );

        self.scope(scope, &i.connection)?;

        match intent {
            MediaControlIntent::Initialize => (),
            MediaControlIntent::Gate {
                expected,
                streams,
                action,
            } if (i.read.target().private_consultation.is_some()
                || i.read.target().handoff_preparation)
                && *expected == 1
                && *action == rvoip_core::stream_lifecycle::StreamGateAction::Mute
                && **streams
                    == i.connection
                        .plan
                        .selected()
                        .iter()
                        .filter(|s| {
                            s.requested.direction
                                != rvoip_core::stream_lifecycle::StreamDirection::Send
                        })
                        .map(|s| s.requested.logical_id.clone())
                        .collect::<BTreeSet<_>>() =>
            {
                ()
            }
            MediaControlIntent::PreparePrivateAudio {
                operation,
                expected,
            } if (i.read.target().private_consultation.is_some()
                || i.read.target().handoff_preparation)
                && *operation == i.connection.origin.offer_record_id()
                && *expected == 2 =>
            {
                ()
            }
            MediaControlIntent::Change {
                expected,
                desired,
                replace,
            } if *expected == i.authorized.envelope().expected_revision().unwrap_or(0)
                && desired
                    .iter()
                    .eq(i.connection.plan.selected().iter().map(|s| &s.requested))
                && replace.is_empty() =>
            {
                ()
            }
            _ => return Err(StreamControlError::Authority),
        }

        let read = self.held_read(&i, None)?;

        let owner = i.connection.owner(read.target(), scope.principal, false)?;

        MediaControl::new(
            Box::new(Held {
                invocation: i.clone(),
                owner,
            }),
            i.authorized.invocation_cancellation(),
            tokio::time::Instant::now() + Duration::from_secs(30),
        )
    }
}
pub(crate) fn source_matches(
    source: &MediaSource,
    entry: &Connection,
    view: &PeerConnectionProjection,
) -> ControlResult<()> {
    if !matches!(view.connection.state.as_str(), "ready" | "active") {
        return Err(StreamControlError::NotReady);
    }
    let owner = entry.owner(view, &entry.principal, false)?;
    let ready = entry
        .activation()?
        .ok_or(StreamControlError::NotReady)?
        .current(owner)?;
    if ready.connection.generation() != source.connection_generation {
        return Err(StreamControlError::Authority);
    }

    if source.conversation.as_str() != entry.origin.conversation_id()
        || source.session.as_str() != entry.origin.session_id()
        || source.connection.as_str() != entry.origin.runtime_connection_id()
        || source.participant.as_str() != view.connection.participant_id
        || source.membership_epoch != view.connection.membership_epoch
        || source.connection_generation != view.connection.generation
        || !entry.plan.selected().iter().any(|s| {
            s.requested.logical_id == source.logical_id
                && allows(view, s.requested.medium, &s.format)
        })
    {
        return Err(StreamControlError::Authority);
    }
    Ok(())
}

pub(crate) mod consultation;
mod participants;
mod roles;
pub(crate) mod streams;
#[cfg(test)]
mod tests;
