//! Canonical transition admission/refusal/cancellation parity only. These
//! trusted adapter rows never qualify live handoff/replacement media readiness.
use super::*;
use connect_client::{application::ApplicationClientConfig, canonical::CommandScope};
use connect_protocol::{transitions::*, Operation, Outcome};
use rusqlite::{params, Connection};

pub(super) fn seed(f: &Fixture, other: &CanonicalClientConfig) -> Vec<Value> {
    let third = f.server.store.authenticate(&f.admin).unwrap().identity_id;
    let ids = vec![
        f.client.identity_id.clone(),
        other.identity_id.clone(),
        third,
    ];
    let actor = f.server.store.authenticate(&f.client.credential).unwrap();
    let review = f
        .server
        .store
        .issue(
            &ids[1],
            Scope::communications(ids.clone()),
            chrono::Utc::now().timestamp() + 3600,
        )
        .unwrap();
    (0..3).map(|kind| {
        let prefix=format!("browser-transition-{kind}");
        let Outcome::Conversation(conv)=f.server.store.execute(&actor.credential_id,&Operation::Create{request_id:format!("{prefix}-room"),request:connect_protocol::conversations::CreateRequest::persistent(ids[1..].to_vec())}).unwrap()else{panic!("transition room")};
        let parts=ids.iter().map(|id|conv.participants.iter().find(|p|&p.identity_id==id).unwrap().participant_id.clone()).collect::<Vec<_>>();
        let Outcome::Session(session)=f.server.store.execute(&actor.credential_id,&Operation::SessionInvite{request_id:format!("{prefix}-session"),conversation_id:conv.id.clone(),participant_id:ids[1].clone(),media:vec!["audio".into()]}).unwrap()else{panic!("transition Session")};
        f.server.store.execute(&review.id,&Operation::SessionAccept{request_id:format!("{prefix}-accept"),conversation_id:conv.id.clone(),session_id:session.id.clone()}).unwrap();
        let connection=|n|format!("{prefix}-connection-{n}");
        let endpoint=|n|format!("{prefix}-endpoint-{n}");
        let c=Connection::open(f.server.store.path()).unwrap();
        for n in 0..4 {
            let owner=if n==3 {0}else{n};let device=format!("{prefix}-device-{n}");
            c.execute("INSERT INTO devices(id,identity_id,tenant_id,kind,platform) VALUES(?1,?2,'tnt_default','fixture','fixture')",params![device,ids[owner]]).unwrap();
            c.execute("INSERT INTO endpoints(id,identity_id,tenant_id,device_id,transport,address,capabilities_json,verification_state,verification_evidence,verified_at) VALUES(?1,?2,'tnt_default',?3,'webrtc',?4,'[\"audio\"]','verified','isolated-transition-adapter',?5)",params![endpoint(n),ids[owner],device,format!("https://fixture.invalid/{prefix}/{n}"),chrono::Utc::now().timestamp()]).unwrap();
            if n==3{continue}
            c.execute("INSERT INTO session_participants(session_id,conversation_id,tenant_id,participant_id,membership_epoch,role,state) VALUES(?1,?2,'tnt_default',?3,1,'participant','present') ON CONFLICT(session_id,participant_id,membership_epoch) DO UPDATE SET role=excluded.role,state=excluded.state",params![session.id,conv.id,parts[n]]).unwrap();
            c.execute("INSERT INTO connections(id,session_id,participant_id,membership_epoch,identity_id,tenant_id,endpoint_id,generation,state,observed_runtime_id,observed_at,readiness_json) VALUES(?1,?2,?3,1,?4,'tnt_default',?5,1,?6,?7,?8,'{\"ingress\":true,\"egress\":true}')",params![connection(n),session.id,parts[n],ids[n],endpoint(n),if n==2{"ready"}else{"active"},f.server.store.runtime_epoch(),chrono::Utc::now().timestamp()]).unwrap();
        }
        let intent=match kind {
            0=>TransitionIntent::Consult{retained_connid:connection(1),private_participants:vec![parts[2].clone(),parts[0].clone()],hold_treatment:HoldTreatment::Silence},
            1=>TransitionIntent::Handoff{source_participant:parts[0].clone(),target_participant:parts[2].clone(),retained_connid:connection(1),candidate_connid:Some(connection(2)),outgoing:OutgoingDisposition::Leave,role_scope:RoleScope::Session,expected_conversation_revision:None},
            _=>TransitionIntent::Replace{retained_connid:connection(1),endpoint_id:Some(endpoint(3)),candidate_connid:None},
        };
        let mut payload=serde_json::to_value(intent).unwrap();let command=payload["type"].as_str().unwrap().to_owned();payload.as_object_mut().unwrap().retain(|k,v|k!="type"&&!v.is_null());
        json!({"type":command,"payload":payload,"scope":{"cid":conv.id,"sid":session.id,"connid":connection(0),"operation_id":format!("{prefix}-original"),"expected_revision":0},"terminal":if kind==0{"cancelled"}else{"failed"},"problem":match kind{1=>json!({"code":"invalid-request","source":"native-handoff"}),2=>json!({"code":"forbidden","source":"native-replacement"}),_=>Value::Null}})
    }).collect()
}

pub(super) async fn audit(f: &Fixture, probes: &[Value], report: &Value) -> Value {
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut http = CanonicalHttpClient::open(ApplicationClientConfig {
        endpoint: f.server.http_address.unwrap(),
        credential: f.client.credential.clone(),
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(10),
        timeout_ms: 5000,
    })
    .await
    .unwrap();
    let c = Connection::open(f.server.store.path()).unwrap();
    let mut rows = Vec::new();
    assert_eq!(
        report["transition_scope"],
        "signed control admission/refusal/cancellation/replay; no live movement media claim"
    );
    assert_eq!(report["transitions"].as_array().unwrap().len(), 3);
    for (i, probe) in probes.iter().enumerate() {
        let s = &probe["scope"];
        let op = s["operation_id"].as_str().unwrap();
        let scope = CommandScope {
            conversation_id: Some(s["cid"].as_str().unwrap().into()),
            session_id: Some(s["sid"].as_str().unwrap().into()),
            connection_id: Some(s["connid"].as_str().unwrap().into()),
            ..Default::default()
        };
        let n =
            serde_json::to_value(native.operation_get(op, scope.clone()).await.unwrap()).unwrap();
        let h = serde_json::to_value(http.operation_get(op, scope).await.unwrap()).unwrap();
        assert_eq!(n, h);
        assert_eq!(n["state"], probe["terminal"]);
        let effects:u64=c.query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id=?1",[op],|r|r.get(0)).unwrap();
        assert_eq!(effects, 0);
        if !probe["problem"].is_null() {
            assert_eq!(n["problem"], probe["problem"])
        }
        let b = &report["transitions"][i];
        assert_eq!(b["operation_id"], op);
        assert_eq!(b["terminal_state"], n["state"]);
        assert_eq!(b["original_record_id"], n["record_id"]);
        for field in [
            "operation_id",
            "record_id",
            "state",
            "revision",
            "result",
            "problem",
        ] {
            assert_eq!(
                b["terminal_status"][field], n[field],
                "common status field {field}"
            );
        }
        let expected: Option<u64> = c
            .query_row(
                "SELECT expected_revision FROM operations WHERE operation_id=?1",
                [op],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(b["original_expected_revision"], json!(expected));
        assert_eq!(expected, Some(0));
        let expiry: Option<i64> = c
            .query_row(
                "SELECT expires_at FROM operations WHERE operation_id=?1",
                [op],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(b["original_expires_at"], json!(expiry));
        assert_eq!(b["expiry_unchanged"], true);
        assert_eq!(b["oracle_effects"], effects);
        assert_eq!(b["replay_state"], n["state"]);
        assert_eq!(b["changed_revision_refused"], true);
        assert_eq!(b["foreign_scope_refused"], true);
        let count: u64 = c
            .query_row(
                "SELECT COUNT(*) FROM operations WHERE operation_id=?1",
                [op],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        let revision: u64 = c
            .query_row(
                "SELECT topology_revision FROM sessions WHERE id=?1",
                [s["sid"].as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(revision, 0);
        let active: String = c
            .query_row(
                "SELECT state FROM connections WHERE id=?1",
                [s["connid"].as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(active, "active");
        rows.push(json!({"type":probe["type"],"operation_id":op,"state":n["state"],"original_expires_at":expiry,"native_http_browser_common_status_match":true,"operation_rows":count,"external_effects":0,"topology_revision":revision,"source_connection_state":active}));
    }
    native.shutdown().await.unwrap();
    json!({"passed":true,"scope":"Canonical transition command parity on actual browser fallback control. Trusted adapter rows; no live handoff/replacement or decoded movement quality claim.","rows":rows})
}
