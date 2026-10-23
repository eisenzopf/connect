//! Participant routing fixtures use public provisioning and current credentials.
#![allow(clippy::unwrap_used)]
use connect_protocol::{provisioning::*, routing::*, Limits, Operation, Outcome};
use connect_store::{
    DiscoveryRuntime, EndpointObservation, EndpointProfile, EndpointRuntime, ObservationSource,
    Scope, Store,
};

struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
    admin: String,
    keys: Vec<String>,
    ids: Vec<String>,
    cid: String,
    parts: Vec<String>,
    runtime: DiscoveryRuntime,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store =
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap();
        let mut ids = vec![];
        let mut keys = vec![];
        for alias in ["alice", "bob", "outsider"] {
            let id = store.create_identity(alias).unwrap().id;
            keys.push(
                store
                    .issue(
                        &id,
                        Scope::collaboration(vec!["*".into()]),
                        chrono::Utc::now().timestamp() + 600,
                    )
                    .unwrap()
                    .id,
            );
            ids.push(id);
        }
        let admin = store
            .issue(
                &ids[0],
                Scope::operator(),
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap()
            .id;
        let Outcome::Conversation(c) = store
            .execute(
                &keys[0],
                &Operation::Create {
                    request_id: "create".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![ids
                        [1]
                    .clone()]),
                },
            )
            .unwrap()
        else {
            panic!("conversation")
        };
        let snapshot = store.membership_snapshot(&keys[0], &c.id).unwrap();
        let parts = ids[..2]
            .iter()
            .map(|id| {
                snapshot
                    .participants
                    .iter()
                    .find(|p| p.identity_id == *id)
                    .unwrap()
                    .participant_id
                    .clone()
            })
            .collect();
        let endpoints = EndpointRuntime::new(vec![
            EndpointProfile {
                transport: EndpointTransport::Quic,
                media: vec![
                    RouteMedium::Audio,
                    RouteMedium::Video,
                    RouteMedium::Messages,
                    RouteMedium::Files,
                ],
            },
            EndpointProfile {
                transport: EndpointTransport::Webrtc,
                media: vec![RouteMedium::Audio, RouteMedium::Video],
            },
            EndpointProfile {
                transport: EndpointTransport::Sms,
                media: vec![RouteMedium::Messages],
            },
            EndpointProfile {
                transport: EndpointTransport::Sip,
                media: vec![RouteMedium::Audio],
            },
        ])
        .unwrap();
        let endpoints = endpoints
            .with_secret_refs(
                [
                    EndpointTransport::Sms,
                    EndpointTransport::Sip,
                    EndpointTransport::Telephone,
                    EndpointTransport::VapiVoice,
                    EndpointTransport::VapiChat,
                ]
                .into_iter()
                .map(|t| (t, "env_FIXTURE_SECRET".into()))
                .collect(),
            )
            .unwrap();
        Self {
            _dir: dir,
            store,
            admin,
            keys,
            ids,
            cid: c.id,
            parts,
            runtime: DiscoveryRuntime {
                endpoints,
                ..Default::default()
            },
        }
    }
    fn endpoint(
        &self,
        who: usize,
        device: &str,
        transport: EndpointTransport,
        address: &str,
        media: &[&str],
        verify: bool,
    ) -> String {
        let secret_ref = matches!(
            transport,
            EndpointTransport::Sip
                | EndpointTransport::Sms
                | EndpointTransport::Telephone
                | EndpointTransport::VapiVoice
                | EndpointTransport::VapiChat
        )
        .then(|| "env_FIXTURE_SECRET".into());
        let ProvisioningOutcome::Endpoint { endpoint_id, .. } = self
            .store
            .execute_provisioning(
                &self.admin,
                ProvisioningCommand::Endpoint {
                    operation_id: format!("create-{device}"),
                    identity_id: self.ids[who].clone(),
                    device: DeviceSpec {
                        id: device.into(),
                        kind: "fixture".into(),
                        platform: "fixture".into(),
                    },
                    transport,
                    address: address.into(),
                    capabilities: media.iter().map(|s| s.to_string()).collect(),
                    secret_ref,
                },
            )
            .unwrap()
        else {
            panic!("endpoint")
        };
        if verify {
            self.store
                .execute_provisioning(
                    &self.admin,
                    ProvisioningCommand::VerifyEndpoint {
                        operation_id: format!("verify-{device}"),
                        endpoint_id: endpoint_id.clone(),
                        expected_revision: 0,
                        evidence_ref: "fixture-proof".into(),
                    },
                )
                .unwrap();
        }
        endpoint_id
    }
    fn observe(&self, id: &str, sequence: u64, state: EndpointReachability) {
        let now = chrono::Utc::now().timestamp();
        self.store
            .record_endpoint_observation(
                &self.runtime.endpoints,
                EndpointObservation {
                    installation_id: self.store.installation_id().unwrap(),
                    runtime_epoch: self.runtime.endpoints.epoch().into(),
                    endpoint_id: id.into(),
                    endpoint_revision: 1,
                    sequence,
                    observed_at: now,
                    expires_at: now + 60,
                    state,
                    source: ObservationSource::LocalFixture,
                },
            )
            .unwrap();
    }
    fn request(&self) -> ParticipantRouteRequest {
        ParticipantRouteRequest {
            conversation_id: self.cid.clone(),
            participant_id: self.parts[1].clone(),
            purpose: RoutePurpose::Session,
            direction: RouteDirection::Outgoing,
            media: vec![RouteMedium::Audio],
            transports: vec![EndpointTransport::Quic],
            preferred_device_id: None,
            failover: FailoverPolicy::None,
            reachability: ReachabilityPolicy::RequireFresh,
        }
    }
    fn route(&self, request: ParticipantRouteRequest) -> ParticipantRoute {
        let Outcome::ParticipantRoute(r) = self
            .store
            .execute_with_runtime(
                &self.keys[0],
                &Operation::ParticipantRoute { request },
                &self.runtime,
            )
            .unwrap()
        else {
            panic!("route")
        };
        r
    }
}

#[test]
fn multiple_devices_require_explicit_preference_and_failover_never_changes_identity_or_channel() {
    let f = Fixture::new();
    let a = f.endpoint(
        1,
        "laptop",
        EndpointTransport::Quic,
        "quic://private-laptop.example:4433",
        &["audio", "video"],
        true,
    );
    let b = f.endpoint(
        1,
        "phone",
        EndpointTransport::Quic,
        "quic://private-phone.example:4433",
        &["audio"],
        true,
    );
    assert_eq!(f.route(f.request()).reason, RouteReason::Unknown);
    f.observe(&a, 1, EndpointReachability::Reachable);
    f.observe(&b, 1, EndpointReachability::Reachable);
    assert_eq!(f.route(f.request()).reason, RouteReason::Ambiguous);
    let mut request = f.request();
    request.preferred_device_id = Some("laptop".into());
    let selected = f.route(request.clone());
    assert_eq!(selected.selected.as_ref().unwrap().endpoint_id, a);
    let binding = f
        .store
        .authorize_selected_endpoint(&f.keys[0], request.clone(), &selected, &f.runtime)
        .unwrap();
    assert!(binding.address.contains("private-laptop"));
    assert!(!format!("{binding:?}").contains("private-laptop"));
    let json = serde_json::to_string(&selected).unwrap();
    assert!(!json.contains("private-") && !json.contains("secret") && !json.contains("route_id"));
    f.observe(&a, 2, EndpointReachability::Offline);
    assert_eq!(f.route(request.clone()).reason, RouteReason::Offline);
    assert!(f
        .store
        .authorize_selected_endpoint(&f.keys[0], request.clone(), &selected, &f.runtime)
        .is_err());
    request.failover = FailoverPolicy::OtherAvailableDevice;
    assert_eq!(f.route(request.clone()).selected.unwrap().endpoint_id, b);
    request.media.push(RouteMedium::Video);
    assert_eq!(f.route(request).reason, RouteReason::PrerequisitesMissing);
    // Other device is audio-only: no silent upgrade downgrade or SIP fallback.
    let mut request = f.request();
    request.transports = vec![EndpointTransport::Sip];
    assert_eq!(f.route(request).reason, RouteReason::NoEndpoint);
}

#[test]
fn stale_reachability_revocation_restore_and_reordered_observations_fail_closed() {
    let f = Fixture::new();
    let endpoint = f.endpoint(
        1,
        "laptop",
        EndpointTransport::Quic,
        "quic://private.example:4433",
        &["audio"],
        true,
    );
    f.observe(&endpoint, 2, EndpointReachability::Busy);
    assert_eq!(f.route(f.request()).reason, RouteReason::Busy);
    let now = chrono::Utc::now().timestamp();
    let mut obs = EndpointObservation {
        installation_id: f.store.installation_id().unwrap(),
        runtime_epoch: f.runtime.endpoints.epoch().into(),
        endpoint_id: endpoint.clone(),
        endpoint_revision: 1,
        sequence: 1,
        observed_at: now,
        expires_at: now + 60,
        state: EndpointReachability::Reachable,
        source: ObservationSource::LocalFixture,
    };
    assert_eq!(
        f.store
            .record_endpoint_observation(&f.runtime.endpoints, obs.clone())
            .unwrap_err()
            .code,
        "conflict"
    );
    obs.sequence = 3;
    obs.runtime_epoch = "previous-process".into();
    assert!(f
        .store
        .record_endpoint_observation(&f.runtime.endpoints, obs.clone())
        .is_err());
    obs.runtime_epoch = f.runtime.endpoints.epoch().into();
    obs.installation_id = "other-installation".into();
    assert!(f
        .store
        .record_endpoint_observation(&f.runtime.endpoints, obs.clone())
        .is_err());
    obs.installation_id = f.store.installation_id().unwrap();
    obs.endpoint_revision = 0;
    assert!(f
        .store
        .record_endpoint_observation(&f.runtime.endpoints, obs.clone())
        .is_err());
    obs.endpoint_revision = 1;
    obs.observed_at = now - 61;
    obs.expires_at = now - 1;
    assert!(f
        .store
        .record_endpoint_observation(&f.runtime.endpoints, obs)
        .is_err());
    let unobserved = DiscoveryRuntime {
        endpoints: EndpointRuntime::new(vec![EndpointProfile {
            transport: EndpointTransport::Quic,
            media: vec![RouteMedium::Audio],
        }])
        .unwrap(),
        ..Default::default()
    };
    assert_eq!(
        f.store
            .participant_route(&f.keys[0], f.request(), &unobserved)
            .unwrap()
            .reason,
        RouteReason::Unknown
    );
    let selected_before = f.route(f.request());
    assert_eq!(selected_before.reason, RouteReason::Busy);
    f.observe(&endpoint, 4, EndpointReachability::Reachable);
    let selected = f.route(f.request());
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::RevokeEndpoint {
                operation_id: "revoke".into(),
                endpoint_id: endpoint,
                expected_revision: 1,
            },
        )
        .unwrap();
    assert_eq!(f.route(f.request()).reason, RouteReason::NoEndpoint);
    assert!(f
        .store
        .authorize_selected_endpoint(&f.keys[0], f.request(), &selected, &f.runtime)
        .is_err());
}

#[test]
fn permission_tenant_membership_and_incoming_identity_are_checked_before_inventory() {
    let f = Fixture::new();
    let endpoint = f.endpoint(
        1,
        "bob-device",
        EndpointTransport::Quic,
        "quic://private.example:4433",
        &["audio"],
        true,
    );
    f.observe(&endpoint, 1, EndpointReachability::Reachable);
    for key in [
        &f.keys[2],
        &f.store
            .issue(
                &f.ids[0],
                Scope {
                    operations: vec!["resolve".into()],
                    recipients: vec!["*".into()],
                },
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap()
            .id,
    ] {
        assert_eq!(
            f.store
                .participant_route(key, f.request(), &f.runtime)
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
    let mut incoming = f.request();
    incoming.direction = RouteDirection::Incoming;
    assert_eq!(
        f.store
            .participant_route(&f.keys[0], incoming.clone(), &f.runtime)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .participant_route(&f.keys[1], incoming, &f.runtime)
            .unwrap()
            .reason,
        RouteReason::Selected
    );
    let mut unknown = f.request();
    unknown.participant_id = "part_unknown".into();
    assert_eq!(
        f.store
            .participant_route(&f.keys[0], unknown, &f.runtime)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let mut wrong = f.request();
    wrong.preferred_device_id = Some("unknown-device".into());
    assert_eq!(
        f.store
            .participant_route(&f.keys[0], wrong, &f.runtime)
            .unwrap_err()
            .code,
        "forbidden"
    );
    f.store
        .execute_membership(
            &f.keys[0],
            connect_protocol::membership::MembershipRequest {
                operation_id: "remove".into(),
                conversation_id: f.cid.clone(),
                expected_revision: 0,
                change: connect_protocol::membership::MembershipAction::Remove {
                    participant_id: f.parts[1].clone(),
                    reason: None,
                },
            },
        )
        .unwrap();
    assert_eq!(
        f.store
            .participant_route(&f.keys[0], f.request(), &f.runtime)
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn verification_capability_configuration_and_consent_are_not_reachability() {
    let f = Fixture::new();
    f.endpoint(
        1,
        "unverified",
        EndpointTransport::Quic,
        "quic://private.example:4433",
        &["audio"],
        false,
    );
    assert_eq!(f.route(f.request()).reason, RouteReason::Unverified);
    let audio = f.endpoint(
        1,
        "audio-only",
        EndpointTransport::Quic,
        "quic://audio.example:4433",
        &["audio"],
        true,
    );
    f.observe(&audio, 1, EndpointReachability::Reachable);
    let mut request = f.request();
    request.preferred_device_id = Some("audio-only".into());
    request.media.push(RouteMedium::Video);
    assert_eq!(
        f.route(request.clone()).reason,
        RouteReason::UnsupportedCapability
    );
    request.media = vec![RouteMedium::Audio];
    let empty = DiscoveryRuntime::default();
    assert_eq!(
        f.store
            .participant_route(&f.keys[0], request, &empty)
            .unwrap()
            .reason,
        RouteReason::Unconfigured
    );
    let sms = f.endpoint(
        1,
        "sms",
        EndpointTransport::Sms,
        "+12025550101",
        &["messaging"],
        true,
    );
    f.observe(&sms, 1, EndpointReachability::Reachable);
    let mut request = f.request();
    request.purpose = RoutePurpose::Message;
    request.media = vec![RouteMedium::Messages];
    request.transports = vec![EndpointTransport::Sms];
    assert_eq!(f.route(request.clone()).reason, RouteReason::Unconfigured);
    f.store
        .configure_route(
            &f.admin,
            &connect_store::Route {
                id: "private-route".into(),
                identity_id: f.ids[1].clone(),
                credential_id: f.keys[1].clone(),
                conversation_id: f.cid.clone(),
                connector: "telnyx".into(),
                target: "+12025550101".into(),
                source: Some("+12025550102".into()),
                enabled: true,
                incoming_to: connect_protocol::messages::MessageAudience::all(),
            },
        )
        .unwrap();
    let result = f.route(request);
    assert_eq!(result.reason, RouteReason::ConsentRequired);
    let json = serde_json::to_string(&result).unwrap();
    assert!(!json.contains("+1202") && !json.contains("private-route"));
}

#[test]
fn expired_observations_do_not_resurrect_and_read_only_selection_does_not_change_the_journal() {
    let f = Fixture::new();
    let id = f.endpoint(
        1,
        "laptop",
        EndpointTransport::Quic,
        "quic://private.example:4433",
        &["audio"],
        true,
    );
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let now = chrono::Utc::now().timestamp();
    let expires = now + 1;
    let value = EndpointObservation {
        installation_id: f.store.installation_id().unwrap(),
        runtime_epoch: f.runtime.endpoints.epoch().into(),
        endpoint_id: id.clone(),
        endpoint_revision: 1,
        sequence: 5,
        observed_at: now - 59,
        expires_at: expires,
        state: EndpointReachability::Reachable,
        source: ObservationSource::LocalFixture,
    };
    f.store
        .record_endpoint_observation(&f.runtime.endpoints, value.clone())
        .unwrap();
    assert_eq!(f.route(f.request()).reason, RouteReason::Selected);
    while chrono::Utc::now().timestamp() < expires {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(f.route(f.request()).reason, RouteReason::Unknown);
    assert!(f
        .store
        .record_endpoint_observation(&f.runtime.endpoints, value)
        .is_err());
    f.observe(&id, 6, EndpointReachability::Reachable);
    assert_eq!(f.route(f.request()).reason, RouteReason::Selected);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}

#[test]
fn other_tenants_are_not_discoverable_even_with_wildcard_recipient_scope() {
    let f = Fixture::new();
    let c = rusqlite::Connection::open(f.store.path()).unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON;INSERT INTO tenants(id) VALUES('tnt_other')")
        .unwrap();
    c.execute(
        "UPDATE identities SET tenant_id='tnt_other' WHERE id=?1",
        [&f.ids[2]],
    )
    .unwrap();
    for op in [
        Operation::Resolve {
            address: "connect:outsider@local".into(),
        },
        Operation::Capabilities {
            address: "connect:outsider@local".into(),
            conversation_id: None,
        },
        Operation::AvailabilityGet {
            identity_id: f.ids[2].clone(),
        },
    ] {
        assert_eq!(
            f.store
                .execute_with_runtime(&f.keys[0], &op, &f.runtime)
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
    let Outcome::Conversation(other) = f
        .store
        .execute(
            &f.keys[2],
            &Operation::Create {
                request_id: "foreign".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![]),
            },
        )
        .unwrap()
    else {
        panic!("conversation")
    };
    let mut request = f.request();
    request.conversation_id = other.id;
    assert_eq!(
        f.store
            .participant_route(&f.keys[0], request, &f.runtime)
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn incoming_selection_requires_accept_authority_without_borrowing_origination_permissions() {
    let f = Fixture::new();
    let id = f.endpoint(
        1,
        "laptop",
        EndpointTransport::Quic,
        "quic://private.example:4433",
        &["audio"],
        true,
    );
    f.observe(&id, 1, EndpointReachability::Reachable);
    let incoming = f
        .store
        .issue(
            &f.ids[1],
            Scope {
                operations: vec![
                    "resolve".into(),
                    "availability_get".into(),
                    "session_accept".into(),
                ],
                recipients: vec![],
            },
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut request = f.request();
    request.direction = RouteDirection::Incoming;
    assert_eq!(
        f.store
            .participant_route(&incoming.id, request.clone(), &f.runtime)
            .unwrap()
            .reason,
        RouteReason::Selected
    );
    request.direction = RouteDirection::Outgoing;
    assert_eq!(
        f.store
            .participant_route(&incoming.id, request, &f.runtime)
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn sms_consent_and_delegate_revocation_remain_independent_of_an_online_endpoint() {
    let f = Fixture::new();
    let id = f.endpoint(
        1,
        "sms",
        EndpointTransport::Sms,
        "+12025550101",
        &["messaging"],
        true,
    );
    f.observe(&id, 1, EndpointReachability::Reachable);
    f.store
        .configure_route(
            &f.admin,
            &connect_store::Route {
                id: "private-route".into(),
                identity_id: f.ids[1].clone(),
                credential_id: f.keys[1].clone(),
                conversation_id: f.cid.clone(),
                connector: "telnyx".into(),
                target: "+12025550101".into(),
                source: Some("+12025550102".into()),
                enabled: true,
                incoming_to: connect_protocol::messages::MessageAudience::all(),
            },
        )
        .unwrap();
    let mut request = f.request();
    request.purpose = RoutePurpose::Message;
    request.media = vec![RouteMedium::Messages];
    request.transports = vec![EndpointTransport::Sms];
    assert_eq!(
        f.route(request.clone()).reason,
        RouteReason::ConsentRequired
    );
    let now = chrono::Utc::now().timestamp();
    f.store
        .refresh_sms_grants(
            "fixture",
            &connect_store::SmsGrantSnapshot {
                revision: 1,
                grants: vec![connect_store::SmsGrant {
                    id: "grant".into(),
                    phone: "+12025550101".into(),
                    sender: "+12025550102".into(),
                    conversation_id: f.cid.clone(),
                    purpose: "Reviewed fixture communication".into(),
                    consent_id: "consent".into(),
                    disclosure_version: "fixture-1".into(),
                    disclosure_text: "Local fixture only".into(),
                    submitted_at: now,
                    approved_at: now,
                    expires_at: now + 600,
                }],
            },
        )
        .unwrap();
    let selected = f.route(request.clone());
    assert_eq!(selected.reason, RouteReason::Selected);
    f.store.expire_sms_authorities().unwrap();
    assert_eq!(
        f.route(request.clone()).reason,
        RouteReason::ConsentRequired
    );
    assert!(f
        .store
        .authorize_selected_endpoint(&f.keys[0], request.clone(), &selected, &f.runtime)
        .is_err());
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &f.keys[1]).unwrap();
    assert_eq!(f.route(request).reason, RouteReason::PermissionRequired);
}

#[test]
fn unknown_phone_attempt_is_explicit_and_preserves_endpoint_and_assistant_identities() {
    let mut f = Fixture::new();
    f.runtime.endpoints = EndpointRuntime::new(vec![EndpointProfile {
        transport: EndpointTransport::Telephone,
        media: vec![RouteMedium::Audio],
    }])
    .unwrap()
    .with_secret_refs(vec![(
        EndpointTransport::Telephone,
        "env_FIXTURE_SECRET".into(),
    )])
    .unwrap();
    let endpoint = f.endpoint(
        1,
        "phone",
        EndpointTransport::Telephone,
        "+12025550101",
        &["audio"],
        true,
    );
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::Participant {
                operation_id: "assistant-member".into(),
                conversation_id: f.cid.clone(),
                identity_id: f.ids[2].clone(),
                role: "assistant".into(),
            },
        )
        .unwrap();
    f.store
        .configure_voice_route(
            &f.admin,
            &connect_store::VoiceRoute {
                id: "private-voice-route".into(),
                conversation_id: f.cid.clone(),
                identity_id: f.ids[2].clone(),
                assistant_id: "12345678-1234-1234-1234-123456789012".into(),
                phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
                destinations: vec!["+12025550101".into()],
                enabled: true,
            },
        )
        .unwrap();
    let mut request = f.request();
    request.transports = vec![EndpointTransport::Telephone];
    assert_eq!(f.route(request.clone()).reason, RouteReason::Unknown);
    request.reachability = ReachabilityPolicy::AllowUnknown;
    let selected = f.route(request.clone());
    assert_eq!(selected.reason, RouteReason::AttemptAllowed);
    assert_eq!(selected.participant_id, f.parts[1]);
    assert_eq!(selected.selected.as_ref().unwrap().endpoint_id, endpoint);
    assert_eq!(
        selected.selected.as_ref().unwrap().reachability,
        EndpointReachability::Unknown
    );
    f.store
        .authorize_selected_endpoint(&f.keys[0], request.clone(), &selected, &f.runtime)
        .unwrap();
    let limited = f
        .store
        .issue(
            &f.ids[0],
            Scope::collaboration(vec![f.ids[1].clone()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    assert_eq!(
        f.store
            .participant_route(&limited.id, request.clone(), &f.runtime)
            .unwrap()
            .reason,
        RouteReason::PermissionRequired
    );
    f.observe(&endpoint, 1, EndpointReachability::Offline);
    assert_eq!(f.route(request.clone()).reason, RouteReason::Offline);
    assert!(f
        .store
        .authorize_selected_endpoint(&f.keys[0], request, &selected, &f.runtime)
        .is_err());
}

#[test]
fn old_observer_generation_and_wrong_transport_credential_binding_cannot_reenable_an_endpoint() {
    let f = Fixture::new();
    let endpoint = f.endpoint(
        1,
        "sms",
        EndpointTransport::Sms,
        "+12025550101",
        &["messaging"],
        true,
    );
    let now = chrono::Utc::now().timestamp();
    let frame = EndpointObservation {
        installation_id: f.store.installation_id().unwrap(),
        runtime_epoch: f.runtime.endpoints.epoch().into(),
        endpoint_id: endpoint.clone(),
        endpoint_revision: 1,
        sequence: 1,
        observed_at: now,
        expires_at: now + 60,
        state: EndpointReachability::Reachable,
        source: ObservationSource::LocalFixture,
    };
    let replacement = EndpointRuntime::new(vec![EndpointProfile {
        transport: EndpointTransport::Sms,
        media: vec![RouteMedium::Messages],
    }])
    .unwrap();
    assert_ne!(replacement.epoch(), f.runtime.endpoints.epoch());
    assert_eq!(
        f.store
            .record_endpoint_observation(&replacement, frame.clone())
            .unwrap_err()
            .code,
        "forbidden"
    );
    f.store
        .record_endpoint_observation(&f.runtime.endpoints, frame)
        .unwrap();
    let mut request = f.request();
    request.purpose = RoutePurpose::Message;
    request.media = vec![RouteMedium::Messages];
    request.transports = vec![EndpointTransport::Sms];
    let wrong = DiscoveryRuntime {
        endpoints: f
            .runtime
            .endpoints
            .clone()
            .with_secret_refs(vec![(EndpointTransport::Quic, "env_FIXTURE_SECRET".into())])
            .unwrap(),
        ..Default::default()
    };
    assert_eq!(
        f.store
            .participant_route(&f.keys[0], request, &wrong)
            .unwrap()
            .reason,
        RouteReason::Unconfigured
    );
}

#[test]
fn configured_browser_pair_and_current_delegate_scope_remain_required() {
    let mut f = Fixture::new();
    let endpoint = f.endpoint(
        1,
        "browser",
        EndpointTransport::Webrtc,
        "https://private-browser.example/rtc",
        &["audio"],
        true,
    );
    f.observe(&endpoint, 1, EndpointReachability::Reachable);
    let configured = f
        .store
        .issue(
            &f.ids[0],
            Scope::communications(vec![f.ids[1].clone()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    f.runtime.browser = Some(connect_store::BrowserRoute {
        credential_id: configured.id.clone(),
        binding: connect_protocol::BrowserBinding {
            conversation_id: f.cid.clone(),
            participant_id: f.ids[1].clone(),
            session_id: None,
            signaling_url: "ws://127.0.0.1:8181".into(),
        },
    });
    let mut request = f.request();
    request.transports = vec![EndpointTransport::Webrtc];
    assert_eq!(f.route(request.clone()).reason, RouteReason::Selected);
    let mut wrong = f.runtime.clone();
    wrong.browser.as_mut().unwrap().binding.participant_id = f.ids[2].clone();
    assert_eq!(
        f.store
            .participant_route(&f.keys[0], request.clone(), &wrong)
            .unwrap()
            .reason,
        RouteReason::Unconfigured
    );
    f.store
        .revoke(&f.store.revalidate(&f.admin).unwrap(), &configured.id)
        .unwrap();
    assert_eq!(f.route(request).reason, RouteReason::PermissionRequired);
}
