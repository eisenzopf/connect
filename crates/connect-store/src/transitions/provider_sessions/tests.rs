#![allow(clippy::unwrap_used)]
use super::*;
use crate::Scope;
use connect_protocol::{Limits, Outcome};
use serde_json::json;
mod incoming_custody;
mod incoming_resolution;
struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
    _ids: Vec<String>,
    keys: Vec<String>,
    parts: Vec<String>,
    request: session_control::Request,
    policy: ProviderSessionVoicePolicy,
}
impl Fixture {
    fn new() -> Self {
        Self::limits(Limits::default())
    }
    fn limits(limits: Limits) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::initialize(&dir.path().join("initial.sqlite3"), limits).unwrap();
        let root = store.create_identity("fixture-root").unwrap().id;
        let root_key = store
            .issue(&root, Scope::operator(), crate::now() + 3600)
            .unwrap()
            .id;
        let mut ids = vec![];
        let mut keys = vec![];
        for name in ["caller", "phone", "other"] {
            let identity = if name == "caller" {
                let connect_protocol::provisioning::ProvisioningOutcome::Identity {
                    identity_id,
                    ..
                } = store
                    .execute_provisioning(
                        &root_key,
                        connect_protocol::provisioning::ProvisioningCommand::Identity {
                            operation_id: "provider-caller".into(),
                            alias: name.into(),
                            kind: connect_protocol::provisioning::IdentityKind::Ai,
                            display_name: None,
                        },
                    )
                    .unwrap()
                else {
                    panic!()
                };
                identity_id
            } else {
                store.create_identity(name).unwrap().id
            };
            keys.push(
                store
                    .issue(&identity, Scope::operator(), crate::now() + 3600)
                    .unwrap()
                    .id,
            );
            ids.push(identity);
        }
        let Outcome::Conversation(room) = store
            .execute(
                &keys[0],
                &Operation::Create {
                    request_id: "room".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(
                        ids[1..].to_vec(),
                    ),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        let members = store
            .membership_snapshot(&keys[0], &room.id)
            .unwrap()
            .participants;
        let parts: Vec<String> = ids
            .iter()
            .map(|id| {
                members
                    .iter()
                    .find(|p| &p.identity_id == id)
                    .unwrap()
                    .participant_id
                    .clone()
            })
            .collect();
        let request = session_control::Request {
            operation_id: "initial-invitation".into(),
            conversation_id: room.id.clone(),
            session_id: "initial-session".into(),
            by_participant: Some(parts[0].clone()),
            expected_revision: None,
            action: session_control::Action::Invite {
                participants: vec![parts[1].clone()],
                medium: "voice".into(),
                intent: "call".into(),
                capabilities: json!({"audio":["g.711-mu"]}),
            },
        };
        store
            .execute(
                &keys[0],
                &Operation::SessionControl {
                    request: request.clone(),
                },
            )
            .unwrap();
        {
            let c = store.connection().unwrap();
            c.execute("INSERT INTO devices(id,identity_id,tenant_id,kind,platform) VALUES('initial-phone-device',?1,'tnt_default','phone','pstn')",[&ids[1]]).unwrap();
            c.execute("INSERT INTO endpoints(id,identity_id,tenant_id,device_id,transport,address,capabilities_json,verification_state,verification_evidence,verified_at,secret_ref,route_owner_id) VALUES('initial-phone',?1,'tnt_default','initial-phone-device','telephone','+15555550111','[\"audio\"]','verified','fixture',?2,'initial-trunk',?3)",params![ids[1],crate::now(),ids[0]]).unwrap();
        }
        {
            let c = store.connection().unwrap();
            c.execute("INSERT INTO devices(id,identity_id,tenant_id,kind,platform) VALUES('provider-assistant-device',?1,'tnt_default','ai-provider','vapi')",[&ids[0]]).unwrap();
            c.execute("INSERT INTO endpoints(id,identity_id,tenant_id,device_id,transport,address,capabilities_json,verification_state,verification_evidence,verified_at,secret_ref,route_owner_id) VALUES('provider-assistant',?1,'tnt_default','provider-assistant-device','vapi-voice','12345678-1234-1234-1234-123456789012','[\"audio\"]','verified','fixture',?2,'initial-trunk',?1)",params![ids[0],crate::now()]).unwrap();
        }
        let policy = ProviderSessionVoicePolicy {
            endpoint_id: "initial-phone".into(),
            endpoint_credential_id: keys[1].clone(),
            route_credential_id: keys[0].clone(),
            route_reference: "initial-trunk".into(),
            provider: "vapi-telephone".into(),
            assistant_id: "12345678-1234-1234-1234-123456789012".into(),
            assistant_endpoint_id: "provider-assistant".into(),
            assistant_credential_id: keys[0].clone(),
            phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
            timeout_seconds: 30,
        };
        Self {
            _dir: dir,
            store,
            _ids: ids,
            keys,
            parts,
            request,
            policy,
        }
    }
    fn origin(&self) -> ProviderSessionVoiceOrigin {
        self.store
            .prepare_provider_session_voice(
                &self.keys[0],
                self.request.clone(),
                &self.parts[1],
                self.policy.clone(),
            )
            .unwrap()
    }
    fn worker(&self) -> Arc<ProviderSessionVoiceWorker> {
        self.store.acquire_provider_session_voice_worker().unwrap()
    }
    fn begin(
        &self,
        worker: &Arc<ProviderSessionVoiceWorker>,
        origin: &ProviderSessionVoiceOrigin,
        action: ProviderSessionVoiceAction,
    ) -> ProviderSessionVoiceDispatch {
        self.store
            .begin_provider_session_voice(worker, origin, action, &manifest())
            .unwrap()
            .unwrap()
    }
    fn revoke(&self, index: usize) {
        self.store
            .revoke(
                &self.store.revalidate(&self.keys[0]).unwrap(),
                &self.keys[index],
            )
            .unwrap();
    }
    fn count(&self, table: &str) -> u64 {
        self.store
            .connection()
            .unwrap()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
}
fn manifest() -> Manifest {
    let mut m = Manifest::messaging(connect_connector_api::VAPI);
    m.name = "vapi-telephone".into();
    m.address = connect_connector_api::AddressKind::E164Pair;
    m.messaging = false;
    m.context = false;
    m.limits.context_bytes = 0;
    m.operations = [
        ConnectorOperation::VoiceStart,
        ConnectorOperation::VoiceStatus,
        ConnectorOperation::VoiceReconcile,
        ConnectorOperation::VoiceEnd,
        ConnectorOperation::VoiceDiscover,
    ]
    .into();
    m
}
fn receipt(
    origin: &ProviderSessionVoiceOrigin,
    state: &str,
    event: &str,
    reconcile: bool,
) -> VoiceReceipt {
    VoiceReceipt {
        call: connect_connector_api::RemoteCall {
            id: "12345678-1234-1234-1234-123456789015".into(),
            state: state.into(),
            control_url: Some(
                "https://fixture.vapi.ai/12345678-1234-1234-1234-123456789015/control".into(),
            ),
            ended_reason: None,
        },
        observation: Observation {
            mutation_id: origin.id().into(),
            external_id: Some("12345678-1234-1234-1234-123456789015".into()),
            event_id: event.into(),
            state: State::Voice(super::lifecycle::normalized(state).unwrap()),
            verification: if reconcile {
                Verification::Reconciliation
            } else {
                Verification::AuthenticatedResponse
            },
        },
    }
}
fn call_revision(f: &Fixture, o: &ProviderSessionVoiceOrigin) -> u64 {
    f.store
        .connection()
        .unwrap()
        .query_row(
            "SELECT revision FROM provider_session_voice WHERE id=?1",
            [o.id()],
            |r| r.get(0),
        )
        .unwrap()
}
fn effect_states(f: &Fixture) -> Vec<(String, Option<String>)> {
    let c = f.store.connection().unwrap();
    let mut q=c.prepare("SELECT state,compensates_effect_id FROM operation_effects ORDER BY compensates_effect_id IS NOT NULL").unwrap();
    q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}
#[test]
fn canonical_invitation_replay_has_one_provider_effect_and_no_legacy_call_or_media() {
    let f = Fixture::new();
    let o = f.origin();
    let again = f.origin();
    assert_eq!(o.id(), again.id());
    let w = f.worker();
    let d = f.begin(&w, &o, ProviderSessionVoiceAction::Start);
    let req = d.start_request().unwrap();
    assert_eq!(req.destination, "+15555550111");
    assert_eq!(effect_states(&f)[0].0, "submitting");
    assert!(f
        .store
        .begin_provider_session_voice(&w, &again, ProviderSessionVoiceAction::Start, &manifest())
        .unwrap()
        .is_none());
    let captured = d.capture(Ok(receipt(&o, "ringing", "created", false)));
    let view = f.store.record_provider_session_voice(&captured).unwrap();
    assert_eq!(view.state, ProviderCallState::Starting);
    assert_eq!(f.count("voice_calls"), 0);
    assert_eq!(f.count("connections"), 0);
    assert_eq!(f.count("native_stream_operations"), 0);
    assert_eq!(f.count("provider_session_voice"), 1);
    let encoded = serde_json::to_string(
        &f.store
            .provider_session_voice_view(&f.keys[0], o.id())
            .unwrap(),
    )
    .unwrap();
    assert!(!encoded.contains("control"));
    assert!(!encoded.contains("12345678-1234-1234-1234-123456789015"));
    assert_eq!(effect_states(&f), vec![("confirmed".into(), None)]);
}
#[test]
fn missing_start_capability_never_marks_submission_but_status_end_contract_remains_valid() {
    let f = Fixture::new();
    let o = f.origin();
    let w = f.worker();
    let mut m = manifest();
    m.operations.remove(&ConnectorOperation::VoiceStart);
    m.validate().unwrap();
    assert_eq!(
        f.store
            .begin_provider_session_voice(&w, &o, ProviderSessionVoiceAction::Start, &m)
            .unwrap_err()
            .code,
        "unsupported-connector-operation"
    );
    assert_eq!(f.count("operation_effects"), 0);
    assert!(f
        .store
        .abandon_provider_session_voice_unstarted(&o)
        .unwrap());
    assert!(!f
        .store
        .abandon_provider_session_voice_unstarted(&o)
        .unwrap());
    assert!(f
        .store
        .begin_provider_session_voice(&w, &o, ProviderSessionVoiceAction::Start, &manifest())
        .unwrap()
        .is_none());
}
#[test]
fn endpoint_revocation_before_dispatch_never_creates_an_external_effect() {
    let f = Fixture::new();
    let o = f.origin();
    let w = f.worker();
    f.revoke(1);
    assert!(f
        .store
        .begin_provider_session_voice(&w, &o, ProviderSessionVoiceAction::Start, &manifest())
        .is_err());
    assert_eq!(f.count("operation_effects"), 0);
    assert!(f
        .store
        .abandon_provider_session_voice_unstarted(&o)
        .unwrap());
}
#[test]
fn own_endpoint_route_revision_and_policy_are_bound_to_original_invitation() {
    let f = Fixture::new();
    let o = f.origin();
    let w = f.worker();
    let mut changed = f.policy.clone();
    changed.phone_number_id = "another-account".into();
    assert_eq!(
        f.store
            .prepare_provider_session_voice(&f.keys[0], f.request.clone(), &f.parts[1], changed)
            .unwrap_err()
            .code,
        "conflict"
    );
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE endpoints SET revision=revision+1 WHERE id='initial-phone'",
            [],
        )
        .unwrap();
    assert!(f
        .store
        .begin_provider_session_voice(&w, &o, ProviderSessionVoiceAction::Start, &manifest())
        .is_err());
    assert_eq!(f.count("operation_effects"), 0);
}
#[test]
fn provider_effects_require_an_exact_successful_explicit_target_invitation() {
    let f = Fixture::new();
    let mut request = f.request.clone();
    request.operation_id = "not-executed".into();
    assert!(f
        .store
        .prepare_provider_session_voice(&f.keys[0], request, &f.parts[1], f.policy.clone())
        .is_err());
    assert!(f
        .store
        .prepare_provider_session_voice(
            &f.keys[0],
            f.request.clone(),
            &f.parts[2],
            f.policy.clone()
        )
        .is_err());
    assert_eq!(f.count("provider_session_voice"), 0);
}
#[test]
fn captured_response_releases_transport_fence_and_survives_command_revocation_without_acceptance() {
    let f = Fixture::new();
    let o = f.origin();
    let w = f.worker();
    let capture = f
        .begin(&w, &o, ProviderSessionVoiceAction::Start)
        .capture(Ok(receipt(&o, "in-progress", "remote-answer", false)));
    f.revoke(1);
    let view = f.store.record_provider_session_voice(&capture).unwrap();
    assert_eq!(view.state, ProviderCallState::Active);
    assert_eq!(f.count("connections"), 0);
    let c = f.store.connection().unwrap();
    let state: String = c
        .query_row(
            "SELECT state FROM session_participants WHERE session_id=?1 AND participant_id=?2",
            params![o.session_id(), o.participant_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "invited");
    drop(c);
    assert!(f
        .store
        .provider_session_voice_view(&f.keys[1], o.id())
        .is_err());
    assert_eq!(
        f.store
            .provider_session_voice_view(&f.keys[0], o.id())
            .unwrap()
            .state,
        ProviderCallState::Active
    );
}
#[test]
fn unknown_creation_survives_worker_restart_and_cannot_redial_or_guess_cleanup() {
    let f = Fixture::new();
    let o = f.origin();
    let w = f.worker();
    let capture = f
        .begin(&w, &o, ProviderSessionVoiceAction::Start)
        .capture(Err(Failure::Uncertain));
    f.store.record_provider_session_voice(&capture).unwrap();
    w.retire();
    assert!(f.store.acquire_provider_session_voice_worker().is_err());
    drop(w);
    assert!(f.store.acquire_provider_session_voice_worker().is_err());
    drop(capture);
    let next = f.worker();
    assert_eq!(f.store.recover_provider_session_voice(&next).unwrap(), 0);
    let resumed = f
        .store
        .resume_provider_session_voice(&next, o.id())
        .unwrap();
    assert_eq!(resumed.id(), o.id());
    assert!(f
        .store
        .begin_provider_session_voice(
            &next,
            &resumed,
            ProviderSessionVoiceAction::Start,
            &manifest()
        )
        .unwrap()
        .is_none());
    assert_eq!(
        f.store
            .begin_provider_session_voice(
                &next,
                &resumed,
                ProviderSessionVoiceAction::End,
                &manifest()
            )
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    assert_eq!(effect_states(&f), vec![("unknown".into(), None)]);
}
#[test]
fn interrupted_submission_recovers_unknown_only_after_original_custody_is_gone() {
    let f = Fixture::new();
    let o = f.origin();
    let w = f.worker();
    let dispatch = f.begin(&w, &o, ProviderSessionVoiceAction::Start);
    assert!(f.store.recover_provider_session_voice(&w).is_err());
    w.retire();
    drop(w);
    assert!(f.store.acquire_provider_session_voice_worker().is_err());
    drop(dispatch);
    let next = f.worker();
    assert_eq!(f.store.recover_provider_session_voice(&next).unwrap(), 1);
    assert_eq!(
        f.store
            .recover_provider_session_voice(&next)
            .unwrap_err()
            .code,
        "conflict"
    );
    let resumed = f
        .store
        .resume_provider_session_voice(&next, o.id())
        .unwrap();
    assert!(f
        .store
        .begin_provider_session_voice(
            &next,
            &resumed,
            ProviderSessionVoiceAction::Start,
            &manifest()
        )
        .unwrap()
        .is_none());
    assert_eq!(effect_states(&f)[0].0, "unknown");
    assert_eq!(
        f.store
            .provider_session_voice_view(&f.keys[0], o.id())
            .unwrap()
            .state,
        ProviderCallState::Unknown
    );
}
#[test]
fn cleanup_has_its_own_once_only_effect_and_original_creation_remains_confirmed() {
    let f = Fixture::new();
    let o = f.origin();
    let w = f.worker();
    let created = f
        .begin(&w, &o, ProviderSessionVoiceAction::Start)
        .capture(Ok(receipt(&o, "in-progress", "created", false)));
    f.store.record_provider_session_voice(&created).unwrap();
    drop(created);
    let end = f.begin(&w, &o, ProviderSessionVoiceAction::End);
    assert_eq!(
        end.existing_resource().unwrap().0,
        "12345678-1234-1234-1234-123456789015"
    );
    assert!(f
        .store
        .begin_provider_session_voice(&w, &o, ProviderSessionVoiceAction::End, &manifest())
        .unwrap()
        .is_none());
    let capture = end.capture(Ok(receipt(&o, "ended", "ended", true)));
    let result = f.store.record_provider_session_voice(&capture).unwrap();
    assert_eq!(result.state, ProviderCallState::Ended);
    assert!(result.closed_at.is_some());
    let states = effect_states(&f);
    assert_eq!(states.len(), 2);
    assert_eq!(states[0], ("confirmed".into(), None));
    assert_eq!(states[1].0, "confirmed");
    assert!(states[1].1.is_some());
    let rev = call_revision(&f, &o);
    f.store.record_provider_session_voice(&capture).unwrap();
    assert_eq!(call_revision(&f, &o), rev);
}
#[test]
fn lost_cleanup_response_is_not_retried_and_status_can_reconcile_original_resource() {
    let f = Fixture::new();
    let o = f.origin();
    let w = f.worker();
    let created = f
        .begin(&w, &o, ProviderSessionVoiceAction::Start)
        .capture(Ok(receipt(&o, "in-progress", "created", false)));
    f.store.record_provider_session_voice(&created).unwrap();
    drop(created);
    drop(f.begin(&w, &o, ProviderSessionVoiceAction::End));
    drop(w);
    let next = f.worker();
    assert_eq!(f.store.recover_provider_session_voice(&next).unwrap(), 1);
    let resumed = f
        .store
        .resume_provider_session_voice(&next, o.id())
        .unwrap();
    assert!(f
        .store
        .begin_provider_session_voice(
            &next,
            &resumed,
            ProviderSessionVoiceAction::End,
            &manifest()
        )
        .unwrap()
        .is_none());
    let poll = f.begin(&next, &resumed, ProviderSessionVoiceAction::Status);
    let capture = poll.capture(Ok(receipt(&resumed, "ended", "reconciled-end", true)));
    assert_eq!(
        f.store
            .record_provider_session_voice(&capture)
            .unwrap()
            .state,
        ProviderCallState::Ended
    );
    assert!(effect_states(&f).iter().all(|s| s.0 == "confirmed"));
}
#[test]
fn observation_replay_and_reordering_do_not_replace_current_status_or_private_capability() {
    let f = Fixture::new();
    let o = f.origin();
    let w = f.worker();
    let start = f
        .begin(&w, &o, ProviderSessionVoiceAction::Start)
        .capture(Ok(receipt(&o, "in-progress", "active", false)));
    f.store.record_provider_session_voice(&start).unwrap();
    drop(start);
    let mut old = receipt(&o, "ringing", "late-ringing", true);
    old.call.control_url = Some("https://stale.invalid/private".into());
    let captured = f
        .begin(&w, &o, ProviderSessionVoiceAction::Status)
        .capture(Ok(old));
    let before = call_revision(&f, &o);
    assert_eq!(
        f.store
            .record_provider_session_voice(&captured)
            .unwrap()
            .state,
        ProviderCallState::Active
    );
    assert_eq!(call_revision(&f, &o), before);
    drop(captured);
    let poll = f.begin(&w, &o, ProviderSessionVoiceAction::Status);
    assert!(!poll
        .existing_resource()
        .unwrap()
        .1
        .unwrap()
        .contains("stale"));
    let capture = poll.capture(Ok(receipt(&o, "in-progress", "same-poll", true)));
    f.store.record_provider_session_voice(&capture).unwrap();
    let rev = call_revision(&f, &o);
    f.store.record_provider_session_voice(&capture).unwrap();
    assert_eq!(call_revision(&f, &o), rev);
}
#[test]
fn mismatched_provider_receipt_cannot_bind_an_external_id_or_cleanup_capability() {
    let f = Fixture::new();
    let o = f.origin();
    let w = f.worker();
    let mut bad = receipt(&o, "in-progress", "bad", false);
    bad.observation.mutation_id = "some-other-call".into();
    let capture = f
        .begin(&w, &o, ProviderSessionVoiceAction::Start)
        .capture(Ok(bad));
    assert_eq!(
        f.store
            .record_provider_session_voice(&capture)
            .unwrap()
            .state,
        ProviderCallState::Unknown
    );
    let c = f.store.connection().unwrap();
    let binding: (Option<String>, Option<String>) = c
        .query_row(
            "SELECT provider_id,control_url FROM provider_session_voice",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(binding, (None, None));
}
#[test]
fn worker_retirement_stops_new_io_but_does_not_erase_an_admitted_historical_result() {
    let f = Fixture::new();
    let o = f.origin();
    let w = f.worker();
    let dispatch = f.begin(&w, &o, ProviderSessionVoiceAction::Start);
    w.retire();
    assert!(!dispatch.current());
    assert!(dispatch.start_request().is_err());
    let captured = dispatch.capture(Ok(receipt(&o, "in-progress", "late-answer", false)));
    assert_eq!(
        f.store
            .record_provider_session_voice(&captured)
            .unwrap()
            .state,
        ProviderCallState::Active
    );
    assert!(f
        .store
        .begin_provider_session_voice(&w, &o, ProviderSessionVoiceAction::Status, &manifest())
        .is_err());
}
#[test]
fn private_call_observations_remain_unavailable_to_uninvited_or_replaced_members() {
    let f = Fixture::new();
    let o = f.origin();
    assert!(f
        .store
        .provider_session_voice_view(&f.keys[2], o.id())
        .is_err());
    use connect_protocol::membership::{MembershipAction, MembershipRequest};
    for (operation_id, change) in [
        (
            "remove-provider-member",
            MembershipAction::Remove {
                participant_id: f.parts[1].clone(),
                reason: Some("epoch boundary fixture".into()),
            },
        ),
        (
            "readmit-provider-member",
            MembershipAction::Add {
                identity_id: o.source.target.identity_id.clone(),
                kind: o.source.target.kind.clone(),
                role: o.source.target.role.clone(),
            },
        ),
    ] {
        let revision = f
            .store
            .membership_snapshot(&f.keys[0], o.conversation_id())
            .unwrap()
            .revision;
        f.store
            .execute(
                &f.keys[0],
                &Operation::Membership {
                    request: MembershipRequest {
                        operation_id: operation_id.into(),
                        conversation_id: o.conversation_id().into(),
                        expected_revision: revision,
                        change,
                    },
                },
            )
            .unwrap();
    }
    let snapshot = f
        .store
        .membership_snapshot(&f.keys[0], o.conversation_id())
        .unwrap();
    let current = snapshot
        .participants
        .iter()
        .find(|m| m.identity_id == o.source.target.identity_id)
        .unwrap();
    assert_eq!(current.participant_id, o.participant_id());
    assert!(current.membership_epoch > o.source.target.membership_epoch);
    assert!(f
        .store
        .provider_session_voice_view(&f.keys[1], o.id())
        .is_err());
}
#[test]
fn immutable_sql_bindings_prevent_retarget_reset_and_retained_identity_deletion() {
    let f = Fixture::new();
    let o = f.origin();
    let w = f.worker();
    let d = f.begin(&w, &o, ProviderSessionVoiceAction::Start);
    drop(d);
    let c = f.store.connection().unwrap();
    for sql in ["UPDATE provider_session_voice SET endpoint_id='other' WHERE id=?1","UPDATE provider_session_voice SET create_started=0,create_effect_id=NULL,attempt_runtime=NULL WHERE id=?1","UPDATE provider_session_voice SET source_json='{}' WHERE id=?1","DELETE FROM provider_session_voice WHERE id=?1"]{assert!(c.execute(sql,[o.id()]).is_err(),"{sql}");}
}
#[test]
fn a_session_target_cannot_be_dialed_by_both_initial_sip_and_provider_control_hosts() {
    let f = Fixture::new();
    let o = f.origin();
    let sip = crate::InitialTelephonePolicy {
        endpoint_id: f.policy.endpoint_id.clone(),
        endpoint_credential_id: f.policy.endpoint_credential_id.clone(),
        route_credential_id: f.policy.route_credential_id.clone(),
        route_reference: f.policy.route_reference.clone(),
        sip_authority: "127.0.0.1:5099".into(),
        timeout_seconds: 30,
    };
    assert!(f
        .store
        .prepare_initial_telephone(&f.keys[0], f.request.clone(), &f.parts[1], sip)
        .is_err());
    assert_eq!(f.count("initial_telephone_operations"), 0);
    assert_eq!(f.count("provider_session_voice"), 1);
    assert_eq!(
        f.store
            .provider_session_voice_view(&f.keys[0], o.id())
            .unwrap()
            .state,
        ProviderCallState::Starting
    );
}
#[test]
fn role_change_preserves_historical_visibility_without_reusing_old_dispatch_authority() {
    let f = Fixture::new();
    let o = f.origin();
    let w = f.worker();
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE participants SET role='observer' WHERE participant_id=?1",
            [&f.parts[1]],
        )
        .unwrap();
    assert!(f
        .store
        .provider_session_voice_view(&f.keys[1], o.id())
        .is_ok());
    assert!(f
        .store
        .begin_provider_session_voice(&w, &o, ProviderSessionVoiceAction::Start, &manifest())
        .is_err());
    assert_eq!(f.count("operation_effects"), 0);
}
#[test]
fn provider_identifier_collision_is_retained_privately_and_never_grants_cross_call_cleanup() {
    resource_collision_case("in-progress");
}
#[test]
fn unattributable_terminal_receipt_never_settles_second_call_or_releases_quarantine() {
    resource_collision_case("ended");
}
#[test]
fn unattributable_uncertain_receipt_stays_quarantined_without_resource_attribution() {
    resource_collision_case("uncertain");
}
fn resource_collision_case(remote_state: &str) {
    let f = Fixture::new();
    let first = f.origin();
    let w = f.worker();
    let created = f
        .begin(&w, &first, ProviderSessionVoiceAction::Start)
        .capture(Ok(receipt(&first, "in-progress", "first", false)));
    f.store.record_provider_session_voice(&created).unwrap();
    drop(created);
    let mut request = f.request.clone();
    request.operation_id = "second-invitation".into();
    request.session_id = "second-session".into();
    f.store
        .execute(
            &f.keys[0],
            &Operation::SessionControl {
                request: request.clone(),
            },
        )
        .unwrap();
    let second = f
        .store
        .prepare_provider_session_voice(&f.keys[0], request, &f.parts[1], f.policy.clone())
        .unwrap();
    let capture = f
        .begin(&w, &second, ProviderSessionVoiceAction::Start)
        .capture(Ok(receipt(&second, remote_state, "second", false)));
    let view = f.store.record_provider_session_voice(&capture).unwrap();
    assert_eq!(view.state, ProviderCallState::Unknown);
    assert_eq!(view.problem.as_deref(), Some("provider-binding-conflict"));
    assert!(view.closed_at.is_none());
    let before = call_revision(&f, &second);
    assert_eq!(
        f.store.record_provider_session_voice(&capture).unwrap(),
        view
    );
    assert_eq!(call_revision(&f, &second), before);
    let c = f.store.connection().unwrap();
    let (resource,control,history):(Option<String>,Option<String>,String)=c.query_row("SELECT provider_id,control_url,quarantine_json FROM provider_session_voice WHERE id=?1",[second.id()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert!(resource.is_none() && control.is_none());
    assert!(history.contains("12345678-1234-1234-1234-123456789015"));
    assert!(history.contains("control"));
    assert!(c
        .execute(
            "UPDATE provider_session_voice SET quarantine_json='{}' WHERE id=?1",
            [second.id()]
        )
        .is_err());
    drop(c);
    assert!(f
        .store
        .begin_provider_session_voice(&w, &second, ProviderSessionVoiceAction::End, &manifest())
        .is_err());
    assert!(f
        .store
        .begin_provider_session_voice(&w, &second, ProviderSessionVoiceAction::Status, &manifest())
        .is_err());
    assert!(f
        .store
        .begin_provider_session_voice(&w, &second, ProviderSessionVoiceAction::Start, &manifest())
        .unwrap()
        .is_none());
    assert_eq!(f.count("operation_effects"), 2);
    let pending = f.store.pending_provider_session_voice(&w).unwrap();
    assert!(pending.iter().any(|o| o.id() == second.id()));
    assert_eq!(
        f.store
            .provider_session_voice_view(&f.keys[0], first.id())
            .unwrap()
            .state,
        ProviderCallState::Active
    );
}
#[test]
fn confirmed_call_cleanup_uses_owned_resource_after_endpoint_authority_is_withdrawn() {
    let f = Fixture::new();
    let o = f.origin();
    let w = f.worker();
    let capture = f
        .begin(&w, &o, ProviderSessionVoiceAction::Start)
        .capture(Ok(receipt(&o, "in-progress", "created", false)));
    f.store.record_provider_session_voice(&capture).unwrap();
    drop(capture);
    f.revoke(1);
    assert!(f
        .store
        .prepare_provider_session_voice(
            &f.keys[0],
            f.request.clone(),
            &f.parts[1],
            f.policy.clone()
        )
        .is_err());
    let end = f.begin(&w, &o, ProviderSessionVoiceAction::End);
    let capture = end.capture(Ok(receipt(&o, "ended", "owned-cleanup", true)));
    assert_eq!(
        f.store
            .record_provider_session_voice(&capture)
            .unwrap()
            .state,
        ProviderCallState::Ended
    );
    assert_eq!(f.count("connections"), 0);
    assert!(f
        .store
        .pending_provider_session_voice(&w)
        .unwrap()
        .is_empty());
}
#[test]
fn original_control_receipt_stays_immutable_while_provider_revision_and_journal_advance() {
    let f = Fixture::new();
    let o = f.origin();
    let w = f.worker();
    let c = f.store.connection().unwrap();
    let before: (String, String, u64, i64) = c
        .query_row(
            "SELECT state,result_json,revision,updated_at FROM operations WHERE record_id=?1",
            [o.invitation_record_id()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    drop(c);
    let captured = f
        .begin(&w, &o, ProviderSessionVoiceAction::Start)
        .capture(Ok(receipt(&o, "in-progress", "started", false)));
    let view = f.store.record_provider_session_voice(&captured).unwrap();
    assert!(view.revision > 1);
    let c = f.store.connection().unwrap();
    let after: (String, String, u64, i64) = c
        .query_row(
            "SELECT state,result_json,revision,updated_at FROM operations WHERE record_id=?1",
            [o.invitation_record_id()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(before, after);
    assert!(c
        .execute(
            "UPDATE operations SET revision=revision+1 WHERE record_id=?1",
            [o.invitation_record_id()]
        )
        .is_err());
    let events: u64 = c
        .query_row(
            "SELECT count(*) FROM journal WHERE kind='provider_voice' AND operation_record_id=?1",
            [o.invitation_record_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(events, 4);
}
#[test]
fn a_reordered_nonterminal_end_response_keeps_cleanup_unknown_until_terminal_status() {
    let f = Fixture::new();
    let o = f.origin();
    let w = f.worker();
    let capture = f
        .begin(&w, &o, ProviderSessionVoiceAction::Start)
        .capture(Ok(receipt(&o, "in-progress", "active", false)));
    f.store.record_provider_session_voice(&capture).unwrap();
    drop(capture);
    let capture = f
        .begin(&w, &o, ProviderSessionVoiceAction::End)
        .capture(Ok(receipt(&o, "ringing", "old-end-status", true)));
    assert_eq!(
        f.store
            .record_provider_session_voice(&capture)
            .unwrap()
            .state,
        ProviderCallState::Active
    );
    assert_eq!(effect_states(&f)[1].0, "unknown");
    drop(capture);
    let capture = f
        .begin(&w, &o, ProviderSessionVoiceAction::Status)
        .capture(Ok(receipt(&o, "ended", "reconciled-terminal", true)));
    assert_eq!(
        f.store
            .record_provider_session_voice(&capture)
            .unwrap()
            .state,
        ProviderCallState::Ended
    );
    assert!(effect_states(&f).iter().all(|s| s.0 == "confirmed"));
}
#[test]
fn assistant_binding_rejects_other_identity_endpoint_and_changed_account() {
    let f = Fixture::new();
    assert!(f.store.provider_session_voice_binding(&f.policy).is_ok());
    let mut wrong = f.policy.clone();
    wrong.assistant_credential_id = f.keys[1].clone();
    assert!(f.store.provider_session_voice_binding(&wrong).is_err());
    let mut wrong = f.policy.clone();
    wrong.assistant_id = "12345678-1234-1234-1234-123456789019".into();
    assert!(f
        .store
        .prepare_provider_session_voice(&f.keys[0], f.request.clone(), &f.parts[1], wrong)
        .is_err());
    let mut wrong = f.policy.clone();
    wrong.route_reference = "OTHER_ACCOUNT".into();
    assert!(f.store.provider_session_voice_binding(&wrong).is_err());
    assert_eq!(f.count("provider_session_voice"), 0);
}
#[test]
fn natural_provider_terminal_observation_confirms_separate_cleanup_without_end_dispatch() {
    let f = Fixture::new();
    let o = f.origin();
    let worker = f.worker();
    let captured = f
        .begin(&worker, &o, ProviderSessionVoiceAction::Start)
        .capture(Ok(receipt(&o, "in-progress", "active", false)));
    f.store.record_provider_session_voice(&captured).unwrap();
    drop(captured);
    let captured = f
        .begin(&worker, &o, ProviderSessionVoiceAction::Status)
        .capture(Ok(receipt(&o, "ended", "natural-terminal", true)));
    let ended = f.store.record_provider_session_voice(&captured).unwrap();
    assert_eq!(ended.state, ProviderCallState::Ended);
    assert!(ended.closed_at.is_some());
    let c = f.store.connection().unwrap();
    let states:Vec<(String,Option<String>)>=c.prepare("SELECT state,compensates_effect_id FROM operation_effects WHERE operation_record_id=?1 ORDER BY compensates_effect_id IS NOT NULL").unwrap().query_map([o.invitation_record_id()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(states.len(), 2);
    assert_eq!(states[0], ("confirmed".into(), None));
    assert_eq!(states[1].0, "confirmed");
    assert!(states[1].1.is_some());
    drop(c);
    assert!(f
        .store
        .begin_provider_session_voice(&worker, &o, ProviderSessionVoiceAction::End, &manifest())
        .unwrap()
        .is_none());
}
#[test]
fn established_provider_uses_own_credentials_and_current_assistant_endpoint() {
    let f = Fixture::new();
    let own = f
        .store
        .issue(&f._ids[0], Scope::operator(), crate::now() + 3600)
        .unwrap()
        .id;
    let mut policy = f.policy.clone();
    policy.assistant_credential_id = own.clone();
    policy.route_credential_id = own;
    let o = f
        .store
        .prepare_provider_session_voice(&f.keys[0], f.request.clone(), &f.parts[1], policy)
        .unwrap();
    let worker = f.worker();
    let captured = f
        .begin(&worker, &o, ProviderSessionVoiceAction::Start)
        .capture(Ok(receipt(&o, "in-progress", "active", false)));
    f.store.record_provider_session_voice(&captured).unwrap();
    drop(captured);
    f.revoke(0);
    assert!(f.store.provider_session_voice_current(&o, false).is_err());
    assert!(f.store.provider_session_voice_current(&o, true).unwrap());
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE endpoints SET revision=revision+1 WHERE id='provider-assistant'",
            [],
        )
        .unwrap();
    assert!(f.store.provider_session_voice_current(&o, true).is_err());
    let captured = f
        .begin(&worker, &o, ProviderSessionVoiceAction::End)
        .capture(Ok(receipt(&o, "ended", "owned-cleanup", false)));
    assert_eq!(
        f.store
            .record_provider_session_voice(&captured)
            .unwrap()
            .state,
        ProviderCallState::Ended
    );
}
#[test]
fn active_provider_refuses_unprojectable_role_change_before_session_mutation() {
    let f = Fixture::new();
    let _o = f.origin();
    let mut r = f.request.clone();
    r.operation_id = "unsupported-provider-role".into();
    r.action = session_control::Action::Role {
        participant_id: f.parts[1].clone(),
        role: "observer".into(),
    };
    assert_eq!(
        f.store
            .execute(&f.keys[0], &Operation::SessionControl { request: r })
            .unwrap_err()
            .code,
        "unsupported-feature"
    );
    let c = f.store.connection().unwrap();
    let role: String = c
        .query_row(
            "SELECT role FROM session_participants WHERE session_id=?1 AND participant_id=?2",
            params![f.request.session_id, f.parts[1]],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(role, "observer");
}

#[test]
fn established_provider_stops_on_observer_membership_or_conversation_close() {
    for close in [false, true] {
        let f = Fixture::new();
        let o = f.origin();
        let worker = f.worker();
        let captured = f
            .begin(&worker, &o, ProviderSessionVoiceAction::Start)
            .capture(Ok(receipt(&o, "in-progress", "active", false)));
        f.store.record_provider_session_voice(&captured).unwrap();
        drop(captured);
        if close {
            let mut end = f.request.clone();
            end.operation_id = "end-before-close-provider-room".into();
            end.expected_revision = Some(
                f.store
                    .session_view(&f.keys[0], o.conversation_id(), o.session_id())
                    .unwrap()
                    .topology_revision,
            );
            end.action = session_control::Action::End {
                reason_code: 200,
                reason: "done".into(),
            };
            f.store
                .execute(&f.keys[0], &Operation::SessionControl { request: end })
                .unwrap();

            f.store
                .execute(
                    &f.keys[0],
                    &Operation::Close {
                        request_id: "close-provider-room".into(),
                        conversation_id: o.conversation_id().into(),
                        options: Default::default(),
                    },
                )
                .unwrap();
        } else {
            let revision = f
                .store
                .membership_snapshot(&f.keys[0], o.conversation_id())
                .unwrap()
                .revision;
            f.store
                .execute(
                    &f.keys[0],
                    &Operation::Membership {
                        request: connect_protocol::membership::MembershipRequest {
                            operation_id: "provider-observer".into(),
                            conversation_id: o.conversation_id().into(),
                            expected_revision: revision,
                            change: connect_protocol::membership::MembershipAction::UpdateRole {
                                participant_id: f.parts[1].clone(),
                                role: "observer".into(),
                            },
                        },
                    },
                )
                .unwrap();
        }
        assert!(!f
            .store
            .provider_session_voice_current(&o, true)
            .unwrap_or(false));
        let captured = f
            .begin(&worker, &o, ProviderSessionVoiceAction::End)
            .capture(Ok(receipt(&o, "ended", "cleanup", false)));
        assert_eq!(
            f.store
                .record_provider_session_voice(&captured)
                .unwrap()
                .state,
            ProviderCallState::Ended
        );
    }
}

fn incoming_policy(
    f: &Fixture,
    w: &Arc<ProviderSessionVoiceWorker>,
    name: &str,
) -> ProviderVoiceIngressPolicy {
    let p = ProviderVoiceIngressPolicy {
        id: name.into(),
        conversation_id: f.request.conversation_id.clone(),
        policy: f.policy.clone(),
    };
    f.store
        .configure_provider_voice_ingress(w, p.clone())
        .unwrap();
    p
}
fn incoming_receipt(
    since: i64,
    id: &str,
    state: &str,
) -> connect_connector_api::InboundVoiceReceipt {
    connect_connector_api::InboundVoiceReceipt {
        call: connect_connector_api::InboundCall {
            provider_id: id.into(),
            assistant_id: "12345678-1234-1234-1234-123456789012".into(),
            phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
            caller: "+15555550111".into(),
            created_at: since,
            state: state.into(),
            control_url: Some(format!("https://fixture.vapi.ai/{id}/control")),
            ended_reason: None,
        },
        event_id: format!("discover:{id}:{state}"),
        state: super::lifecycle::normalized(state).unwrap(),
        verification: Verification::AuthenticatedResponse,
    }
}
fn incoming_import(
    f: &Fixture,
    w: &Arc<ProviderSessionVoiceWorker>,
    p: &ProviderVoiceIngressPolicy,
    id: &str,
    state: &str,
) -> ProviderVoiceImport {
    let scan = f
        .store
        .begin_provider_voice_scan(w, p, &manifest())
        .unwrap();
    let since = scan.query().unwrap().since;
    let captured = scan
        .capture(vec![incoming_receipt(since, id, state)])
        .unwrap();
    f.store.record_provider_voice_scan(&captured).unwrap()
}
#[test]
fn incoming_discovery_uses_generic_invitation_and_observation_without_acceptance_or_create() {
    let f = Fixture::new();
    let w = f.worker();
    f.store.recover_provider_session_voice(&w).unwrap();
    let p = incoming_policy(&f, &w, "incoming");
    let batch = incoming_import(
        &f,
        &w,
        &p,
        "12345678-1234-1234-1234-123456789015",
        "in-progress",
    );
    assert_eq!(batch.imported, 1);
    assert_eq!(batch.calls.len(), 1);
    let o = &batch.calls[0];
    assert_eq!(o.direction(), ProviderCallDirection::Inbound);
    let view = f
        .store
        .provider_session_voice_view(&f.keys[0], o.id())
        .unwrap();
    assert_eq!(view.state, ProviderCallState::Active);
    assert_eq!(view.direction, ProviderCallDirection::Inbound);
    assert!(!view.remote_identity_verified);
    assert!(!view.local_media);
    assert_eq!(f.count("sessions"), 2);
    assert_eq!(f.count("voice_calls"), 0);
    assert_eq!(f.count("connections"), 0);
    let c = f.store.connection().unwrap();
    let appearance: String = c
        .query_row(
            "SELECT state FROM session_participants WHERE session_id=?1 AND participant_id=?2",
            params![o.session_id(), o.participant_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(appearance, "invited");
    let effect: (String, String) = c
        .query_row(
            "SELECT idempotency_mode,state FROM operation_effects WHERE operation_record_id=?1",
            [o.invitation_record_id()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(effect, ("status-query".into(), "confirmed".into()));
    drop(c);
    assert_eq!(
        f.store
            .begin_provider_session_voice(&w, o, ProviderSessionVoiceAction::Start, &manifest())
            .unwrap_err()
            .code,
        "unsupported-connector-operation"
    );
    assert!(f
        .store
        .provider_session_voice_view(&f.keys[2], o.id())
        .is_err());
}
#[test]
fn incoming_duplicate_preserves_invitation_projection_and_private_capability() {
    let f = Fixture::new();
    let w = f.worker();
    let p = incoming_policy(&f, &w, "incoming");
    let first = incoming_import(
        &f,
        &w,
        &p,
        "12345678-1234-1234-1234-123456789015",
        "in-progress",
    );
    let id = first.calls[0].id().to_owned();
    let revision = call_revision(&f, &first.calls[0]);
    let events = f.count("journal");
    let operations = f.count("operations");
    let second = incoming_import(&f, &w, &p, "12345678-1234-1234-1234-123456789015", "ended");
    assert_eq!(second.imported, 0);
    assert_eq!(second.calls[0].id(), id);
    assert_eq!(f.count("operations"), operations);
    assert_eq!(f.count("journal"), events);
    assert_eq!(call_revision(&f, &second.calls[0]), revision);
    assert!(second.scanned_until >= first.scanned_until);
    assert_eq!(
        f.store
            .provider_session_voice_view(&f.keys[0], &id)
            .unwrap()
            .state,
        ProviderCallState::Active
    );
    let status = f.begin(&w, &second.calls[0], ProviderSessionVoiceAction::Status);
    let (id, control) = status.existing_resource().unwrap();
    assert_eq!(id, "12345678-1234-1234-1234-123456789015");
    assert!(control.unwrap().ends_with("/control"));
}
#[test]
fn incoming_batch_capacity_failure_rolls_back_invitation_effects_and_cursor_together() {
    let f = Fixture::limits(Limits {
        connections: 1,
        ..Limits::default()
    });
    let w = f.worker();
    let p = incoming_policy(&f, &w, "incoming");
    let before = (
        f.count("operations"),
        f.count("journal"),
        f.count("sessions"),
    );
    let scan = f
        .store
        .begin_provider_voice_scan(&w, &p, &manifest())
        .unwrap();
    let query = scan.query().unwrap();
    let cursor = query.scanned_until;
    let captured = scan
        .capture(vec![
            incoming_receipt(
                query.since,
                "12345678-1234-1234-1234-123456789015",
                "in-progress",
            ),
            incoming_receipt(
                query.since,
                "12345678-1234-1234-1234-123456789016",
                "in-progress",
            ),
        ])
        .unwrap();
    assert_eq!(
        f.store
            .record_provider_voice_scan(&captured)
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    assert_eq!(
        before,
        (
            f.count("operations"),
            f.count("journal"),
            f.count("sessions")
        )
    );
    assert_eq!(f.count("provider_session_voice"), 0);
    assert_eq!(f.count("operation_effects"), 0);
    let next = f
        .store
        .begin_provider_voice_scan(&w, &p, &manifest())
        .unwrap();
    assert_eq!(next.query().unwrap().scanned_until, cursor);
}
#[test]
fn incoming_replay_rechecks_own_credentials_and_current_epoch_before_duplicate_detection() {
    let f = Fixture::new();
    let w = f.worker();
    let p = incoming_policy(&f, &w, "incoming");
    incoming_import(
        &f,
        &w,
        &p,
        "12345678-1234-1234-1234-123456789015",
        "in-progress",
    );
    let scan = f
        .store
        .begin_provider_voice_scan(&w, &p, &manifest())
        .unwrap();
    let query = scan.query().unwrap();
    let captured = scan
        .capture(vec![incoming_receipt(
            query.since,
            "12345678-1234-1234-1234-123456789015",
            "ended",
        )])
        .unwrap();
    f.revoke(1);
    assert!(f.store.record_provider_voice_scan(&captured).is_err());
    assert!(f
        .store
        .begin_provider_voice_scan(&w, &p, &manifest())
        .is_err());
    assert_eq!(f.count("provider_session_voice"), 1);
}
#[test]
fn incoming_normalization_and_missing_discovery_refuse_before_import_or_cursor_advance() {
    let f = Fixture::new();
    let w = f.worker();
    let p = incoming_policy(&f, &w, "incoming");
    let mut m = manifest();
    m.operations
        .retain(|op| *op != ConnectorOperation::VoiceDiscover);
    assert_eq!(
        f.store
            .begin_provider_voice_scan(&w, &p, &m)
            .unwrap_err()
            .code,
        "unsupported-connector-operation"
    );
    let scan = f
        .store
        .begin_provider_voice_scan(&w, &p, &manifest())
        .unwrap();
    let since = scan.query().unwrap().since;
    let mut receipt =
        incoming_receipt(since, "12345678-1234-1234-1234-123456789015", "in-progress");
    receipt.verification = Verification::LocalAttempt;
    assert!(scan.capture(vec![receipt]).is_err());
    assert_eq!(f.count("provider_session_voice"), 0);
}
#[test]
fn incoming_recovery_keeps_original_resource_and_cursor_without_creation_permission() {
    let f = Fixture::new();
    let w = f.worker();
    let p = incoming_policy(&f, &w, "incoming");
    let batch = incoming_import(
        &f,
        &w,
        &p,
        "12345678-1234-1234-1234-123456789015",
        "in-progress",
    );
    let id = batch.calls[0].id().to_owned();
    w.retire();
    drop(w);
    let next = f.worker();
    f.store.recover_provider_session_voice(&next).unwrap();
    f.store
        .configure_provider_voice_ingress(&next, p.clone())
        .unwrap();
    let pending = f.store.pending_provider_session_voice(&next).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id(), id);
    assert_eq!(pending[0].direction(), ProviderCallDirection::Inbound);
    let duplicate = incoming_import(
        &f,
        &next,
        &p,
        "12345678-1234-1234-1234-123456789015",
        "ended",
    );
    assert_eq!(duplicate.imported, 0);
    assert!(duplicate.scanned_until >= batch.scanned_until);
    let captured = f
        .begin(&next, &pending[0], ProviderSessionVoiceAction::End)
        .capture(Ok(receipt(&pending[0], "ended", "original-cleanup", false)));
    assert_eq!(
        f.store
            .record_provider_session_voice(&captured)
            .unwrap()
            .state,
        ProviderCallState::Ended
    );
}
#[test]
fn incoming_observation_cannot_rebind_an_owned_outbound_provider_resource() {
    let f = Fixture::new();
    let w = f.worker();
    let o = f.origin();
    let captured = f
        .begin(&w, &o, ProviderSessionVoiceAction::Start)
        .capture(Ok(receipt(&o, "in-progress", "outbound", false)));
    f.store.record_provider_session_voice(&captured).unwrap();
    drop(captured);
    let p = incoming_policy(&f, &w, "incoming");
    let before = (f.count("operations"), f.count("journal"));
    let scan = f
        .store
        .begin_provider_voice_scan(&w, &p, &manifest())
        .unwrap();
    let q = scan.query().unwrap();
    let captured = scan
        .capture(vec![incoming_receipt(
            q.since,
            "12345678-1234-1234-1234-123456789015",
            "ended",
        )])
        .unwrap();
    assert_eq!(
        f.store
            .record_provider_voice_scan(&captured)
            .unwrap_err()
            .code,
        "provider-binding-conflict"
    );
    assert_eq!(before, (f.count("operations"), f.count("journal")));
    assert_eq!(f.count("provider_session_voice"), 1);
    assert_eq!(
        f.store
            .provider_session_voice_view(&f.keys[0], o.id())
            .unwrap()
            .direction,
        ProviderCallDirection::Outbound
    );
}
#[test]
fn incoming_terminal_observation_confirms_original_absence_without_end_submission() {
    let f = Fixture::new();
    let w = f.worker();
    let p = incoming_policy(&f, &w, "incoming");
    let batch = incoming_import(&f, &w, &p, "12345678-1234-1234-1234-123456789015", "ended");
    let o = &batch.calls[0];
    let view = f
        .store
        .provider_session_voice_view(&f.keys[0], o.id())
        .unwrap();
    assert_eq!(view.state, ProviderCallState::Ended);
    assert!(view.closed_at.is_some());
    assert_eq!(f.count("operation_effects"), 2);
    assert!(f
        .store
        .begin_provider_session_voice(&w, o, ProviderSessionVoiceAction::End, &manifest())
        .unwrap()
        .is_none());
}

#[test]
fn incoming_private_provider_reference_or_control_uri_in_reason_never_reaches_snapshot_or_journal()
{
    let f = Fixture::new();
    let w = f.worker();
    let p = incoming_policy(&f, &w, "incoming");
    let scan = f
        .store
        .begin_provider_voice_scan(&w, &p, &manifest())
        .unwrap();
    let since = scan.query().unwrap().since;
    let mut receipt = incoming_receipt(since, "12345678-1234-1234-1234-123456789015", "ended");
    let private = receipt.call.control_url.clone().unwrap();
    receipt.call.ended_reason = Some(private.clone());
    let captured = scan.capture(vec![receipt]).unwrap();
    let batch = f.store.record_provider_voice_scan(&captured).unwrap();
    let view = f
        .store
        .provider_session_voice_view(&f.keys[0], batch.calls[0].id())
        .unwrap();
    assert!(view.ended_reason.is_none());
    let encoded = serde_json::to_string(&view).unwrap();
    assert!(!encoded.contains("12345678-1234-1234-1234-123456789015"));
    assert!(!encoded.contains("/control"));
    let c = f.store.connection().unwrap();
    let saved: String = c
        .query_row(
            "SELECT ended_reason FROM provider_session_voice WHERE id=?1",
            [batch.calls[0].id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(saved, private);
    let journal: String = c
        .query_row(
            "SELECT data_json FROM journal WHERE kind='provider_voice' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!journal.contains("/control"));
    assert!(!journal.contains("12345678-1234-1234-1234-123456789015"));
}

#[test]
fn incoming_unconfigured_caller_cannot_create_principals_membership_or_session_history() {
    let f = Fixture::new();
    let w = f.worker();
    let p = incoming_policy(&f, &w, "incoming");
    let before = (
        f.count("identities"),
        f.count("participants"),
        f.count("sessions"),
        f.count("journal"),
    );
    let scan = f
        .store
        .begin_provider_voice_scan(&w, &p, &manifest())
        .unwrap();
    let q = scan.query().unwrap();
    let mut r = incoming_receipt(
        q.since,
        "12345678-1234-1234-1234-123456789015",
        "in-progress",
    );
    r.call.caller = "+15555550999".into();
    let captured = scan.capture(vec![r]).unwrap();
    let batch = f.store.record_provider_voice_scan(&captured).unwrap();
    assert_eq!(batch.imported, 0);
    assert!(batch.calls.is_empty());
    assert_eq!(
        before,
        (
            f.count("identities"),
            f.count("participants"),
            f.count("sessions"),
            f.count("journal")
        )
    );
    assert!(batch.scanned_until >= q.scanned_until);
}
#[test]
fn incoming_readmission_cannot_reuse_a_configured_epoch_even_for_an_existing_provider_id() {
    let f = Fixture::new();
    let w = f.worker();
    let p = incoming_policy(&f, &w, "incoming");
    let batch = incoming_import(
        &f,
        &w,
        &p,
        "12345678-1234-1234-1234-123456789015",
        "in-progress",
    );
    let o = &batch.calls[0];
    let scan = f
        .store
        .begin_provider_voice_scan(&w, &p, &manifest())
        .unwrap();
    let q = scan.query().unwrap();
    let captured = scan
        .capture(vec![incoming_receipt(
            q.since,
            "12345678-1234-1234-1234-123456789015",
            "ended",
        )])
        .unwrap();
    use connect_protocol::membership::{MembershipAction, MembershipRequest};
    for (operation_id, change) in [
        (
            "remove-incoming-member",
            MembershipAction::Remove {
                participant_id: f.parts[1].clone(),
                reason: Some("epoch boundary".into()),
            },
        ),
        (
            "readmit-incoming-member",
            MembershipAction::Add {
                identity_id: f._ids[1].clone(),
                kind: connect_protocol::provisioning::IdentityKind::Human,
                role: "participant".into(),
            },
        ),
    ] {
        let revision = f
            .store
            .membership_snapshot(&f.keys[0], o.conversation_id())
            .unwrap()
            .revision;
        f.store
            .execute(
                &f.keys[0],
                &Operation::Membership {
                    request: MembershipRequest {
                        operation_id: operation_id.into(),
                        conversation_id: o.conversation_id().into(),
                        expected_revision: revision,
                        change,
                    },
                },
            )
            .unwrap();
    }
    assert!(f.store.record_provider_voice_scan(&captured).is_err());
    assert!(f
        .store
        .begin_provider_voice_scan(&w, &p, &manifest())
        .is_err());
    assert!(f
        .store
        .provider_session_voice_view(&f.keys[1], o.id())
        .is_err());
    assert_eq!(f.count("provider_session_voice"), 1);
    let c = f.store.connection().unwrap();
    let cursor: i64 = c
        .query_row(
            "SELECT scanned_until FROM provider_voice_ingress WHERE id=?1",
            [&p.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cursor, q.scanned_until);
}
#[test]
fn incoming_already_terminal_history_does_not_require_an_active_call_slot() {
    let f = Fixture::limits(Limits {
        connections: 1,
        ..Limits::default()
    });
    let w = f.worker();
    let o = f.origin();
    let captured = f
        .begin(&w, &o, ProviderSessionVoiceAction::Start)
        .capture(Ok(receipt(&o, "in-progress", "live-outbound", false)));
    f.store.record_provider_session_voice(&captured).unwrap();
    drop(captured);
    let p = incoming_policy(&f, &w, "incoming");
    let batch = incoming_import(&f, &w, &p, "12345678-1234-1234-1234-123456789016", "ended");
    assert_eq!(batch.imported, 1);
    assert_eq!(f.count("provider_session_voice"), 2);
    assert!(f
        .store
        .provider_session_voice_view(&f.keys[0], batch.calls[0].id())
        .unwrap()
        .closed_at
        .is_some());
    let c = f.store.connection().unwrap();
    let active: u64 = c
        .query_row(
            "SELECT count(*) FROM provider_session_voice WHERE closed_at IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(active, 1);
}

#[test]
fn uncertain_creation_correlation_after_restart_keeps_original_binding_and_cleans_after_revocation()
{
    let f = Fixture::new();
    let o = f.origin();
    let w = f.worker();
    let original: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT result_json FROM operations WHERE record_id=?1",
            [o.invitation_record_id()],
            |r| r.get(0),
        )
        .unwrap();
    let c = f
        .begin(&w, &o, ProviderSessionVoiceAction::Start)
        .capture(Err(Failure::Uncertain));
    f.store.record_provider_session_voice(&c).unwrap();
    drop(c);
    w.retire();
    drop(w);
    let next = f.worker();
    f.store.recover_provider_session_voice(&next).unwrap();
    let resumed = f
        .store
        .resume_provider_session_voice(&next, o.id())
        .unwrap();
    f.revoke(0);
    let dispatch = f.begin(&next, &resumed, ProviderSessionVoiceAction::Reconcile);
    let q = dispatch.reconcile_query().unwrap();
    assert_eq!(q.request.assistant_id, o.policy().assistant_id);
    assert_eq!(q.request.phone_number_id, o.policy().phone_number_id);
    assert_eq!(q.request.destination, "+15555550111");
    assert_eq!(q.until, o.expires_at() + 60);
    assert_eq!(
        q.since,
        o.expires_at() - i64::from(o.policy().timeout_seconds) - 60
    );
    let c = dispatch.capture(Ok(receipt(
        &resumed,
        "in-progress",
        "original-correlated",
        true,
    )));
    assert_eq!(
        f.store.record_provider_session_voice(&c).unwrap().state,
        ProviderCallState::Active
    );
    assert!(f
        .store
        .provider_session_voice_view(&f.keys[0], o.id())
        .is_err());
    let end = f.begin(&next, &resumed, ProviderSessionVoiceAction::End);
    assert_eq!(
        end.existing_resource().unwrap().0,
        "12345678-1234-1234-1234-123456789015"
    );
    let ended = end.capture(Ok(receipt(
        &resumed,
        "ended",
        "original-correlated-end",
        true,
    )));
    assert_eq!(
        f.store.record_provider_session_voice(&ended).unwrap().state,
        ProviderCallState::Ended
    );
    assert_eq!(
        effect_states(&f)
            .iter()
            .map(|s| s.0.as_str())
            .collect::<Vec<_>>(),
        vec!["confirmed", "confirmed"]
    );
    let current: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT result_json FROM operations WHERE record_id=?1",
            [o.invitation_record_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(original, current);
}
#[test]
fn correlation_requires_declared_capability_and_a_settled_unknown_submission() {
    let f = Fixture::new();
    let o = f.origin();
    let w = f.worker();
    assert!(f
        .store
        .begin_provider_session_voice(&w, &o, ProviderSessionVoiceAction::Reconcile, &manifest())
        .is_err());
    let start = f.begin(&w, &o, ProviderSessionVoiceAction::Start);
    assert!(f
        .store
        .begin_provider_session_voice(&w, &o, ProviderSessionVoiceAction::Reconcile, &manifest())
        .is_err());
    let c = start.capture(Err(Failure::Uncertain));
    f.store.record_provider_session_voice(&c).unwrap();
    drop(c);
    let mut unsupported = manifest();
    unsupported
        .operations
        .remove(&ConnectorOperation::VoiceReconcile);
    assert_eq!(
        f.store
            .begin_provider_session_voice(
                &w,
                &o,
                ProviderSessionVoiceAction::Reconcile,
                &unsupported
            )
            .unwrap_err()
            .code,
        "unsupported-connector-operation"
    );
    let c = f
        .begin(&w, &o, ProviderSessionVoiceAction::Reconcile)
        .capture(Ok(receipt(&o, "in-progress", "found", true)));
    f.store.record_provider_session_voice(&c).unwrap();
    assert!(f
        .store
        .begin_provider_session_voice(&w, &o, ProviderSessionVoiceAction::Reconcile, &manifest())
        .is_err());
    assert!(f
        .store
        .begin_provider_session_voice(&w, &o, ProviderSessionVoiceAction::Start, &manifest())
        .unwrap()
        .is_none());
    assert_eq!(f.count("operation_effects"), 1);
}
#[test]
fn failed_or_uncorrelated_lookup_cannot_settle_unknown_or_emit_a_new_provider_fact() {
    let f = Fixture::new();
    let o = f.origin();
    let w = f.worker();
    let c = f
        .begin(&w, &o, ProviderSessionVoiceAction::Start)
        .capture(Err(Failure::Uncertain));
    f.store.record_provider_session_voice(&c).unwrap();
    drop(c);
    let revision = call_revision(&f, &o);
    let journal = f.count("journal");
    for response in [
        Err(Failure::Rejected),
        Err(Failure::Uncertain),
        Ok(receipt(&o, "in-progress", "not-correlated", false)),
    ] {
        let c = f
            .begin(&w, &o, ProviderSessionVoiceAction::Reconcile)
            .capture(response);
        assert_eq!(
            f.store.record_provider_session_voice(&c).unwrap().state,
            ProviderCallState::Unknown
        );
        assert_eq!(call_revision(&f, &o), revision);
        assert_eq!(f.count("journal"), journal);
        assert_eq!(effect_states(&f), vec![("unknown".into(), None)]);
    }
}
#[test]
fn correlated_terminal_resource_settles_creation_and_cleanup_once_without_end_dispatch() {
    let f = Fixture::new();
    let o = f.origin();
    let w = f.worker();
    let c = f
        .begin(&w, &o, ProviderSessionVoiceAction::Start)
        .capture(Err(Failure::Uncertain));
    f.store.record_provider_session_voice(&c).unwrap();
    drop(c);
    let c = f
        .begin(&w, &o, ProviderSessionVoiceAction::Reconcile)
        .capture(Ok(receipt(&o, "ended", "found-terminal", true)));
    let v = f.store.record_provider_session_voice(&c).unwrap();
    assert_eq!(v.state, ProviderCallState::Ended);
    assert!(v.closed_at.is_some());
    let revision = call_revision(&f, &o);
    f.store.record_provider_session_voice(&c).unwrap();
    assert_eq!(call_revision(&f, &o), revision);
    assert!(f
        .store
        .begin_provider_session_voice(&w, &o, ProviderSessionVoiceAction::End, &manifest())
        .unwrap()
        .is_none());
    assert_eq!(
        effect_states(&f)
            .iter()
            .map(|s| s.0.as_str())
            .collect::<Vec<_>>(),
        vec!["confirmed", "confirmed"]
    );
}

#[test]
fn correlated_resource_collision_quarantines_and_a_stale_lookup_cannot_overwrite_that_conflict() {
    let f = Fixture::new();
    let first = f.origin();
    let w = f.worker();
    let c = f
        .begin(&w, &first, ProviderSessionVoiceAction::Start)
        .capture(Ok(receipt(&first, "in-progress", "first-owned", false)));
    f.store.record_provider_session_voice(&c).unwrap();
    drop(c);
    let mut request = f.request.clone();
    request.operation_id = "collision-invite".into();
    request.session_id = "collision-session".into();
    f.store
        .execute(
            &f.keys[0],
            &Operation::SessionControl {
                request: request.clone(),
            },
        )
        .unwrap();
    let second = f
        .store
        .prepare_provider_session_voice(&f.keys[0], request, &f.parts[1], f.policy.clone())
        .unwrap();
    let c = f
        .begin(&w, &second, ProviderSessionVoiceAction::Start)
        .capture(Err(Failure::Uncertain));
    f.store.record_provider_session_voice(&c).unwrap();
    drop(c);
    let stale = f.begin(&w, &second, ProviderSessionVoiceAction::Reconcile);
    let collision = f
        .begin(&w, &second, ProviderSessionVoiceAction::Reconcile)
        .capture(Ok(receipt(&second, "ended", "collision", true)));
    let v = f.store.record_provider_session_voice(&collision).unwrap();
    assert_eq!(v.state, ProviderCallState::Unknown);
    assert_eq!(v.problem.as_deref(), Some("provider-binding-conflict"));
    assert!(v.closed_at.is_none());
    let mut other = receipt(&second, "in-progress", "later-different-correlation", true);
    other.call.id = "12345678-1234-1234-1234-123456789016".into();
    other.observation.external_id = Some(other.call.id.clone());
    other.call.control_url = Some(format!("https://fixture.vapi.ai/{}/control", other.call.id));
    let c = stale.capture(Ok(other));
    assert_eq!(
        f.store.record_provider_session_voice(&c).unwrap_err().code,
        "reconciliation-required"
    );
    assert!(f
        .store
        .begin_provider_session_voice(
            &w,
            &second,
            ProviderSessionVoiceAction::Reconcile,
            &manifest()
        )
        .is_err());
    assert!(f
        .store
        .begin_provider_session_voice(&w, &second, ProviderSessionVoiceAction::End, &manifest())
        .is_err());
    let c = f.store.connection().unwrap();
    let (provider, control): (Option<String>, Option<String>) = c
        .query_row(
            "SELECT provider_id,control_url FROM provider_session_voice WHERE id=?1",
            [second.id()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(provider.is_none() && control.is_none());
}

fn quarantined_second(
    f: &Fixture,
    w: &Arc<ProviderSessionVoiceWorker>,
) -> ProviderSessionVoiceOrigin {
    let first = f.origin();
    let c = f
        .begin(w, &first, ProviderSessionVoiceAction::Start)
        .capture(Ok(receipt(&first, "in-progress", "first-owned", false)));
    f.store.record_provider_session_voice(&c).unwrap();
    drop(c);
    let mut request = f.request.clone();
    request.operation_id = "quarantined-invitation".into();
    request.session_id = "quarantined-session".into();
    f.store
        .execute(
            &f.keys[0],
            &Operation::SessionControl {
                request: request.clone(),
            },
        )
        .unwrap();
    let o = f
        .store
        .prepare_provider_session_voice(&f.keys[0], request, &f.parts[1], f.policy.clone())
        .unwrap();
    let c = f
        .begin(w, &o, ProviderSessionVoiceAction::Start)
        .capture(Ok(receipt(&o, "ended", "foreign-terminal", false)));
    let v = f.store.record_provider_session_voice(&c).unwrap();
    assert_eq!(v.problem.as_deref(), Some("provider-binding-conflict"));
    o
}
fn resolution_receipt(o: &ProviderSessionVoiceOrigin, state: &str) -> VoiceReceipt {
    let mut r = receipt(
        o,
        state,
        &format!("original-correlated-resolution:{state}"),
        true,
    );
    r.call.id = "12345678-1234-1234-1234-123456789016".into();
    r.observation.external_id = Some(r.call.id.clone());
    r.call.control_url = Some(format!("https://fixture.vapi.ai/{}/control", r.call.id));
    r
}
fn quarantine_history(f: &Fixture, o: &ProviderSessionVoiceOrigin) -> String {
    f.store
        .connection()
        .unwrap()
        .query_row(
            "SELECT quarantine_json FROM provider_session_voice WHERE id=?1",
            [o.id()],
            |r| r.get(0),
        )
        .unwrap()
}
#[test]
fn typed_quarantine_resolution_after_restart_retains_original_conflict_and_owns_only_the_distinct_resource(
) {
    let f = Fixture::new();
    let w = f.worker();
    let o = quarantined_second(&f, &w);
    let history = quarantine_history(&f, &o);
    let original: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT result_json FROM operations WHERE record_id=?1",
            [o.invitation_record_id()],
            |r| r.get(0),
        )
        .unwrap();
    w.retire();
    drop(w);
    let next = f.worker();
    f.store.recover_provider_session_voice(&next).unwrap();
    let resumed = f
        .store
        .resume_provider_session_voice(&next, o.id())
        .unwrap();
    f.revoke(0);
    let c = f
        .begin(
            &next,
            &resumed,
            ProviderSessionVoiceAction::ResolveQuarantine,
        )
        .capture(Ok(resolution_receipt(&resumed, "in-progress")));
    let v = f.store.record_provider_session_voice(&c).unwrap();
    assert_eq!(v.state, ProviderCallState::Active);
    assert!(v.problem.is_none());
    let revision = v.revision;
    f.store.record_provider_session_voice(&c).unwrap();
    assert_eq!(call_revision(&f, &o), revision);
    assert_eq!(quarantine_history(&f, &o), history);
    assert_eq!(f.count("provider_voice_quarantine_resolutions"), 1);
    assert_eq!(f.count("operation_effects"), 2);
    assert!(f
        .store
        .provider_session_voice_view(&f.keys[0], o.id())
        .is_err());
    let end = f.begin(&next, &resumed, ProviderSessionVoiceAction::End);
    assert_eq!(
        end.existing_resource().unwrap().0,
        "12345678-1234-1234-1234-123456789016"
    );
    let c = end.capture(Ok(resolution_receipt(&resumed, "ended")));
    f.store.record_provider_session_voice(&c).unwrap();
    let current: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT result_json FROM operations WHERE record_id=?1",
            [o.invitation_record_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(original, current);
    let first: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT provider_id FROM provider_session_voice WHERE id!=?1",
            [o.id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(first, "12345678-1234-1234-1234-123456789015");
}
#[test]
fn missing_uncorrelated_or_still_claimed_resolution_proofs_leave_the_original_quarantine_unchanged()
{
    let f = Fixture::new();
    let w = f.worker();
    let first = f.origin();
    assert!(f
        .store
        .begin_provider_session_voice(
            &w,
            &first,
            ProviderSessionVoiceAction::ResolveQuarantine,
            &manifest()
        )
        .is_err());
    let o = quarantined_second(&f, &w);
    let history = quarantine_history(&f, &o);
    let before = call_revision(&f, &o);
    let journal = f.count("journal");
    let mut uncorrelated = resolution_receipt(&o, "in-progress");
    uncorrelated.observation.verification = Verification::AuthenticatedResponse;
    for r in [
        Err(Failure::Uncertain),
        Err(Failure::Rejected),
        Ok(uncorrelated),
        Ok(receipt(&o, "ended", "still-other-call", true)),
    ] {
        let c = f
            .begin(&w, &o, ProviderSessionVoiceAction::ResolveQuarantine)
            .capture(r);
        let v = f.store.record_provider_session_voice(&c).unwrap();
        assert_eq!(v.state, ProviderCallState::Unknown);
        assert_eq!(v.problem.as_deref(), Some("provider-binding-conflict"));
        assert_eq!(call_revision(&f, &o), before);
        assert_eq!(quarantine_history(&f, &o), history);
        assert_eq!(f.count("journal"), journal);
        assert_eq!(f.count("provider_voice_quarantine_resolutions"), 0);
    }
    let mut unsupported = manifest();
    unsupported
        .operations
        .remove(&ConnectorOperation::VoiceReconcile);
    assert_eq!(
        f.store
            .begin_provider_session_voice(
                &w,
                &o,
                ProviderSessionVoiceAction::ResolveQuarantine,
                &unsupported
            )
            .unwrap_err()
            .code,
        "unsupported-connector-operation"
    );
    assert!(f
        .store
        .begin_provider_session_voice(&w, &o, ProviderSessionVoiceAction::End, &manifest())
        .is_err());
}
#[test]
fn resolution_schema_refuses_implicit_binding_unverified_proof_or_mutable_resolution_history() {
    let f = Fixture::new();
    let w = f.worker();
    let o = quarantined_second(&f, &w);
    let raw = quarantine_history(&f, &o);
    let db = f.store.connection().unwrap();
    assert!(db
        .execute(
            "UPDATE provider_session_voice SET provider_id='unproven-resource' WHERE id=?1",
            [o.id()]
        )
        .is_err());
    let bad = json!({"provider_id":"unproven-resource","observation":{"mutation_id":o.id(),"external_id":"unproven-resource","verification":"AuthenticatedResponse","state":{"Voice":"Active"}}});
    assert!(db.execute("INSERT INTO provider_voice_quarantine_resolutions(call_id,quarantine_json,provider_id,proof_json,resolved_at) VALUES(?1,?2,'unproven-resource',?3,?4)",params![o.id(),raw,bad.to_string(),crate::now()]).is_err());
    drop(db);
    let c = f
        .begin(&w, &o, ProviderSessionVoiceAction::ResolveQuarantine)
        .capture(Ok(resolution_receipt(&o, "in-progress")));
    f.store.record_provider_session_voice(&c).unwrap();
    let db = f.store.connection().unwrap();
    assert!(db.execute("UPDATE provider_voice_quarantine_resolutions SET provider_id='changed' WHERE call_id=?1",[o.id()]).is_err());
    assert!(db
        .execute(
            "DELETE FROM provider_voice_quarantine_resolutions WHERE call_id=?1",
            [o.id()]
        )
        .is_err());
    assert!(db
        .execute(
            "UPDATE provider_session_voice SET quarantine_json='{}' WHERE id=?1",
            [o.id()]
        )
        .is_err());
}
#[test]
fn failed_resolution_commit_rolls_back_proof_effect_projection_and_journal_before_read_only_replay()
{
    let f = Fixture::new();
    let w = f.worker();
    let o = quarantined_second(&f, &w);
    let before = call_revision(&f, &o);
    let journal = f.count("journal");
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER resolution_fail_fixture BEFORE UPDATE OF provider_id ON provider_session_voice WHEN NEW.quarantine_json IS NOT NULL AND NEW.provider_id IS NOT NULL BEGIN SELECT RAISE(ABORT,'fixture durable commit failure');END;").unwrap();
    let c = f
        .begin(&w, &o, ProviderSessionVoiceAction::ResolveQuarantine)
        .capture(Ok(resolution_receipt(&o, "in-progress")));
    assert!(f.store.record_provider_session_voice(&c).is_err());
    assert_eq!(f.count("provider_voice_quarantine_resolutions"), 0);
    assert_eq!(call_revision(&f, &o), before);
    assert_eq!(f.count("journal"), journal);
    assert_eq!(
        effect_states(&f)
            .iter()
            .filter(|s| s.0 == "unknown")
            .count(),
        1
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER resolution_fail_fixture;")
        .unwrap();
    assert_eq!(
        f.store.record_provider_session_voice(&c).unwrap().state,
        ProviderCallState::Active
    );
    assert_eq!(f.count("provider_voice_quarantine_resolutions"), 1);
}
#[test]
fn competing_resolution_captures_cannot_retarget_or_reopen_the_first_confirmed_resource() {
    let f = Fixture::new();
    let w = f.worker();
    let o = quarantined_second(&f, &w);
    let first = f
        .begin(&w, &o, ProviderSessionVoiceAction::ResolveQuarantine)
        .capture(Ok(resolution_receipt(&o, "in-progress")));
    let mut alternative = resolution_receipt(&o, "in-progress");
    alternative.call.id = "12345678-1234-1234-1234-123456789017".into();
    alternative.observation.external_id = Some(alternative.call.id.clone());
    alternative.call.control_url = Some(format!(
        "https://fixture.vapi.ai/{}/control",
        alternative.call.id
    ));
    let stale = f
        .begin(&w, &o, ProviderSessionVoiceAction::ResolveQuarantine)
        .capture(Ok(alternative));
    f.store.record_provider_session_voice(&first).unwrap();
    assert!(f.store.record_provider_session_voice(&stale).is_err());
    let c = f
        .begin(&w, &o, ProviderSessionVoiceAction::Status)
        .capture(Ok(resolution_receipt(&o, "ended")));
    f.store.record_provider_session_voice(&c).unwrap();
    let revision = call_revision(&f, &o);
    assert_eq!(
        f.store.record_provider_session_voice(&first).unwrap().state,
        ProviderCallState::Ended
    );
    assert_eq!(call_revision(&f, &o), revision);
    assert_eq!(f.count("provider_voice_quarantine_resolutions"), 1);
}
#[test]
fn terminal_quarantine_resolution_confirms_absence_without_end_and_redacts_retained_private_ids() {
    let f = Fixture::new();
    let w = f.worker();
    let o = quarantined_second(&f, &w);
    let mut r = resolution_receipt(&o, "ended");
    r.call.ended_reason =
        Some("old provider reference 12345678-1234-1234-1234-123456789015".into());
    let c = f
        .begin(&w, &o, ProviderSessionVoiceAction::ResolveQuarantine)
        .capture(Ok(r));
    let v = f.store.record_provider_session_voice(&c).unwrap();
    assert_eq!(v.state, ProviderCallState::Ended);
    assert!(v.closed_at.is_some() && v.problem.is_none() && v.ended_reason.is_none());
    assert!(!v.remote_identity_verified && !v.local_media);
    assert!(f
        .store
        .begin_provider_session_voice(&w, &o, ProviderSessionVoiceAction::End, &manifest())
        .unwrap()
        .is_none());
    assert_eq!(f.count("operation_effects"), 3);
    let db = f.store.connection().unwrap();
    let mut q = db
        .prepare("SELECT data_json FROM journal WHERE kind='provider_voice'")
        .unwrap();
    let events: Vec<String> = q
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    for raw in events {
        assert!(!raw.contains("12345678-1234-1234-1234-123456789015"));
        assert!(!raw.contains("12345678-1234-1234-1234-123456789016"));
        assert!(!raw.contains("/control"));
    }
}
