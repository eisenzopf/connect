#![allow(clippy::unwrap_used)] // Integration assertions deliberately panic.

use connect_protocol::{Limits, Operation, Outcome};
use connect_store::{Scope, SipLegAction, Store};

#[test]
fn sip_intent_is_authorized_idempotent_recoverable_and_terminable() {
    let directory = tempfile::tempdir().unwrap();
    let store =
        Store::initialize(&directory.path().join("state.sqlite3"), Limits::default()).unwrap();
    let app = store.create_identity("app").unwrap();
    let agent = store.create_identity("agent").unwrap();
    let outsider = store.create_identity("outsider").unwrap();
    let operator_identity = store.create_identity("operator").unwrap();
    let expiry = chrono::Utc::now().timestamp() + 3_600;
    let app_key = store
        .issue(
            &app.id,
            Scope::communications(vec![agent.id.clone()]),
            expiry,
        )
        .unwrap();
    let agent_key = store
        .issue(
            &agent.id,
            Scope::communications(vec![app.id.clone()]),
            expiry,
        )
        .unwrap();
    let outsider_key = store
        .issue(&outsider.id, Scope::communications(vec![]), expiry)
        .unwrap();
    let operator_key = store
        .issue(&operator_identity.id, Scope::operator(), expiry)
        .unwrap();
    let operator = store.authenticate(&operator_key.token).unwrap();
    let conversation = match store
        .execute(
            &app_key.id,
            &Operation::Create {
                request_id: "sip-create".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![agent
                    .id
                    .clone()]),
            },
        )
        .unwrap()
    {
        Outcome::Conversation(value) => value,
        other => panic!("expected conversation, got {other:?}"),
    };
    let session = match store
        .execute(
            &app_key.id,
            &Operation::SessionInvite {
                request_id: "sip-invite".into(),
                conversation_id: conversation.id.clone(),
                participant_id: agent.id,
                media: vec!["audio".into()],
            },
        )
        .unwrap()
    {
        Outcome::Session(value) => value,
        other => panic!("expected session, got {other:?}"),
    };
    store
        .execute(
            &agent_key.id,
            &Operation::SessionAccept {
                request_id: "sip-accept".into(),
                conversation_id: conversation.id.clone(),
                session_id: session.id.clone(),
            },
        )
        .unwrap();

    let invalid = Operation::SipDial {
        request_id: "invalid-sip-dial".into(),
        conversation_id: conversation.id.clone(),
        session_id: session.id.clone(),
        destination: "https://not-sip.example.test".into(),
    };
    assert_eq!(
        store.execute(&app_key.id, &invalid).unwrap_err().code,
        "invalid-request"
    );
    assert_eq!(
        store
            .execute(
                &agent_key.id,
                &Operation::SipDial {
                    request_id: "participant-cannot-dial".into(),
                    conversation_id: conversation.id.clone(),
                    session_id: session.id.clone(),
                    destination: "sip:peer@127.0.0.1:5099".into(),
                },
            )
            .unwrap_err()
            .code,
        "forbidden"
    );

    let operation = Operation::SipDial {
        request_id: "sip-dial".into(),
        conversation_id: conversation.id.clone(),
        session_id: session.id.clone(),
        destination: "sip:peer@127.0.0.1:5099".into(),
    };
    let leg = match store.execute(&app_key.id, &operation).unwrap() {
        Outcome::SipLeg(value) => value,
        other => panic!("expected SIP leg, got {other:?}"),
    };
    assert_eq!(leg.state, "pending");
    assert!(
        matches!(store.execute(&app_key.id, &operation).unwrap(), Outcome::SipLeg(value) if value.id == leg.id)
    );
    assert_eq!(
        store
            .execute(
                &outsider_key.id,
                &Operation::SipStatus {
                    conversation_id: conversation.id.clone(),
                    sip_leg_id: leg.id.clone(),
                },
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    let job = store.claim_sip_leg_job().unwrap().unwrap();
    assert_eq!(job.action, SipLegAction::Start);
    store
        .prepare_sip_leg(&job, "connection-local-test")
        .unwrap();
    store.finish_sip_leg_job(&job, "active", None).unwrap();
    let active = match store
        .execute(
            &app_key.id,
            &Operation::SipStatus {
                conversation_id: conversation.id.clone(),
                sip_leg_id: leg.id.clone(),
            },
        )
        .unwrap()
    {
        Outcome::SipLeg(value) => value,
        other => panic!("expected SIP leg, got {other:?}"),
    };
    assert_eq!(
        active.connection_id.as_deref(),
        Some("connection-local-test")
    );
    store
        .execute(
            &agent_key.id,
            &Operation::SipEnd {
                request_id: "sip-end".into(),
                conversation_id: conversation.id.clone(),
                sip_leg_id: leg.id,
            },
        )
        .unwrap();
    let end_job = store.claim_sip_leg_job().unwrap().unwrap();
    assert_eq!(end_job.action, SipLegAction::End);
    store
        .finish_sip_leg_job(&end_job, "ended", Some("local-hangup"))
        .unwrap();

    let cancelled = match store
        .execute(
            &app_key.id,
            &Operation::SipDial {
                request_id: "cancel-before-signaling".into(),
                conversation_id: conversation.id.clone(),
                session_id: session.id.clone(),
                destination: "sip:peer@127.0.0.1:5099".into(),
            },
        )
        .unwrap()
    {
        Outcome::SipLeg(value) => value,
        other => panic!("expected SIP leg, got {other:?}"),
    };
    let cancelled = match store
        .execute(
            &app_key.id,
            &Operation::SipEnd {
                request_id: "cancel-before-signaling-end".into(),
                conversation_id: conversation.id.clone(),
                sip_leg_id: cancelled.id,
            },
        )
        .unwrap()
    {
        Outcome::SipLeg(value) => value,
        other => panic!("expected SIP leg, got {other:?}"),
    };
    assert_eq!(cancelled.state, "cancelled");
    assert!(store.claim_sip_leg_job().unwrap().is_none());

    let active_before_restart = match store
        .execute(
            &app_key.id,
            &Operation::SipDial {
                request_id: "active-before-restart".into(),
                conversation_id: conversation.id.clone(),
                session_id: session.id.clone(),
                destination: "sip:peer@127.0.0.1:5099".into(),
            },
        )
        .unwrap()
    {
        Outcome::SipLeg(value) => value,
        other => panic!("expected SIP leg, got {other:?}"),
    };
    let active_job = store.claim_sip_leg_job().unwrap().unwrap();
    store
        .prepare_sip_leg(&active_job, "connection-active-before-restart")
        .unwrap();
    store
        .finish_sip_leg_job(&active_job, "active", None)
        .unwrap();

    let starting_before_restart = match store
        .execute(
            &app_key.id,
            &Operation::SipDial {
                request_id: "starting-before-restart".into(),
                conversation_id: conversation.id.clone(),
                session_id: session.id.clone(),
                destination: "sip:peer@127.0.0.1:5099".into(),
            },
        )
        .unwrap()
    {
        Outcome::SipLeg(value) => value,
        other => panic!("expected SIP leg, got {other:?}"),
    };
    let starting_job = store.claim_sip_leg_job().unwrap().unwrap();
    assert_eq!(starting_job.leg.id, starting_before_restart.id);
    assert_eq!(store.recover_sip_legs().unwrap(), 2);
    for (id, state) in [
        (active_before_restart.id, "failed"),
        (starting_before_restart.id, "uncertain"),
    ] {
        let recovered = match store
            .execute(
                &app_key.id,
                &Operation::SipStatus {
                    conversation_id: conversation.id.clone(),
                    sip_leg_id: id,
                },
            )
            .unwrap()
        {
            Outcome::SipLeg(value) => value,
            other => panic!("expected SIP leg, got {other:?}"),
        };
        assert_eq!(recovered.state, state);
        assert_eq!(recovered.ended_reason.as_deref(), Some("server-restarted"));
    }

    let delegated = store
        .issue(
            &app.id,
            Scope::communications(vec![session.participant_id.clone()]),
            expiry,
        )
        .unwrap();
    let revoked = match store
        .execute(
            &delegated.id,
            &Operation::SipDial {
                request_id: "revoked-dial".into(),
                conversation_id: conversation.id.clone(),
                session_id: session.id,
                destination: "sip:peer@127.0.0.1:5099".into(),
            },
        )
        .unwrap()
    {
        Outcome::SipLeg(value) => value,
        other => panic!("expected SIP leg, got {other:?}"),
    };
    store.revoke(&operator, &delegated.id).unwrap();
    assert!(store.claim_sip_leg_job().unwrap().is_none());
    let connection = rusqlite::Connection::open(store.path()).unwrap();
    assert_eq!(
        connection
            .query_row::<String, _, _>(
                "SELECT state FROM sip_legs WHERE id=?1",
                [&revoked.id],
                |row| row.get(0),
            )
            .unwrap(),
        "failed"
    );
}

struct MediaFixture {
    _directory: tempfile::TempDir,
    store: Store,
    app: String,
    agent: String,
    app_key: String,
    agent_key: String,
    conversation: String,
    av_session: String,
    audio_session: String,
}
fn media_fixture() -> MediaFixture {
    let directory = tempfile::tempdir().unwrap();
    let store =
        Store::initialize(&directory.path().join("state.sqlite3"), Limits::default()).unwrap();
    let app = store.create_identity("audio-app").unwrap().id;
    let agent = store.create_identity("audio-agent").unwrap().id;
    let expiry = chrono::Utc::now().timestamp() + 600;
    let app_key = store
        .issue(&app, Scope::communications(vec![agent.clone()]), expiry)
        .unwrap()
        .id;
    let agent_key = store
        .issue(&agent, Scope::communications(vec![app.clone()]), expiry)
        .unwrap()
        .id;
    let Outcome::Conversation(conversation) = store
        .execute(
            &app_key,
            &Operation::Create {
                request_id: "profile-conversation".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![
                    agent.clone()
                ]),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let mut sessions = Vec::new();
    for (name, media) in [
        ("av", vec!["audio".into(), "video".into()]),
        ("audio", vec!["audio".into()]),
    ] {
        let Outcome::Session(session) = store
            .execute(
                &app_key,
                &Operation::SessionInvite {
                    request_id: format!("invite-{name}"),
                    conversation_id: conversation.id.clone(),
                    participant_id: agent.clone(),
                    media,
                },
            )
            .unwrap()
        else {
            panic!()
        };
        store
            .execute(
                &agent_key,
                &Operation::SessionAccept {
                    request_id: format!("accept-{name}"),
                    conversation_id: conversation.id.clone(),
                    session_id: session.id.clone(),
                },
            )
            .unwrap();
        sessions.push(session.id);
    }
    MediaFixture {
        _directory: directory,
        store,
        app,
        agent,
        app_key,
        agent_key,
        conversation: conversation.id,
        av_session: sessions[0].clone(),
        audio_session: sessions[1].clone(),
    }
}
fn profile_dial(f: &MediaFixture, session: &str, request: &str) -> Operation {
    Operation::SipDial {
        request_id: request.into(),
        conversation_id: f.conversation.clone(),
        session_id: session.into(),
        destination: "sip:peer@127.0.0.1:5099".into(),
    }
}
fn count(f: &MediaFixture, query: &str) -> u64 {
    rusqlite::Connection::open(f.store.path())
        .unwrap()
        .query_row(query, [], |row| row.get(0))
        .unwrap()
}
fn leg(f: &MediaFixture, id: &str) -> connect_protocol::SipLeg {
    let Outcome::SipLeg(leg) = f
        .store
        .execute(
            &f.app_key,
            &Operation::SipStatus {
                conversation_id: f.conversation.clone(),
                sip_leg_id: id.into(),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    leg
}
fn insert_legacy_av_leg(f: &MediaFixture, id: &str, state: &str) {
    // Model an existing unsupported row admitted by the old audio-containing
    // check. This private fixture never signals or uses a real SIP endpoint.
    rusqlite::Connection::open(f.store.path()).unwrap().execute(
        "INSERT INTO sip_legs(id,conversation_id,session_id,credential_id,actor_id,destination,state,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,'sip:peer@127.0.0.1:5099',?6,?7,?7)",
        rusqlite::params![id, f.conversation, f.av_session, f.app_key, f.app, state, chrono::Utc::now().timestamp()],
    ).unwrap();
}

#[test]
fn av_sip_dial_is_canonically_rejected_without_intent_and_same_conversation_audio_dispatches() {
    let f = media_fixture();
    let events = count(&f, "SELECT COUNT(*) FROM journal");
    let operation = profile_dial(&f, &f.av_session, "unsupported-av-dial");
    for _ in 0..2 {
        assert_eq!(
            f.store.execute(&f.app_key, &operation).unwrap_err().code,
            "unsupported-media-profile"
        );
    }
    assert_eq!(count(&f, "SELECT COUNT(*) FROM sip_legs"), 0);
    assert_eq!(
        count(
            &f,
            "SELECT COUNT(*) FROM operations WHERE operation_id='unsupported-av-dial'"
        ),
        1
    );
    let saved: String = rusqlite::Connection::open(f.store.path())
        .unwrap()
        .query_row(
            "SELECT problem_json FROM operations WHERE operation_id='unsupported-av-dial'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let rejection: connect_protocol::Error = serde_json::from_str(&saved).unwrap();
    assert_eq!(rejection.code, "unsupported-media-profile");
    assert_eq!(count(&f, "SELECT COUNT(*) FROM journal"), events);
    assert!(f.store.claim_sip_leg_job().unwrap().is_none());
    let Outcome::SipLeg(audio) = f
        .store
        .execute(
            &f.app_key,
            &profile_dial(&f, &f.audio_session, "audio-dial"),
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(audio.conversation_id, f.conversation);
    let job = f.store.claim_sip_leg_job().unwrap().unwrap();
    assert_eq!(job.leg.id, audio.id);
    assert!(!f.store.sip_origination_cancelled(&job).unwrap());
    assert!(f.store.sip_dispatch_allowed(&job).unwrap());
    assert!(
        !f.store.sip_dispatch_allowed(&job).unwrap(),
        "one possible physical admission"
    );
    assert_eq!(count(&f, "SELECT COUNT(*) FROM media_start_admissions"), 1);
}

#[test]
fn old_unsupported_pending_sip_leg_is_refused_without_worker_error_or_dispatch() {
    let f = media_fixture();
    insert_legacy_av_leg(&f, "legacy-pending", "pending");
    assert!(f.store.claim_sip_leg_job().unwrap().is_none());
    let refused = leg(&f, "legacy-pending");
    assert_eq!(refused.state, "failed");
    assert_eq!(
        refused.ended_reason.as_deref(),
        Some("unsupported-media-profile")
    );
    assert!(refused.connection_id.is_none());
    assert!(f.store.claim_sip_leg_job().unwrap().is_none());
    assert_eq!(count(&f, "SELECT COUNT(*) FROM media_start_admissions"), 0);
    let Outcome::SipLeg(audio) = f
        .store
        .execute(
            &f.app_key,
            &profile_dial(&f, &f.audio_session, "audio-after-refusal"),
        )
        .unwrap()
    else {
        panic!()
    };
    let next = f.store.claim_sip_leg_job().unwrap().unwrap();
    assert_eq!(next.leg.id, audio.id);
    assert!(f.store.sip_dispatch_allowed(&next).unwrap());
}

#[test]
fn old_claimed_av_sip_start_is_cancelled_but_already_owned_leg_can_be_ended() {
    let f = media_fixture();
    insert_legacy_av_leg(&f, "legacy-starting", "starting");
    let job = connect_store::SipLegJob {
        leg: leg(&f, "legacy-starting"),
        credential_id: f.app_key.clone(),
        actor_id: f.app.clone(),
        action: SipLegAction::Start,
        end_intent_id: None,
    };
    assert!(f.store.sip_origination_cancelled(&job).unwrap());
    assert!(!f.store.sip_dispatch_allowed(&job).unwrap());
    assert!(!f.store.sip_dispatch_allowed(&job).unwrap());
    let refused = leg(&f, "legacy-starting");
    assert_eq!(refused.state, "cancelled");
    assert_eq!(
        refused.ended_reason.as_deref(),
        Some("unsupported-media-profile")
    );
    assert_eq!(count(&f, "SELECT COUNT(*) FROM media_start_admissions"), 0);
    insert_legacy_av_leg(&f, "legacy-active", "active");
    // Existing ownership still grants cleanup; audio-only admission is not a
    // reason to strand a physical leg already owned by this participant.
    let Outcome::SipLeg(ending) = f
        .store
        .execute(
            &f.agent_key,
            &Operation::SipEnd {
                request_id: "end-owned-av".into(),
                conversation_id: f.conversation.clone(),
                sip_leg_id: "legacy-active".into(),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert!(ending.end_requested);
    let cleanup = f.store.claim_sip_leg_job().unwrap().unwrap();
    assert_eq!(cleanup.action, SipLegAction::End);
    assert_eq!(cleanup.actor_id, f.agent);
    assert!(f.store.sip_dispatch_allowed(&cleanup).unwrap());
    f.store
        .finish_sip_leg_job(&cleanup, "ended", Some("owned-cleanup"))
        .unwrap();
    assert_eq!(leg(&f, "legacy-active").state, "ended");
}
