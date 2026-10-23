#![allow(clippy::unwrap_used)]
use crate::{Scope, Store};
use connect_protocol::{membership::*, provisioning::IdentityKind, Limits, Operation, Outcome};
use rusqlite::params;

struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
    identities: Vec<String>,
    keys: Vec<String>,
    cid: String,
    sid: String,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store =
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap();
        let identities: Vec<_> = ["human", "assistant", "telephone", "observer"]
            .into_iter()
            .map(|name| store.create_identity(name).unwrap().id)
            .collect();
        let keys: Vec<_> = identities
            .iter()
            .map(|id| {
                store
                    .issue(id, Scope::operator(), crate::now() + 600)
                    .unwrap()
                    .id
            })
            .collect();
        let Outcome::Conversation(conv) = store
            .execute(
                &keys[0],
                &Operation::Create {
                    request_id: "room".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(
                        identities[1..].to_vec(),
                    ),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        let invitation = Operation::SessionInvite {
            request_id: "invite".into(),
            conversation_id: conv.id.clone(),
            participant_id: identities[1].clone(),
            media: vec!["audio".into()],
        };
        let Outcome::Session(session) = store.execute(&keys[0], &invitation).unwrap() else {
            panic!()
        };
        assert_eq!(
            store.execute(&keys[0], &invitation).unwrap(),
            Outcome::Session(session.clone())
        );
        Self {
            _dir: dir,
            store,
            identities,
            keys,
            cid: conv.id,
            sid: session.id,
        }
    }
    fn accept(&self) -> Operation {
        Operation::SessionAccept {
            request_id: "accept".into(),
            conversation_id: self.cid.clone(),
            session_id: self.sid.clone(),
        }
    }
    fn presence(&self) -> Vec<connect_protocol::events::SessionAppearance> {
        self.store
            .session_presence(&self.keys[0], &self.cid, &self.sid)
            .unwrap()
    }
    fn member(&self, index: usize) -> MemberView {
        crate::audience::member(
            &self.store.connection().unwrap(),
            &self.identities[index],
            &self.cid,
            true,
        )
        .unwrap()
    }
}

#[test]
fn invitation_presence_is_atomic_epoch_bound_and_terminal_presence_cannot_attach() {
    let f = Fixture::new();
    assert_eq!(f.presence().len(), 2);
    assert_eq!(
        f.presence().iter().filter(|p| p.state == "invited").count(),
        1
    );
    assert!(f
        .store
        .authorize_session_binding(&f.keys[0], &f.sid)
        .is_err());
    f.store.execute(&f.keys[1], &f.accept()).unwrap();
    assert!(f
        .presence()
        .iter()
        .all(|p| p.state == "present" && p.membership_epoch == 1));
    f.store.execute(&f.keys[1], &f.accept()).unwrap();
    assert_eq!(f.presence().len(), 2);
    f.store
        .execute(
            &f.keys[1],
            &Operation::SessionEnd {
                request_id: "end".into(),
                conversation_id: f.cid.clone(),
                session_id: f.sid.clone(),
                reason: None,
            },
        )
        .unwrap();
    assert!(f.presence().iter().all(|p| p.state == "left"));
    assert!(f
        .store
        .authorize_session_binding(&f.keys[0], &f.sid)
        .is_err());
    assert_eq!(
        f.store
            .authorize_session_lifetime(&f.keys[0], &f.sid)
            .unwrap()
            .state,
        "ended"
    );
}

#[test]
fn missing_invitation_presence_rolls_back_accept_and_retains_rejection() {
    let f = Fixture::new();
    let participant = f.member(1);
    let c = f.store.connection().unwrap();
    c.execute(
        "UPDATE session_participants SET state='removed' WHERE session_id=?1 AND participant_id=?2",
        params![f.sid, participant.participant_id],
    )
    .unwrap();
    drop(c);
    assert_eq!(
        f.store.execute(&f.keys[1], &f.accept()).unwrap_err().code,
        "forbidden"
    );
    let c = f.store.connection().unwrap();
    let state: String = c
        .query_row("SELECT state FROM sessions WHERE id=?1", [&f.sid], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(state, "invited");
    // Isolated corruption repair; this does not claim a public re-admission API.
    c.execute(
        "UPDATE session_participants SET state='invited' WHERE session_id=?1 AND participant_id=?2",
        params![f.sid, participant.participant_id],
    )
    .unwrap();
    drop(c);
    // A business rejection is retained under its original ID; changed state does
    // not rewrite that rejected result. A new acceptance is an explicit decision.
    assert_eq!(
        f.store.execute(&f.keys[1], &f.accept()).unwrap_err().code,
        "forbidden"
    );
    let mut accept = f.accept();
    if let Operation::SessionAccept { request_id, .. } = &mut accept {
        *request_id = "repaired-accept".into();
    }
    f.store.execute(&f.keys[1], &accept).unwrap();
    assert!(f.presence().iter().all(|p| p.state == "present"));
}

#[test]
fn four_party_presence_authorizes_current_members_and_removal_is_isolated() {
    let f = Fixture::new();
    f.store.execute(&f.keys[1], &f.accept()).unwrap();
    assert_eq!(
        f.store
            .authorize_session_binding(&f.keys[2], &f.sid)
            .unwrap_err()
            .code,
        "forbidden"
    );
    // Trusted general-session rows qualify the Store boundary only. Public
    // multiparty invitation/join and physical six-slot media remain G6-038 work.
    for index in 2..4 {
        let member = f.member(index);
        f.store.connection().unwrap().execute("INSERT INTO session_participants(session_id,conversation_id,tenant_id,participant_id,membership_epoch,role,state) SELECT ?1,conversation_id,tenant_id,participant_id,membership_epoch,'participant','present' FROM participants WHERE participant_id=?2",params![f.sid,member.participant_id]).unwrap();
    }
    assert_eq!(f.presence().len(), 4);
    // Presence alone is insufficient: a current original resource audience is
    // also required. This fixture supplies both halves of a future public join.
    assert!(f
        .store
        .authorize_session_binding(&f.keys[2], &f.sid)
        .is_err());
    for index in 2..4 {
        let member = f.member(index);
        f.store
            .connection()
            .unwrap()
            .execute(
                "INSERT INTO resource_audiences VALUES('session',?1,?2,?3,?4)",
                params![f.sid, f.cid, member.participant_id, member.membership_epoch],
            )
            .unwrap();
    }
    for key in &f.keys {
        f.store.authorize_session_binding(key, &f.sid).unwrap();
    }
    let limited = f
        .store
        .issue(
            &f.identities[0],
            Scope::communications(vec![f.identities[1].clone()]),
            crate::now() + 600,
        )
        .unwrap();
    assert_eq!(
        f.store
            .session_presence(&limited.id, &f.cid, &f.sid)
            .unwrap()
            .len(),
        2
    );
    let removed = f.member(2);
    let revision = f
        .store
        .membership_snapshot(&f.keys[0], &f.cid)
        .unwrap()
        .revision;
    f.store
        .execute(
            &f.keys[0],
            &Operation::Membership {
                request: MembershipRequest {
                    operation_id: "remove-third".into(),
                    conversation_id: f.cid.clone(),
                    expected_revision: revision,
                    change: MembershipAction::Remove {
                        participant_id: removed.participant_id.clone(),
                        reason: Some("leave media".into()),
                    },
                },
            },
        )
        .unwrap();
    assert_eq!(f.presence().len(), 3);
    assert!(f
        .store
        .authorize_session_binding(&f.keys[2], &f.sid)
        .is_err());
    for index in [0, 1, 3] {
        f.store
            .authorize_session_binding(&f.keys[index], &f.sid)
            .unwrap();
    }
    assert_eq!(
        f.store
            .authorize_session_binding(&f.keys[3], &f.sid)
            .unwrap()
            .state,
        "accepted"
    );
    let revision = f
        .store
        .membership_snapshot(&f.keys[0], &f.cid)
        .unwrap()
        .revision;
    f.store
        .execute(
            &f.keys[0],
            &Operation::Membership {
                request: MembershipRequest {
                    operation_id: "readmit-third".into(),
                    conversation_id: f.cid.clone(),
                    expected_revision: revision,
                    change: MembershipAction::Add {
                        identity_id: f.identities[2].clone(),
                        kind: IdentityKind::Human,
                        role: "participant".into(),
                    },
                },
            },
        )
        .unwrap();
    assert_eq!(f.member(2).membership_epoch, 2);
    assert_eq!(f.presence().len(), 3);
    assert!(f
        .store
        .authorize_session_binding(&f.keys[2], &f.sid)
        .is_err());
    assert!(f
        .store
        .session_presence(&f.keys[0], "other-conversation", &f.sid)
        .is_err());
}
