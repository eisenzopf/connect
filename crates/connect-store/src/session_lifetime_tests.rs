#![allow(clippy::unwrap_used)]
use crate::{IssuedCredential, Store};
use connect_protocol::{Limits, Operation, Outcome};

struct Fixture {
    _directory: tempfile::TempDir,
    store: Store,
    owner: String,
    key: IssuedCredential,
    peer: IssuedCredential,
    conversation: String,
    session: String,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let store =
            Store::initialize(&directory.path().join("state.sqlite3"), Limits::default()).unwrap();
        let owner = store.create_identity("owner").unwrap().id;
        let other = store.create_identity("other").unwrap().id;
        let expires = crate::now() + 600;
        let key = store
            .issue(&owner, crate::Scope::operator(), expires)
            .unwrap();
        let peer = store
            .issue(
                &other,
                crate::Scope::communications(vec![owner.clone()]),
                expires,
            )
            .unwrap();
        let Outcome::Conversation(conversation) = store
            .execute(
                &key.id,
                &Operation::Create {
                    request_id: "conversation".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![
                        other.clone(),
                    ]),
                },
            )
            .unwrap()
        else {
            panic!("conversation");
        };
        let Outcome::Session(session) = store
            .execute(
                &key.id,
                &Operation::SessionInvite {
                    request_id: "invite".into(),
                    conversation_id: conversation.id.clone(),
                    participant_id: other,
                    media: vec!["audio".into(), "video".into()],
                },
            )
            .unwrap()
        else {
            panic!("session");
        };
        store
            .execute(
                &peer.id,
                &Operation::SessionAccept {
                    request_id: "accept".into(),
                    conversation_id: conversation.id.clone(),
                    session_id: session.id.clone(),
                },
            )
            .unwrap();
        Self {
            _directory: directory,
            store,
            owner,
            key,
            peer,
            conversation: conversation.id,
            session: session.id,
        }
    }
    fn end(&self) {
        self.store
            .execute(
                &self.peer.id,
                &Operation::SessionEnd {
                    request_id: "end".into(),
                    conversation_id: self.conversation.clone(),
                    session_id: self.session.clone(),
                    reason: None,
                },
            )
            .unwrap();
    }
}

#[test]
fn current_authorized_end_is_cleanup_only_and_preserves_conversation_commands() {
    let f = Fixture::new();
    assert_eq!(
        f.store
            .authorize_session_lifetime(&f.key.id, &f.session)
            .unwrap()
            .state,
        "accepted"
    );
    f.end();
    assert_eq!(
        f.store
            .authorize_session_lifetime(&f.key.id, &f.session)
            .unwrap()
            .state,
        "ended"
    );
    assert_eq!(
        f.store
            .authorize_session_binding(&f.key.id, &f.session)
            .unwrap_err()
            .code,
        "forbidden"
    );
    f.store
        .execute(
            &f.key.id,
            &Operation::Send {
                request_id: "after-end".into(),
                conversation_id: f.conversation.clone(),
                message: connect_protocol::messages::MessageDraft::text_all(
                    "Conversation continues after media",
                ),
            },
        )
        .unwrap();
}

#[test]
fn revoked_current_credential_never_becomes_terminal_cleanup_in_either_order() {
    for end_first in [false, true] {
        let f = Fixture::new();
        let actor = f.store.revalidate(&f.key.id).unwrap();
        if end_first {
            f.end();
        }
        f.store.revoke(&actor, &f.key.id).unwrap();
        if !end_first {
            f.end();
        }
        assert_eq!(
            f.store
                .authorize_session_lifetime(&f.key.id, &f.session)
                .unwrap_err()
                .code,
            "unauthorized"
        );
    }
}

#[test]
fn lost_membership_unknown_and_storage_failure_never_classify_as_terminal() {
    let f = Fixture::new();
    f.end();
    assert_eq!(
        f.store
            .authorize_session_lifetime(&f.key.id, "unknown")
            .unwrap_err()
            .code,
        "forbidden"
    );
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE participants SET active=0 WHERE conversation_id=?1 AND identity_id=?2",
            rusqlite::params![f.conversation, f.owner],
        )
        .unwrap();
    assert_eq!(
        f.store
            .authorize_session_lifetime(&f.key.id, &f.session)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let g = Fixture::new();
    g.end();
    g.store
        .connection()
        .unwrap()
        .execute_batch("ALTER TABLE sessions RENAME TO deliberately_unavailable_sessions")
        .unwrap();
    assert_eq!(
        g.store
            .authorize_session_lifetime(&g.key.id, &g.session)
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
}
