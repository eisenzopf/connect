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
                participants: vec![agent.id.clone()],
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
