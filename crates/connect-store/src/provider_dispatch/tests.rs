#![allow(clippy::unwrap_used)]
use super::*;
use crate::{Route, Scope};
use connect_protocol::{Limits, Operation, Outcome};
pub(crate) struct Fixture {
    pub(crate) dir: tempfile::TempDir,
    pub(crate) store: Arc<Store>,
    pub(crate) sender: String,
    pub(crate) key: String,
    pub(crate) bot: String,
    pub(crate) bot_key: String,
    pub(crate) admin: String,
    pub(crate) cid: String,
}
impl Fixture {
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap(),
        );
        let sender = store.create_identity("sender").unwrap().id;
        let bot = store.create_identity("bot").unwrap().id;
        let admin = store
            .issue(&sender, Scope::operator(), crate::now() + 600)
            .unwrap()
            .id;
        let key = store
            .issue(&sender, Scope::operator(), crate::now() + 600)
            .unwrap()
            .id;
        let bot_key = store
            .issue(&bot, Scope::operator(), crate::now() + 600)
            .unwrap()
            .id;
        let Outcome::Conversation(conv) = store
            .execute(
                &key,
                &Operation::Create {
                    request_id: "conversation".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![
                        bot.clone()
                    ]),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        store
            .configure_route(
                &admin,
                &Route {
                    id: "route".into(),
                    identity_id: bot.clone(),
                    credential_id: bot_key.clone(),
                    conversation_id: conv.id.clone(),
                    connector: "vapi".into(),
                    target: "12345678-1234-1234-1234-123456789012".into(),
                    source: None,
                    enabled: true,
                    incoming_to: connect_protocol::messages::MessageAudience::all(),
                },
            )
            .unwrap();
        Self {
            dir,
            store,
            sender,
            key,
            bot,
            bot_key,
            admin,
            cid: conv.id,
        }
    }
    pub(crate) fn send(&self, id: &str) {
        self.store
            .execute(
                &self.key,
                &Operation::Send {
                    request_id: id.into(),
                    conversation_id: self.cid.clone(),
                    message: connect_protocol::messages::MessageDraft::text_all(id),
                },
            )
            .unwrap();
    }
    pub(crate) fn claim(&self) -> ProviderJob {
        self.send("first");
        self.store.claim_provider_job().unwrap().unwrap()
    }
    pub(crate) fn admit(
        &self,
        role: &Arc<ProviderWorkerOwnership>,
        job: &ProviderJob,
    ) -> Result<DispatchPermit> {
        self.store
            .admit_provider_dispatch(role, job, &AuthorityWait::new(Duration::from_secs(1)))
    }
    pub(crate) fn state(&self, id: &str) -> String {
        self.store
            .connection()
            .unwrap()
            .query_row("SELECT state FROM provider_jobs WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .unwrap()
    }
}
#[test]
fn dispatch_is_single_use_exact_claim_and_body_bound_with_no_foreign_role_fallback() {
    let f = Fixture::new();
    let job = f.claim();
    let role = f.store.acquire_provider_worker().unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let mut copied = job.clone();
    copied.body.push_str(" tampered");
    assert_eq!(f.admit(&role, &copied).unwrap_err().code, "forbidden");
    copied = job.clone();
    copied.context = None;
    assert_eq!(f.admit(&role, &copied).unwrap_err().code, "forbidden");
    copied = job.clone();
    copied.previous_chat_id = Some("borrowed-chat".into());
    assert_eq!(f.admit(&role, &copied).unwrap_err().code, "conflict");
    let other = Fixture::new();
    let foreign = other.store.acquire_provider_worker().unwrap();
    assert_eq!(f.admit(&foreign, &job).unwrap_err().code, "forbidden");
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let permit = f.admit(&role, &job).unwrap();
    assert!(permit.check().is_ok());
    let admitted = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(f.admit(&role, &job).unwrap_err().code, "conflict");
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), admitted);
    permit.admit_effect().unwrap();
    assert_eq!(
        permit.clone().admit_effect(),
        Err(connect_connector_api::Failure::Uncertain)
    );
    drop(permit);
    assert_eq!(
        f.store.reject_provider_claim(&job).unwrap_err().code,
        "forbidden"
    );
}
#[test]
fn current_source_changes_between_claim_and_dispatch_deny_without_admission() {
    for mode in ["credential", "route", "member"] {
        let f = Fixture::new();
        let job = f.claim();
        let role = f.store.acquire_provider_worker().unwrap();
        match mode {
            "credential" => {
                let actor =
                    actor_from_connection(&f.store.connection().unwrap(), &f.admin).unwrap();
                f.store.revoke(&actor, &f.key).unwrap();
            }
            "route" => {
                let mut route = job.route.clone();
                route.enabled = false;
                f.store.configure_route(&f.admin, &route).unwrap();
            }
            _ => {
                use connect_protocol::membership::*;
                let snap = f.store.membership_snapshot(&f.admin, &f.cid).unwrap();
                let part = snap
                    .participants
                    .iter()
                    .find(|p| p.identity_id == f.bot)
                    .unwrap();
                f.store
                    .execute_membership(
                        &f.admin,
                        MembershipRequest {
                            operation_id: "remove".into(),
                            conversation_id: f.cid.clone(),
                            expected_revision: snap.revision,
                            change: MembershipAction::Remove {
                                participant_id: part.participant_id.clone(),
                                reason: None,
                            },
                        },
                    )
                    .unwrap();
            }
        }
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        assert!(f.admit(&role, &job).is_err());
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
        f.store.reject_provider_claim(&job).unwrap();
        assert_eq!(f.state(&job.id), "cancelled");
    }
}
#[test]
fn retired_worker_keeps_pending_authority_until_last_transport_clone_and_does_not_adopt() {
    let f = Fixture::new();
    let job = f.claim();
    let role = f.store.acquire_provider_worker().unwrap();
    let permit = f.admit(&role, &job).unwrap();
    let transport = permit.clone();
    drop(permit);
    role.retire();
    drop(role);
    assert!(transport.check().is_err());
    assert_eq!(
        f.store.acquire_provider_worker().unwrap_err().code,
        "runtime-role-busy"
    );
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    std::thread::scope(|threads| {
        let writer = threads.spawn(|| {
            let actor = actor_from_connection(&f.store.connection().unwrap(), &f.admin)?;
            f.store.revoke(&actor, &f.key)
        });
        crate::authority_fences::test_wait_for_writer(
            &f.store,
            &AuthorityScope::identity(&f.sender, AuthorityMode::Write).unwrap(),
        );
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
        drop(transport);
        writer.join().unwrap().unwrap();
    });
    f.store.acquire_provider_worker().unwrap();
}
#[test]
fn context_revalidation_is_watermark_bounded_and_does_not_export_changed_session_metadata() {
    let f = Fixture::new();
    let job = f.claim();
    f.send("later-source");
    let role = f.store.acquire_provider_worker().unwrap();
    let permit = f.admit(&role, &job).unwrap();
    drop(permit);
    let f = Fixture::new();
    let Outcome::Session(session) = f
        .store
        .execute(
            &f.key,
            &Operation::SessionInvite {
                request_id: "invite".into(),
                conversation_id: f.cid.clone(),
                participant_id: f.bot.clone(),
                media: vec!["audio".into()],
            },
        )
        .unwrap()
    else {
        panic!()
    };
    f.store
        .execute(
            &f.bot_key,
            &Operation::SessionAccept {
                request_id: "accept".into(),
                conversation_id: f.cid.clone(),
                session_id: session.id.clone(),
            },
        )
        .unwrap();
    let job = f.claim();
    let role = f.store.acquire_provider_worker().unwrap();
    let selected: serde_json::Value =
        serde_json::from_str(job.context.as_deref().unwrap()).unwrap();
    assert!(selected.to_string().contains("accepted"));
    let view: connect_protocol::ContextView = serde_json::from_value(selected.clone()).unwrap();
    assert!(view
        .items
        .iter()
        .filter(|item| item.kind.starts_with("session_"))
        .all(|item| {
            let body: serde_json::Value = serde_json::from_str(&item.body).unwrap();
            body["state"] != "ended"
        }));
    f.store
        .execute(
            &f.key,
            &Operation::SessionEnd {
                request_id: "end".into(),
                conversation_id: f.cid.clone(),
                session_id: session.id,
                reason: None,
            },
        )
        .unwrap();
    // Dispatch retains the exact accepted snapshot selected at its watermark.
    // A later Session end is neither substituted into that snapshot nor added
    // to this already selected export. Current authorization is still checked.
    let permit = f.admit(&role, &job).unwrap();
    assert!(permit.check().is_ok());
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(job.context.as_deref().unwrap()).unwrap(),
        selected
    );
}
#[test]
fn recovery_distinguishes_unadmitted_local_claims_from_possible_effects_and_restored_pending() {
    let f = Fixture::new();
    let job = f.claim();
    assert_eq!(f.store.recover_provider_jobs().unwrap(), 1);
    assert_eq!(f.state(&job.id), "cancelled");
    let f = Fixture::new();
    let job = f.claim();
    let role = f.store.acquire_provider_worker().unwrap();
    let permit = f.admit(&role, &job).unwrap();
    drop(permit);
    assert_eq!(f.store.recover_provider_jobs().unwrap(), 1);
    assert_eq!(f.state(&job.id), "uncertain");
    assert!(f.store.claim_provider_job().unwrap().is_none());
    let f = Fixture::new();
    f.send("pending-at-backup");
    let backup = f.dir.path().join("copy.sqlite3");
    f.store.backup(&backup).unwrap();
    let original = f.store.claim_provider_job().unwrap().unwrap();
    let role = f.store.acquire_provider_worker().unwrap();
    let permit = f.admit(&role, &original).unwrap();
    drop(permit);
    let copy = Store::open(&backup, Limits::default()).unwrap();
    assert_eq!(copy.recover_provider_jobs().unwrap(), 1);
    assert!(copy.claim_provider_job().unwrap().is_none());
    assert_eq!(
        copy.connection()
            .unwrap()
            .query_row("SELECT state FROM provider_jobs", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "uncertain"
    );
    assert_eq!(f.state(&original.id), "sending");
}
#[test]
fn admitted_custody_and_queued_route_order_survive_backup_without_a_second_creation() {
    let f = Fixture::new();
    let first = f.claim();
    f.send("second");
    assert!(f.store.claim_provider_job().unwrap().is_none());
    let role = f.store.acquire_provider_worker().unwrap();
    let permit = f.admit(&role, &first).unwrap();
    drop(permit);
    let backup = f.dir.path().join("admitted-copy.sqlite3");
    let snap = f.store.backup(&backup).unwrap();
    assert_eq!(snap, Store::inspect_snapshot(&backup).unwrap());
    let copy = Store::open(&backup, Limits::default()).unwrap();
    assert_eq!(copy.recover_provider_jobs().unwrap(), 2);
    assert!(copy.claim_provider_job().unwrap().is_none());
    assert_eq!(
        copy.connection()
            .unwrap()
            .query_row("SELECT SUM(admitted) FROM provider_dispatch", [], |r| r
                .get::<_, usize>(
                0
            ))
            .unwrap(),
        1
    );
}

#[test]
fn restored_provider_recovery_drains_1030_conversations_in_bounded_exact_batches() {
    let f = Fixture::new();
    f.send("source");
    // Isolated persisted fixture: real queued source row, distinct current
    // Conversations/routes/messages, and the enqueue-time file identity stamp.
    {
        let mut c = f.store.connection().unwrap();
        let tx = c.transaction().unwrap();
        for n in 0..1029 {
            let cid = format!("recovery-cid-{n}");
            let route = format!("recovery-route-{n}");
            let msg = format!("recovery-msg-{n}");
            let job = format!("recovery-job-{n}");
            tx.execute("INSERT INTO conversations(id) VALUES(?1)", [&cid])
                .unwrap();
            tx.execute("INSERT INTO participants(conversation_id,identity_id,participant_id,role) VALUES(?1,?2,?3,'agent'),(?1,?4,?5,'agent')", params![cid,f.sender,format!("sender-{n}"),f.bot,format!("bot-{n}")]).unwrap();
            tx.execute("INSERT INTO connector_routes(id,identity_id,credential_id,conversation_id,connector,target) SELECT ?1,identity_id,credential_id,?2,connector,target FROM connector_routes WHERE id='route'", params![route,cid]).unwrap();
            tx.execute("INSERT INTO messages(id,conversation_id,sender,body,sender_participant_id) VALUES(?1,?2,?3,'fixture',?4)", params![msg,cid,f.sender,format!("sender-{n}")]).unwrap();
            tx.execute("INSERT INTO provider_jobs(id,route_id,message_id,credential_id,state) VALUES(?1,?2,?3,?4,'pending')", params![job,route,msg,f.key]).unwrap();
            tx.execute(
                "INSERT INTO provider_dispatch(job_id,state_identity) VALUES(?1,?2)",
                params![job, f.store.authority.state_identity()],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
    let backup = f.dir.path().join("large-provider-copy.sqlite3");
    f.store.backup(&backup).unwrap();
    let copy = Store::open(&backup, Limits::default()).unwrap();
    let before = Store::inspect_snapshot(copy.path()).unwrap();
    let reader = copy
        .acquire_authority(
            &[AuthorityScope::conversation(&f.cid, AuthorityMode::Read).unwrap()],
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    assert_eq!(
        copy.recover_provider_batch().unwrap_err().code,
        "authority-busy"
    );
    assert_eq!(Store::inspect_snapshot(copy.path()).unwrap(), before);
    drop(reader);
    assert_eq!(copy.recover_provider_batch().unwrap(), 64);
    assert_eq!(copy.recover_provider_jobs().unwrap(), 966);
    assert_eq!(copy.recover_provider_jobs().unwrap(), 0);
    let c = copy.connection().unwrap();
    let states: (usize, usize) = c
        .query_row(
            "SELECT COUNT(*),SUM(state='uncertain') FROM provider_jobs",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(states, (1030, 1030));
    assert_eq!(
        c.query_row("SELECT SUM(admitted) FROM provider_dispatch", [], |r| r
            .get::<_, usize>(
            0
        ))
        .unwrap(),
        0
    );
    drop(c);
    assert!(copy.claim_provider_job().unwrap().is_none());
}

#[test]
fn dispatch_deadline_is_bounded_by_the_original_credential_expiry() {
    let f = Fixture::new();
    let expiry = crate::now() + 2;
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET expires_at=?2 WHERE id=?1",
            params![f.key, expiry],
        )
        .unwrap();
    let job = f.claim();
    let role = f.store.acquire_provider_worker().unwrap();
    let permit = f.admit(&role, &job).unwrap();
    assert!(permit.deadline().saturating_duration_since(Instant::now()) <= Duration::from_secs(2));
    std::thread::sleep(
        permit.deadline().saturating_duration_since(Instant::now()) + Duration::from_millis(10),
    );
    assert!(permit.check().is_err());
    assert!(permit.admit_effect().is_err());
    drop(permit);
    assert_eq!(f.store.recover_provider_jobs().unwrap(), 1);
    assert_eq!(f.state(&job.id), "uncertain");
}
