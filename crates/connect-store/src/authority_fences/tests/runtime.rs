use super::*;
use crate::{Route, SmsGrant, SmsGrantSnapshot, VoiceAction, VoiceRoute};
use connect_connector_api::lifecycle::*;
use connect_protocol::messages::{MessageAudience, MessageDraft};

fn conversation(f: &Fixture, key: &str, id: &str, members: Vec<String>) -> String {
    let Outcome::Conversation(c) = f
        .store
        .execute(
            key,
            &Operation::Create {
                request_id: id.into(),
                request: connect_protocol::conversations::CreateRequest::persistent(members),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    c.id
}
fn wait_read<T: Send + 'static>(
    f: &Fixture,
    cid: &str,
    write: impl FnOnce(Store) -> connect_protocol::Result<T> + Send + 'static,
) -> T {
    wait_read_at(f, cid, false, write)
}
fn wait_read_at<T: Send + 'static>(
    f: &Fixture,
    cid: &str,
    conversation_probe: bool,
    write: impl FnOnce(Store) -> connect_protocol::Result<T> + Send + 'static,
) -> T {
    let read = f
        .store
        .read_authority(
            &f.user,
            Some(cid),
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    let first = f
        .store
        .membership_snapshot(&f.admin, cid)
        .unwrap()
        .participants
        .into_iter()
        .map(|m| m.identity_id)
        .min()
        .unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let peer = f.peer();
    let worker = std::thread::spawn(move || write(peer));
    let probe = if conversation_probe {
        AuthorityScope::conversation(cid, AuthorityMode::Write).unwrap()
    } else {
        AuthorityScope::identity(&first, AuthorityMode::Write).unwrap()
    };
    waiting(&f.store, &probe);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    drop(read);
    worker.join().unwrap().unwrap()
}
fn provider() -> (Fixture, String, Route) {
    let f = Fixture::new();
    let bot = f.store.create_identity("bot").unwrap().id;
    let key = f
        .store
        .issue(&bot, Scope::operator(), crate::now() + 600)
        .unwrap();
    let cid = conversation(&f, &f.admin, "conversation", vec![bot.clone()]);
    let route = Route {
        id: "chat".into(),
        identity_id: bot,
        credential_id: key.id,
        conversation_id: cid.clone(),
        connector: "vapi".into(),
        target: uuid::Uuid::new_v4().to_string(),
        source: None,
        enabled: true,
        incoming_to: MessageAudience::all(),
    };
    f.store.configure_route(&f.admin, &route).unwrap();
    f.store
        .execute(
            &f.admin,
            &Operation::Send {
                request_id: "question".into(),
                conversation_id: cid.clone(),
                message: MessageDraft::text_all("a held question"),
            },
        )
        .unwrap();
    (f, cid, route)
}
fn job_state(f: &Fixture, id: &str) -> String {
    f.store
        .connection()
        .unwrap()
        .query_row("SELECT state FROM provider_jobs WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .unwrap()
}
fn voice_route(f: &Fixture, cid: &str, id: &str) -> VoiceRoute {
    let route = VoiceRoute {
        id: id.into(),
        conversation_id: cid.into(),
        identity_id: f.identity.clone(),
        assistant_id: uuid::Uuid::new_v4().to_string(),
        phone_number_id: uuid::Uuid::new_v4().to_string(),
        destinations: vec!["+12025550101".into()],
        enabled: true,
    };
    f.store.configure_voice_route(&f.admin, &route).unwrap();
    route
}
fn call(f: &Fixture, cid: &str, route: &VoiceRoute, id: &str) -> String {
    let Outcome::Call(call) = f
        .store
        .execute(
            &f.admin,
            &Operation::CallStart {
                request_id: id.into(),
                conversation_id: cid.into(),
                route_id: route.id.clone(),
                destination: route.destinations[0].clone(),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    call.id
}
fn call_state(f: &Fixture, cid: &str, id: &str) -> String {
    let Outcome::Call(call) = f
        .store
        .execute(
            &f.admin,
            &Operation::CallStatus {
                conversation_id: cid.into(),
                call_id: id.into(),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    call.state
}

#[test]
fn provider_claim_completion_receipt_recovery_and_route_disable_wait_for_delivery() {
    for action in [
        "claim", "finish", "receipt", "recovery", "disable", "incoming",
    ] {
        let (f, cid, route) = provider();
        let job = if action == "claim" {
            None
        } else {
            Some(f.store.claim_provider_job().unwrap().unwrap())
        };
        let id = job.as_ref().map(|j| j.id.clone());
        let admin = f.admin.clone();
        wait_read_at(&f, &cid, action == "recovery", move |peer| match action {
            "claim" => peer.claim_provider_job().map(|j| {
                assert!(j.is_some());
            }),
            "finish" => peer.finish_provider_job(
                job.as_ref().unwrap(),
                "completed",
                Some("chat-once"),
                Some("one bounded answer"),
            ),
            "receipt" => peer
                .apply_connector_receipt(
                    "vapi",
                    &MessageReceipt {
                        observation: Observation {
                            mutation_id: job.as_ref().unwrap().id.clone(),
                            external_id: Some("chat-once".into()),
                            event_id: "accepted-once".into(),
                            state: State::Message(MessageState::Accepted),
                            verification: Verification::AuthenticatedResponse,
                        },
                        reply: None,
                        reply_sources: vec![],
                    },
                )
                .map(|_| ()),
            "recovery" => peer
                .recover_provider_jobs()
                .map(|count| assert_eq!(count, 1)),
            "disable" => {
                let mut route = route;
                route.enabled = false;
                peer.configure_route(&admin, &route)
            }
            _ => peer
                .receive_connector_message(
                    "vapi",
                    &route.id,
                    &IncomingMessage {
                        mutation_id: "incoming-once".into(),
                        external_id: "external-once".into(),
                        body: "an independently received message".into(),
                        verification: Verification::VerifiedCallback,
                    },
                )
                .map(|_| ()),
        });
        if let Some(id) = id {
            match action {
                "finish" => assert_eq!(job_state(&f, &id), "completed"),
                "receipt" => assert_eq!(job_state(&f, &id), "accepted"),
                "recovery" => {
                    assert_eq!(job_state(&f, &id), "cancelled");
                    assert!(f.store.claim_provider_job().unwrap().is_none());
                }
                _ => (),
            }
        }
    }
}

#[test]
fn copied_provider_job_cannot_switch_the_persisted_route_at_completion() {
    let (f, _, route) = provider();
    let mut job = f.store.claim_provider_job().unwrap().unwrap();
    let mut alternate = route;
    alternate.id = "other-route".into();
    alternate.target = uuid::Uuid::new_v4().to_string();
    alternate.conversation_id = conversation(
        &f,
        &f.admin,
        "other-conversation",
        vec![alternate.identity_id.clone()],
    );
    f.store.configure_route(&f.admin, &alternate).unwrap();
    job.route = alternate;
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store
            .finish_provider_job(&job, "completed", Some("wrong-route-chat"), None)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}

#[test]
fn voice_claim_dispatch_completion_and_recovery_wait_for_delivery() {
    for action in ["claim", "dispatch", "finish", "recover"] {
        let f = Fixture::new();
        let cid = conversation(&f, &f.admin, "conversation", vec![]);
        let route = voice_route(&f, &cid, "voice");
        let id = call(&f, &cid, &route, "call");
        let job = if action == "claim" {
            None
        } else {
            Some(f.store.claim_voice_job().unwrap().unwrap())
        };
        wait_read(&f, &cid, move |peer| match action {
            "claim" => peer
                .claim_voice_job()
                .map(|j| assert_eq!(j.unwrap().action, VoiceAction::Start)),
            "dispatch" => peer
                .begin_voice_dispatch(job.as_ref().unwrap())
                .map(|a| assert!(a.is_some())),
            "finish" => peer.finish_voice_job(
                job.as_ref().unwrap(),
                "ended",
                Some("owned-call"),
                None,
                None,
            ),
            _ => peer.recover_voice_calls(),
        });
        assert_eq!(
            call_state(&f, &cid, &id),
            match action {
                "finish" => "ended",
                "recover" => "uncertain",
                _ => "starting",
            }
        );
    }
}

#[test]
fn raw_voice_results_cannot_reopen_terminal_state_or_rebind_an_owned_remote_effect() {
    let f = Fixture::new();
    let cid = conversation(&f, &f.admin, "conversation", vec![]);
    let route = voice_route(&f, &cid, "voice");
    let id = call(&f, &cid, &route, "call");
    let job = f.store.claim_voice_job().unwrap().unwrap();
    f.store
        .finish_voice_job(&job, "in-progress", Some("owned-once"), None, None)
        .unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store
            .finish_voice_job(&job, "in-progress", Some("different-effect"), None, None)
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    f.store
        .finish_voice_job(&job, "ended", Some("owned-once"), None, None)
        .unwrap();
    let ended = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store
            .finish_voice_job(&job, "in-progress", Some("owned-once"), None, None)
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), ended);
    assert_eq!(call_state(&f, &cid, &id), "ended");
}

#[test]
fn a_different_time_eligible_voice_candidate_refuses_uncovered_transaction_then_retries() {
    let f = Fixture::new();
    let b = conversation(&f, &f.admin, "b", vec![]);
    let rb = voice_route(&f, &b, "voice-b");
    let bid = call(&f, &b, &rb, "call-b");
    let job = f.store.claim_voice_job().unwrap().unwrap();
    f.store
        .finish_voice_job(&job, "in-progress", Some("owned-b"), None, None)
        .unwrap();
    let a = conversation(&f, &f.admin, "a", vec![]);
    let ra = voice_route(&f, &a, "voice-a");
    let aid = call(&f, &a, &ra, "call-a");
    let due = crate::now() + 3;
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE voice_calls SET next_poll=?2 WHERE id=?1",
            rusqlite::params![bid, due],
        )
        .unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let peer = f.peer();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        crate::authority_admission::runtime::pause_next_admission(entered_tx, release_rx);
        peer.claim_voice_job()
    });
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    while crate::now() < due {
        std::thread::sleep(Duration::from_millis(5));
    }
    release_tx.send(()).unwrap();
    assert_eq!(
        worker.join().unwrap().err().unwrap().code,
        "authority-unavailable"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let next = f.store.claim_voice_job().unwrap().unwrap();
    assert_eq!(next.call.id, bid);
    assert_eq!(next.action, VoiceAction::Poll);
    assert_eq!(call_state(&f, &a, &aid), "pending");
}

fn enrollment(cid: &str, id: &str, sender: &str) -> SmsGrant {
    SmsGrant {
        id: id.into(),
        phone: "+15555550101".into(),
        sender: sender.into(),
        conversation_id: cid.into(),
        purpose: "bounded reviewed fixture".into(),
        consent_id: format!("consent-{id}"),
        disclosure_version: "v1".into(),
        disclosure_text: "fixture opt-in".into(),
        submitted_at: crate::now() - 2,
        approved_at: crate::now() - 1,
        expires_at: crate::now() + 300,
    }
}
#[test]
fn sms_refresh_retirement_stop_and_authority_expiry_wait_for_current_delivery() {
    for action in ["remove", "stop", "expire"] {
        let f = Fixture::new();
        let cid = conversation(&f, &f.admin, "conversation", vec![]);
        f.store
            .refresh_sms_grants(
                "fixture",
                &SmsGrantSnapshot {
                    revision: 1,
                    grants: vec![enrollment(&cid, "first", "+15555550102")],
                },
            )
            .unwrap();
        assert!(crate::sms_eligibility::eligible(
            &f.store.connection().unwrap(),
            "+15555550101",
            "+15555550102",
            &cid
        )
        .unwrap()
        .is_ok());
        wait_read(&f, &cid, move |peer| match action {
            "remove" => peer.refresh_sms_grants(
                "fixture",
                &SmsGrantSnapshot {
                    revision: 2,
                    grants: vec![],
                },
            ),
            "stop" => peer
                .receive_sms_control(
                    "stop-once",
                    "digest",
                    "+15555550101",
                    "+15555550102",
                    "STOP",
                    crate::now(),
                )
                .map(|v| assert!(v)),
            _ => peer.expire_sms_authorities(),
        });
        assert!(crate::sms_eligibility::eligible(
            &f.store.connection().unwrap(),
            "+15555550101",
            "+15555550102",
            &cid
        )
        .unwrap()
        .is_err());
    }
}

#[test]
fn global_phone_stop_covers_other_sender_and_conversation_authority() {
    let f = Fixture::new();
    let a = conversation(&f, &f.admin, "a", vec![]);
    let other = f.store.create_identity("other").unwrap().id;
    let key = f
        .store
        .issue(&other, Scope::operator(), crate::now() + 300)
        .unwrap();
    let b = conversation(&f, &key.id, "b", vec![]);
    f.store
        .refresh_sms_grants(
            "fixture",
            &SmsGrantSnapshot {
                revision: 1,
                grants: vec![
                    enrollment(&a, "a", "+15555550102"),
                    enrollment(&b, "b", "+15555550103"),
                ],
            },
        )
        .unwrap();
    let read = f
        .store
        .read_authority(
            &key.id,
            Some(&b),
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    let peer = f.peer();
    let worker = std::thread::spawn(move || {
        peer.receive_sms_control(
            "global-stop",
            "digest",
            "+15555550101",
            "+15555550102",
            "STOP",
            crate::now(),
        )
    });
    waiting(
        &f.store,
        &AuthorityScope::identity(&other, AuthorityMode::Write).unwrap(),
    );
    drop(read);
    assert!(worker.join().unwrap().unwrap());
    assert!(crate::sms_eligibility::eligible(
        &f.store.connection().unwrap(),
        "+15555550101",
        "+15555550103",
        &b
    )
    .unwrap()
    .is_err());
}

#[test]
fn voice_route_admin_cannot_cross_tenant_even_with_wildcard_recipients() {
    let f = Fixture::new();
    let other = f.store.create_identity("other").unwrap().id;
    let key = f
        .store
        .issue(&other, Scope::operator(), crate::now() + 300)
        .unwrap();
    {
        let c = f.store.connection().unwrap();
        c.execute_batch("INSERT INTO tenants(id) VALUES('tnt_other')")
            .unwrap();
        c.execute(
            "UPDATE identities SET tenant_id='tnt_other' WHERE id=?1",
            [&other],
        )
        .unwrap();
    }
    let cid = conversation(&f, &key.id, "foreign", vec![]);
    let route = VoiceRoute {
        id: "foreign-route".into(),
        conversation_id: cid,
        identity_id: other,
        assistant_id: uuid::Uuid::new_v4().to_string(),
        phone_number_id: uuid::Uuid::new_v4().to_string(),
        destinations: vec!["+12025550101".into()],
        enabled: true,
    };
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store
            .configure_voice_route(&f.admin, &route)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    f.store.configure_voice_route(&key.id, &route).unwrap();
}

#[test]
fn sip_claim_preparation_dispatch_result_and_recovery_wait_for_delivery() {
    for action in [
        "claim",
        "prepare",
        "dispatch",
        "finish",
        "recover",
        "end",
        "cancel-observation",
    ] {
        let f = Fixture::new();
        let other = f.store.create_identity("other").unwrap().id;
        let key = f
            .store
            .issue(&other, Scope::operator(), crate::now() + 600)
            .unwrap();
        let cid = conversation(&f, &f.admin, "conversation", vec![other.clone()]);
        let Outcome::Session(session) = f
            .store
            .execute(
                &f.admin,
                &Operation::SessionInvite {
                    request_id: "invite".into(),
                    conversation_id: cid.clone(),
                    participant_id: other,
                    media: vec!["audio".into()],
                },
            )
            .unwrap()
        else {
            panic!()
        };
        f.store
            .execute(
                &key.id,
                &Operation::SessionAccept {
                    request_id: "accept".into(),
                    conversation_id: cid.clone(),
                    session_id: session.id.clone(),
                },
            )
            .unwrap();
        let Outcome::SipLeg(leg) = f
            .store
            .execute(
                &f.admin,
                &Operation::SipDial {
                    request_id: "dial".into(),
                    conversation_id: cid.clone(),
                    session_id: session.id,
                    destination: "sip:fixture@example.invalid".into(),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        let job = if action == "claim" {
            None
        } else {
            Some(f.store.claim_sip_leg_job().unwrap().unwrap())
        };
        if matches!(action, "end" | "cancel-observation") {
            f.store
                .prepare_sip_leg(job.as_ref().unwrap(), "physical-sip")
                .unwrap();
        }
        wait_read(&f, &cid, move |peer| match action {
            "claim" => peer
                .claim_sip_leg_job()
                .map(|j| assert_eq!(j.unwrap().action, crate::SipLegAction::Start)),
            "prepare" => peer.prepare_sip_leg(job.as_ref().unwrap(), "physical-sip"),
            "dispatch" => peer
                .sip_dispatch_allowed(job.as_ref().unwrap())
                .map(|admitted| assert!(admitted)),
            "finish" => peer.finish_sip_leg_job(job.as_ref().unwrap(), "active", None),
            "recover" => peer.recover_sip_legs().map(|count| assert_eq!(count, 1)),
            "end" => peer.observe_sip_connection_end(
                "physical-sip",
                crate::SipConnectionEnd::Ended,
                "observed-end",
            ),
            _ => peer.observe_sip_connection_end(
                "physical-sip",
                crate::SipConnectionEnd::Cancelled,
                "cleanup-pending",
            ),
        });
        let Outcome::SipLeg(saved) = f
            .store
            .execute(
                &f.admin,
                &Operation::SipStatus {
                    conversation_id: cid.clone(),
                    sip_leg_id: leg.id,
                },
            )
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(
            saved.state,
            match action {
                "finish" => "active",
                "recover" => "uncertain",
                "end" => "ended",
                _ => "starting",
            }
        );
    }
}

#[test]
fn voice_route_disabling_and_ingress_configuration_wait_for_delivery() {
    for ingress in [false, true] {
        let f = Fixture::new();
        let cid = conversation(&f, &f.admin, "conversation", vec![]);
        let mut route = voice_route(&f, &cid, "voice");
        let admin = f.admin.clone();
        wait_read(&f, &cid, move |peer| {
            if ingress {
                peer.configure_voice_ingress(&admin, &route.id, &route.phone_number_id)
            } else {
                route.enabled = false;
                peer.configure_voice_route(&admin, &route)
            }
        });
    }
}
