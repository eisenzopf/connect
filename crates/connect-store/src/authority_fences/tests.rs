use super::*;
use crate::{Scope, Store};
use connect_protocol::{provisioning::*, Limits, Operation, Outcome};
use std::{
    io::{BufRead, Write},
    process::{Child, Command, Stdio},
    sync::mpsc,
};

struct Fixture {
    dir: tempfile::TempDir,
    store: Arc<Store>,
    admin: String,
    user: String,
    identity: String,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap(),
        );
        let identity = store.create_identity("owner").unwrap().id;
        let now = chrono::Utc::now().timestamp();
        let admin = store
            .issue(&identity, Scope::operator(), now + 600)
            .unwrap()
            .id;
        let user = store
            .issue(&identity, Scope::collaboration(vec!["*".into()]), now + 600)
            .unwrap()
            .id;
        Self {
            dir,
            store,
            admin,
            user,
            identity,
        }
    }
    fn peer(&self) -> Store {
        Store::open_administration(self.store.path(), Limits::default()).unwrap()
    }
    fn scope(&self, mode: AuthorityMode) -> AuthorityScope {
        AuthorityScope::identity(&self.identity, mode).unwrap()
    }
}
pub(super) fn waiting(store: &Store, scope: &AuthorityScope) {
    let name = opaque(
        &store.authority.secret,
        match scope.kind {
            Kind::Identity => b"identity",
            Kind::Conversation => b"conversation",
            Kind::Record => b"record",
        },
        scope.id.as_bytes(),
    )
    .unwrap();
    let file = store.authority.file(&format!("{name}.admission")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => FileExt::unlock(&file).unwrap(),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return,
            Err(e) => panic!("admission probe failed: {e}"),
        };
        assert!(Instant::now() < deadline, "writer never reserved admission");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn writer_intent_stops_new_readers_held_proof_reuses_without_reentry_and_other_identity_proceeds() {
    let f = Fixture::new();
    let other = f.store.create_identity("other").unwrap().id;
    let read = f
        .store
        .read_authority(&f.user, None, &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    let retained = read.clone();
    let peer = f.peer();
    let operator = peer.revalidate(&f.admin).unwrap();
    let target = f.user.clone();
    let writer = std::thread::spawn(move || peer.revoke(&operator, &target));
    waiting(&f.store, &f.scope(AuthorityMode::Write));
    assert_eq!(
        f.store
            .read_authority(
                &f.user,
                None,
                &AuthorityWait::new(Duration::from_millis(25))
            )
            .unwrap_err()
            .code,
        "authority-busy"
    );
    assert!(f
        .store
        .authority_covers(&read, &[f.scope(AuthorityMode::Read)]));
    assert!(f.store.credential_metadata(&f.user).is_ok());
    let start = Instant::now();
    f.store
        .issue(
            &other,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    assert!(start.elapsed() < Duration::from_secs(1));
    drop(read);
    assert!(!writer.is_finished());
    drop(retained);
    writer.join().unwrap().unwrap();
    assert!(f.store.revalidate(&f.user).is_err());
    assert!(f
        .store
        .read_authority(&f.user, None, &AuthorityWait::new(Duration::from_secs(1)))
        .is_err());
}

#[test]
fn cancellation_deadline_and_partial_acquisition_release_every_scope() {
    let f = Fixture::new();
    let a = AuthorityScope::identity("scope_a", AuthorityMode::Write).unwrap();
    let b = AuthorityScope::identity("scope_b", AuthorityMode::Write).unwrap();
    let held = f
        .store
        .acquire_authority(
            &[AuthorityScope::identity("scope_b", AuthorityMode::Read).unwrap()],
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    let wait = AuthorityWait::new(Duration::from_secs(1));
    let cancellation = wait.cancel_on_drop();
    let owner = f.store.clone();
    let scopes = vec![b.clone(), a.clone(), a.clone()];
    let pending = std::thread::spawn(move || owner.acquire_authority(&scopes, &wait));
    waiting(&f.store, &b);
    drop(cancellation);
    assert_eq!(pending.join().unwrap().unwrap_err().code, "cancelled");
    let regained = f
        .store
        .acquire_authority(&[a.clone()], &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    drop(regained);
    assert_eq!(
        f.store
            .acquire_authority(
                &[a.clone(), b.clone()],
                &AuthorityWait::new(Duration::from_millis(15))
            )
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(held);
    let result = f
        .store
        .acquire_authority(
            &[
                b.clone(),
                a.clone(),
                AuthorityScope::identity("scope_a", AuthorityMode::Read).unwrap(),
            ],
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    assert!(result.covers(&[a, b]));
    let cancelled = AuthorityWait::new(Duration::from_secs(1));
    cancelled.cancel();
    assert_eq!(
        f.store.acquire_authority(&[], &cancelled).unwrap_err().code,
        "cancelled"
    );
}

#[cfg(unix)]
#[test]
fn namespace_privacy_symlink_alias_hardlink_refusal_and_foreign_guard_are_enforced() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let f = Fixture::new();
    let guard = f
        .store
        .read_authority(&f.user, None, &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    let alias = f.dir.path().join("alias.sqlite3");
    symlink(f.store.path(), &alias).unwrap();
    let same = Store::open_administration(&alias, Limits::default()).unwrap();
    assert_eq!(same.path(), f.store.path());
    assert_eq!(same.blob_directory, f.store.blob_directory);
    assert!(!Store::blob_directory_for(&alias).exists());
    assert!(same.authority_covers(&guard, &[f.scope(AuthorityMode::Read)]));
    // This branch runs on case-insensitive fixture volumes; a spelling alias
    // must share the descriptor cache as well as the kernel lock namespace.
    let case_alias = f.dir.path().join("STATE.SQLITE3");
    if case_alias.exists() {
        let same = Store::open_administration(&case_alias, Limits::default()).unwrap();
        assert!(same.authority_covers(&guard, &[f.scope(AuthorityMode::Read)]));
    }
    let other = Fixture::new();
    assert!(!other
        .store
        .authority_covers(&guard, &[f.scope(AuthorityMode::Read)]));
    assert_eq!(
        fs::metadata(&f.store.authority.directory)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    for entry in fs::read_dir(&f.store.authority.directory).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(!name.contains(&f.identity));
        assert_eq!(
            fs::metadata(entry.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let linked = f.dir.path().join("hardlink.sqlite3");
    fs::hard_link(f.store.path(), &linked).unwrap();
    assert_eq!(
        Store::open_administration(&linked, Limits::default())
            .err()
            .unwrap()
            .code,
        "authority-unavailable"
    );
    fs::remove_file(linked).unwrap();
    fs::set_permissions(
        &f.store.authority.directory,
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert_eq!(
        f.store
            .acquire_authority(
                &[f.scope(AuthorityMode::Read)],
                &AuthorityWait::new(Duration::from_secs(1))
            )
            .unwrap_err()
            .code,
        "authority-unavailable"
    );
    fs::set_permissions(
        &f.store.authority.directory,
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
}

#[test]
fn actual_key_and_device_retirement_wait_for_current_identity_delivery() {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use ring::signature::{Ed25519KeyPair, KeyPair};
    for device in [false, true] {
        let f = Fixture::new();
        f.store
            .execute_provisioning(
                &f.admin,
                ProvisioningCommand::Endpoint {
                    operation_id: "endpoint".into(),
                    identity_id: f.identity.clone(),
                    device: DeviceSpec {
                        id: "desktop".into(),
                        kind: "desktop".into(),
                        platform: "fixture".into(),
                    },
                    transport: EndpointTransport::Quic,
                    address: "quic://localhost:4444".into(),
                    capabilities: vec!["data".into()],
                    secret_ref: None,
                },
            )
            .unwrap();
        let key = Ed25519KeyPair::from_seed_unchecked(&[53; 32]).unwrap();
        f.store
            .execute_provisioning(
                &f.admin,
                ProvisioningCommand::SigningKey {
                    operation_id: "enroll".into(),
                    identity_id: f.identity.clone(),
                    credential_id: f.user.clone(),
                    device_id: "desktop".into(),
                    key_id: "signer".into(),
                    algorithm: SigningAlgorithm::EdDSA,
                    public_key: URL_SAFE_NO_PAD.encode(key.public_key().as_ref()),
                    not_before: crate::now() - 1,
                    expires_at: crate::now() + 300,
                    expected_revision: None,
                },
            )
            .unwrap();
        let read = f
            .store
            .read_authority(&f.user, None, &AuthorityWait::new(Duration::from_secs(1)))
            .unwrap();
        let command = if device {
            ProvisioningCommand::RevokeDevice {
                operation_id: "retire".into(),
                device_id: "desktop".into(),
                expected_revision: 0,
            }
        } else {
            ProvisioningCommand::RevokeSigningKey {
                operation_id: "retire".into(),
                key_id: "signer".into(),
                expected_revision: 0,
            }
        };
        let peer = f.peer();
        let admin = f.admin.clone();
        let writer = std::thread::spawn(move || peer.execute_provisioning(&admin, command));
        waiting(&f.store, &f.scope(AuthorityMode::Write));
        assert!(f
            .store
            .resolve_peer_signing_key(&f.user, "tnt_default", "signer", "EdDSA")
            .is_ok());
        drop(read);
        writer.join().unwrap().unwrap();
        assert!(f
            .store
            .resolve_peer_signing_key(&f.user, "tnt_default", "signer", "EdDSA")
            .is_err());
    }
}

#[test]
fn membership_removal_and_other_member_revocation_wait_for_snapshot_delivery() {
    use connect_protocol::membership::{MembershipAction, MembershipRequest};
    for identity_revocation in [false, true] {
        let f = Fixture::new();
        let other = f.store.create_identity("other").unwrap().id;
        let Outcome::Conversation(conversation) = f
            .store
            .execute(
                &f.admin,
                &Operation::Create {
                    request_id: "conversation".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![
                        other.clone(),
                    ]),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        let snapshot = f
            .store
            .membership_snapshot(&f.admin, &conversation.id)
            .unwrap();
        let part = snapshot
            .participants
            .iter()
            .find(|p| p.identity_id == other)
            .unwrap()
            .participant_id
            .clone();
        let read = f
            .store
            .read_authority(
                &f.user,
                Some(&conversation.id),
                &AuthorityWait::new(Duration::from_secs(1)),
            )
            .unwrap();
        let peer = f.peer();
        let admin = f.admin.clone();
        let cid = conversation.id.clone();
        let target = other.clone();
        let writer = std::thread::spawn(move || {
            if identity_revocation {
                peer.execute_provisioning(
                    &admin,
                    ProvisioningCommand::RevokeIdentity {
                        operation_id: "retire".into(),
                        identity_id: target,
                        expected_revision: 0,
                    },
                )
                .map(|_| ())
            } else {
                peer.execute_membership(
                    &admin,
                    MembershipRequest {
                        operation_id: "remove".into(),
                        conversation_id: cid,
                        expected_revision: snapshot.revision,
                        change: MembershipAction::Remove {
                            participant_id: part,
                            reason: None,
                        },
                    },
                )
                .map(|_| ())
            }
        });
        let first = if identity_revocation {
            &other
        } else {
            std::cmp::min(&other, &f.identity)
        };
        waiting(
            &f.store,
            &AuthorityScope::identity(first, AuthorityMode::Write).unwrap(),
        );
        assert_eq!(
            f.store
                .membership_snapshot_with_authority(&f.admin, &conversation.id, &read)
                .unwrap()
                .participants
                .len(),
            2
        );
        drop(read);
        writer.join().unwrap().unwrap();
        assert_eq!(
            f.store
                .membership_snapshot(&f.admin, &conversation.id)
                .unwrap()
                .participants
                .len(),
            1
        );
    }
}

struct Process {
    child: Child,
    lines: mpsc::Receiver<String>,
}
impl Process {
    fn start(f: &Fixture, role: &str) -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "authority_fences::tests::process_worker",
                "--ignored",
                "--nocapture",
            ])
            .env("CONNECT_AUTHORITY_TEST_PATH", f.store.path())
            .env("CONNECT_AUTHORITY_TEST_ROLE", role)
            .env("CONNECT_AUTHORITY_TEST_ADMIN", &f.admin)
            .env("CONNECT_AUTHORITY_TEST_USER", &f.user)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let out = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(out).lines() {
                if tx.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        Self { child, lines }
    }
    fn line(&self, label: &str) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let line = self
                .lines
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            if line.contains(label) {
                return;
            }
        }
    }
    fn go(&mut self) {
        self.child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(b"go\n")
            .unwrap();
        self.child.stdin.as_mut().unwrap().flush().unwrap();
    }
    fn finish(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                return;
            }
            assert!(Instant::now() < deadline, "child did not finish");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn actual_child_process_revocation_waits_for_parent_delivery_guard_and_admin_open_preserves_runtime(
) {
    let f = Fixture::new();
    let Outcome::Conversation(conversation) = f
        .store
        .execute(
            &f.user,
            &Operation::Create {
                request_id: "conversation".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![]),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let endpoint = f
        .store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::Endpoint {
                operation_id: "endpoint".into(),
                identity_id: f.identity.clone(),
                device: DeviceSpec {
                    id: "device".into(),
                    kind: "desktop".into(),
                    platform: "fixture".into(),
                },
                transport: EndpointTransport::Quic,
                address: "quic://localhost:4444".into(),
                capabilities: vec!["data".into()],
                secret_ref: None,
            },
        )
        .unwrap();
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = endpoint else {
        panic!()
    };
    let member = f
        .store
        .membership_snapshot(&f.user, &conversation.id)
        .unwrap()
        .participants
        .remove(0);
    let tenant = f.store.credential_metadata(&f.user).unwrap().tenant_id;
    let c = rusqlite::Connection::open(f.store.path()).unwrap();
    c.execute("INSERT INTO sessions(id,conversation_id,initiator_id,participant_id,state,media,created_at,updated_at) VALUES('session',?1,?2,?2,'active','[\"data\"]',1,1)",rusqlite::params![conversation.id,f.identity]).unwrap();
    c.execute(
        "INSERT INTO session_participants VALUES('session',?1,?2,?3,1,'participant','present')",
        rusqlite::params![conversation.id, tenant, member.participant_id],
    )
    .unwrap();
    c.execute("INSERT INTO connections(id,session_id,participant_id,membership_epoch,identity_id,tenant_id,endpoint_id,generation,state,observed_runtime_id,observed_at,readiness_json) VALUES('connection','session',?1,1,?2,?3,?4,3,'active',?5,1,'{}')",rusqlite::params![member.participant_id,f.identity,tenant,endpoint_id,f.store.runtime_epoch()]).unwrap();
    let read = f
        .store
        .read_authority(
            &f.user,
            Some(&conversation.id),
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    let mut child = Process::start(&f, "revoke");
    child.line("FENCE READY");
    let current: (String, u64, String) = c
        .query_row(
            "SELECT state,generation,observed_runtime_id FROM connections WHERE id='connection'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        current,
        ("active".into(), 3, f.store.runtime_epoch().into())
    );
    child.go();
    waiting(&f.store, &f.scope(AuthorityMode::Write));
    assert!(f.store.revalidate(&f.user).is_ok());
    assert!(child.child.try_wait().unwrap().is_none());
    drop(read);
    child.line("FENCE COMMITTED");
    child.finish();
    assert!(f.store.revalidate(&f.user).is_err());
}

#[test]
fn actual_child_death_releases_kernel_reader_without_persistent_phantom_authority() {
    let f = Fixture::new();
    let mut child = Process::start(&f, "reader");
    child.line("FENCE HELD");
    let peer = f.peer();
    let actor = peer.revalidate(&f.admin).unwrap();
    let target = f.user.clone();
    let writer = std::thread::spawn(move || peer.revoke(&actor, &target));
    waiting(&f.store, &f.scope(AuthorityMode::Write));
    assert!(f.store.revalidate(&f.user).is_ok());
    child.child.kill().unwrap();
    child.child.wait().unwrap();
    writer.join().unwrap().unwrap();
    assert!(f.store.revalidate(&f.user).is_err());
}

#[test]
#[ignore = "explicitly executed by the cross-process parent tests"]
fn process_worker() {
    let store = Store::open_administration(
        Path::new(&std::env::var("CONNECT_AUTHORITY_TEST_PATH").unwrap()),
        Limits::default(),
    )
    .unwrap();
    let user = std::env::var("CONNECT_AUTHORITY_TEST_USER").unwrap();
    let role = std::env::var("CONNECT_AUTHORITY_TEST_ROLE").unwrap();
    if role == "reader" {
        let _guard = store
            .read_authority(&user, None, &AuthorityWait::new(Duration::from_secs(1)))
            .unwrap();
        println!("FENCE HELD");
        std::io::stdout().flush().unwrap();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
    } else {
        let admin = std::env::var("CONNECT_AUTHORITY_TEST_ADMIN").unwrap();
        println!("FENCE READY");
        std::io::stdout().flush().unwrap();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
        store
            .revoke(&store.revalidate(&admin).unwrap(), &user)
            .unwrap();
        println!("FENCE COMMITTED");
        std::io::stdout().flush().unwrap();
    }
}

mod mutations;

mod names;
mod runtime;

#[test]
fn immediate_large_scene_admission_preserves_every_scope() {
    let f = Fixture::new();
    let mut scopes = (0..50)
        .map(|index| {
            AuthorityScope::identity(&format!("large_scene_{index:02}"), AuthorityMode::Read)
                .unwrap()
        })
        .collect::<Vec<_>>();
    scopes.push(AuthorityScope::conversation("large_scene", AuthorityMode::Read).unwrap());
    let retained = f.store.try_acquire_authority(&scopes).unwrap();
    assert!(f.store.authority_covers(&retained, &scopes));
    let start = Instant::now();
    for _ in 0..100 {
        let fresh = f.store.try_acquire_authority(&scopes).unwrap();
        assert!(f.store.authority_covers(&fresh, &scopes));
    }
    eprintln!(
        "51-scope fresh admission mean_us={}",
        start.elapsed().as_micros() / 100
    );
    for scope in &scopes {
        let mut write = scope.clone();
        write.mode = AuthorityMode::Write;
        assert_eq!(
            f.store.try_acquire_authority(&[write]).unwrap_err().code,
            "authority-busy"
        );
    }
    drop(retained);
    let writes = scopes
        .into_iter()
        .map(|mut scope| {
            scope.mode = AuthorityMode::Write;
            scope
        })
        .collect::<Vec<_>>();
    assert!(f.store.try_acquire_authority(&writes).is_ok());
}

#[test]
fn immediate_admission_refuses_queued_writer_and_releases_partial_scopes() {
    let f = Fixture::new();
    let a = AuthorityScope::identity("immediate_a", AuthorityMode::Read).unwrap();
    let b = AuthorityScope::identity("immediate_b", AuthorityMode::Read).unwrap();
    let held = f.store.try_acquire_authority(&[b.clone()]).unwrap();
    let peer = f.peer();
    let writer_scope = AuthorityScope::identity("immediate_b", AuthorityMode::Write).unwrap();
    let pending_scope = writer_scope.clone();
    let writer = std::thread::spawn(move || {
        peer.acquire_authority(
            &[pending_scope],
            &AuthorityWait::new(Duration::from_secs(2)),
        )
    });
    waiting(&f.store, &writer_scope);
    let start = Instant::now();
    assert_eq!(
        f.store
            .try_acquire_authority(&[a.clone(), b.clone()])
            .unwrap_err()
            .code,
        "authority-busy"
    );
    assert!(start.elapsed() < Duration::from_millis(500));
    let recovered = f
        .store
        .try_acquire_authority(&[
            AuthorityScope::identity("immediate_a", AuthorityMode::Write).unwrap(),
        ])
        .unwrap();
    drop(recovered);
    assert!(f.store.authority_covers(&held, &[b.clone()]));
    drop(held);
    let writer = writer.join().unwrap().unwrap();
    assert_eq!(
        f.store
            .try_acquire_authority(&[b.clone()])
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(writer);
    assert!(f.store.try_acquire_authority(&[a, b]).is_ok());
}

#[test]
fn immediate_admission_refuses_reader_map_contention_without_waiting() {
    let f = Fixture::new();
    let scope = f.scope(AuthorityMode::Read);
    let map = f.store.authority.readers.lock().unwrap();
    let store = f.store.clone();
    let target = scope.clone();
    let (sent, received) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = store.try_acquire_authority(&[target]);
        sent.send(result.err().map(|error| error.code)).unwrap();
    });
    let result = received.recv_timeout(Duration::from_millis(500));
    drop(map);
    worker.join().unwrap();
    assert_eq!(result.unwrap(), Some("authority-busy".into()));
    assert!(f.store.try_acquire_authority(&[scope]).is_ok());
}

#[test]
fn immediate_sql_refuses_mutex_and_database_contention_and_restores_timeout() {
    let f = Fixture::new();
    let held = f.store.connection().unwrap();
    let start = Instant::now();
    match f.store.try_connection() {
        Err(error) => assert_eq!(error.code, "authority-busy"),
        Ok(_) => panic!("immediate SQL entered an occupied mutex"),
    }
    assert!(start.elapsed() < Duration::from_millis(500));
    drop(held);
    let external = rusqlite::Connection::open(f.store.path()).unwrap();
    external.execute_batch("BEGIN IMMEDIATE").unwrap();
    let immediate = f.store.try_connection().unwrap();
    let start = Instant::now();
    assert!(immediate
        .execute("UPDATE installation SET cursor_key=cursor_key", [])
        .is_err());
    assert!(start.elapsed() < Duration::from_millis(500));
    drop(immediate);
    let ordinary = f.store.connection().unwrap();
    assert_eq!(
        ordinary
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, u64>(0))
            .unwrap(),
        2000
    );
    drop(ordinary);
    external.execute_batch("ROLLBACK").unwrap();
    assert!(f.store.try_connection().is_ok());
}
