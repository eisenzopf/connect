use super::*;
use crate::Scope;
use connect_protocol::Limits;

struct Fixture {
    _directory: tempfile::TempDir,
    store: Store,
    identity: String,
    credential: String,
}
impl Fixture {
    fn new(alias: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let store =
            Store::initialize(&directory.path().join("state.sqlite3"), Limits::default()).unwrap();
        // The same scope ID in separate Stores catches a foreign Manager proof
        // even when its redacted scope names would otherwise match exactly.
        let identity = "identity_boundary_shared".to_owned();
        store
            .connection()
            .unwrap()
            .execute(
                "INSERT INTO identities(id,alias,capabilities) VALUES(?1,?2,'[]')",
                rusqlite::params![identity, alias],
            )
            .unwrap();
        let credential = store
            .issue(&identity, Scope::operator(), crate::now() + 600)
            .unwrap()
            .id;
        Self {
            _directory: directory,
            store,
            identity,
            credential,
        }
    }
    fn scope(&self, mode: AuthorityMode) -> AuthorityScope {
        AuthorityScope::identity(&self.identity, mode).unwrap()
    }
    fn writer(&self) -> Arc<AuthorityGuardSet> {
        self.store.write_identity_authority(&self.identity).unwrap()
    }
}

#[test]
fn raw_sql_cannot_revoke_a_live_store_credential() {
    let fixture = Fixture::new("boundary-raw");
    let raw = Connection::open(fixture.store.path()).unwrap();
    let error = raw
        .execute(
            "UPDATE credentials SET revoked=1 WHERE id=?1",
            [&fixture.credential],
        )
        .unwrap_err();
    assert!(
        error.to_string().contains(FUNCTION),
        "wrong refusal: {error}"
    );
    assert!(fixture.store.revalidate(&fixture.credential).is_ok());
}

#[test]
fn actual_row_refuses_missing_read_foreign_and_wrong_identity_writers() {
    let fixture = Fixture::new("boundary-owner");
    let other = fixture.store.create_identity("boundary-other").unwrap().id;
    let foreign = Fixture::new("boundary-foreign");
    let foreign_writer = foreign.writer();
    assert!(fixture
        .store
        .connection_with_authority(&foreign_writer)
        .is_err());
    {
        let connection = fixture.store.connection().unwrap();
        assert!(
            connection
                .bind_mutation_authority(
                    &foreign.store,
                    &foreign_writer,
                    &[foreign.scope(AuthorityMode::Write)]
                )
                .is_err(),
            "direct bind must belong to this exact Store SQL state"
        );
    }
    drop(foreign_writer);
    {
        let connection = fixture.store.connection().unwrap();
        assert!(connection
            .execute(
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                [&fixture.credential]
            )
            .is_err());
    }
    {
        let read = fixture
            .store
            .try_acquire_authority(&[fixture.scope(AuthorityMode::Read)])
            .unwrap();
        let connection = fixture.store.connection_with_authority(&read).unwrap();
        assert!(connection
            .execute(
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                [&fixture.credential]
            )
            .is_err());
    }
    {
        let wrong = fixture.store.write_identity_authority(&other).unwrap();
        let connection = fixture.store.connection_with_authority(&wrong).unwrap();
        assert!(connection
            .execute(
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                [&fixture.credential]
            )
            .is_err());
    }
    assert!(fixture.store.revalidate(&fixture.credential).is_ok());
}

#[test]
fn sql_unit_pins_the_original_writer_and_rolls_back_before_releasing_it() {
    let fixture = Fixture::new("boundary-custody");
    let writer = fixture.writer();
    let weak = Arc::downgrade(&writer);
    let connection = fixture.store.connection_with_authority(&writer).unwrap();
    drop(writer);
    assert!(
        weak.upgrade().is_some(),
        "SQL must pin the actual original writer"
    );
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    connection
        .execute(
            "UPDATE credentials SET revoked=1 WHERE id=?1",
            [&fixture.credential],
        )
        .unwrap();
    assert!(fixture
        .store
        .try_acquire_authority(&[fixture.scope(AuthorityMode::Read)])
        .is_err());
    // Deliberately leave the SQL transaction open. Unit Drop must roll back
    // before releasing its Write proof or permitting the next Store caller.
    drop(connection);
    assert!(weak.upgrade().is_none());
    let read = fixture
        .store
        .try_acquire_authority(&[fixture.scope(AuthorityMode::Read)])
        .unwrap();
    assert!(fixture.store.revalidate(&fixture.credential).is_ok());
    drop(read);
    let connection = fixture.store.connection().unwrap();
    assert!(
        connection
            .execute(
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                [&fixture.credential]
            )
            .is_err(),
        "the next SQL caller must not inherit the old permit"
    );
}

#[test]
fn cached_statement_steps_the_current_exact_unit_permit() {
    let fixture = Fixture::new("boundary-step");
    let writer = fixture.writer();
    let connection = fixture.store.connection().unwrap();
    let mut statement = connection
        .prepare_cached("UPDATE credentials SET revoked=1 WHERE id=?1")
        .unwrap();
    assert!(statement.execute([&fixture.credential]).is_err());
    // No SQL/function registration change or statement rebuild: only the actual
    // already-owned writer may make this same statement eligible now.
    connection
        .bind_mutation_authority(
            &fixture.store,
            &writer,
            &[fixture.scope(AuthorityMode::Write)],
        )
        .unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert_eq!(statement.execute([&fixture.credential]).unwrap(), 1);
    connection.execute_batch("ROLLBACK").unwrap();
    drop(statement);
    drop(writer);
    drop(connection);
    assert!(fixture.store.revalidate(&fixture.credential).is_ok());
}

#[test]
fn update_or_replace_cannot_erase_another_identity_with_only_the_source_writer() {
    let fixture = Fixture::new("boundary-replace");
    let other = fixture
        .store
        .create_identity("boundary-replace-other")
        .unwrap()
        .id;
    let other_credential = fixture
        .store
        .issue(&other, Scope::operator(), crate::now() + 600)
        .unwrap()
        .id;
    let writer = fixture.writer();
    {
        let connection = fixture.store.connection_with_authority(&writer).unwrap();
        // Enforcement cannot depend on this connection's DELETE-trigger mode.
        connection
            .execute_batch("PRAGMA recursive_triggers=OFF")
            .unwrap();
        let other_verifier: String = connection
            .query_row(
                "SELECT verifier FROM credentials WHERE id=?1",
                [&other_credential],
                |row| row.get(0),
            )
            .unwrap();
        assert!(connection
            .execute(
                "UPDATE OR REPLACE credentials SET verifier=?1 WHERE id=?2",
                rusqlite::params![other_verifier, fixture.credential]
            )
            .is_err());
        let count: i64 = connection
            .query_row(
                "SELECT count(*) FROM credentials WHERE id IN (?1,?2)",
                rusqlite::params![fixture.credential, other_credential],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }
    drop(writer);
    assert!(fixture.store.revalidate(&fixture.credential).is_ok());
    assert!(fixture.store.revalidate(&other_credential).is_ok());
}

#[test]
fn original_journal_append_fills_metadata_but_cannot_rewrite_a_saved_export_prefix() {
    let fixture = Fixture::new("boundary-journal");
    let cid = "conv_boundary_journal";
    let writer = fixture
        .store
        .acquire_authority(
            &[
                fixture.scope(AuthorityMode::Write),
                AuthorityScope::conversation(cid, AuthorityMode::Write).unwrap(),
            ],
            &crate::AuthorityWait::new(std::time::Duration::from_secs(2)),
        )
        .unwrap();
    {
        let c = fixture.store.connection_with_authority(&writer).unwrap();
        c.execute("INSERT INTO conversations(id) VALUES(?1)", [cid])
            .unwrap();
        c.execute("INSERT INTO participants(conversation_id,identity_id,participant_id,role) VALUES(?1,?2,'part_boundary','participant')", rusqlite::params![cid, fixture.identity]).unwrap();
    }
    drop(writer);
    let first: i64;
    {
        let c = fixture.store.connection().unwrap();
        c.execute("INSERT INTO journal(id,recipient,conversation_id,kind) VALUES('evt_boundary_first',?1,?2,'boundary')", rusqlite::params![fixture.identity, cid]).unwrap();
        let (sequence, participant, epoch, observed): (i64, String, i64, i64) = c.query_row("SELECT sequence,recipient_participant_id,recipient_epoch,observed_at FROM journal WHERE id='evt_boundary_first'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        first = sequence;
        assert_eq!(participant, "part_boundary");
        assert_eq!(epoch, 1);
        assert!(observed > 0);
    }
    let writer = fixture
        .store
        .acquire_authority(
            &[
                fixture.scope(AuthorityMode::Write),
                AuthorityScope::conversation(cid, AuthorityMode::Write).unwrap(),
            ],
            &crate::AuthorityWait::new(std::time::Duration::from_secs(2)),
        )
        .unwrap();
    {
        let c = fixture.store.connection_with_authority(&writer).unwrap();
        c.execute("INSERT INTO context_deliveries(id,conversation_id,provider,watermark,selection_json,state,created_at,updated_at,sequence,owner_id) VALUES('ctx_boundary',?1,'fixture',?2,'{}','pending',100,100,1,?3)", rusqlite::params![cid,first,fixture.identity]).unwrap();
    }
    drop(writer);
    {
        let c = fixture.store.connection().unwrap();
        assert!(c
            .execute(
                "UPDATE journal SET kind='rewritten' WHERE id='evt_boundary_first'",
                []
            )
            .is_err());
        for sequence in [-1_i64, 0, first] {
            assert!(c.execute("INSERT INTO journal(sequence,id,recipient,conversation_id,kind) VALUES(?1,'evt_boundary_bad',?2,?3,'boundary')", rusqlite::params![sequence,fixture.identity,cid]).is_err());
        }
        c.execute("INSERT INTO journal(id,recipient,conversation_id,kind) VALUES('evt_boundary_tail',?1,?2,'boundary')", rusqlite::params![fixture.identity,cid]).unwrap();
        let unchanged: String = c
            .query_row(
                "SELECT kind FROM journal WHERE id='evt_boundary_first'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(unchanged, "boundary");
    }
    let raw = Connection::open(fixture.store.path()).unwrap();
    let error = raw.execute("INSERT INTO journal(id,recipient,conversation_id,kind) VALUES('evt_boundary_raw',?1,?2,'boundary')", rusqlite::params![fixture.identity,cid]).unwrap_err();
    assert!(
        error.to_string().contains(FUNCTION),
        "wrong refusal: {error}"
    );
}

#[test]
fn watcher_cannot_enable_after_a_guard_trigger_or_schema_table_changes() {
    for ddl in [
        "DROP TRIGGER connect_writer_credentials_update",
        "CREATE TABLE unrecognized_runtime_state(id TEXT PRIMARY KEY)",
    ] {
        let fixture = Fixture::new("boundary-schema");
        fixture
            .store
            .connection()
            .unwrap()
            .execute_batch(ddl)
            .unwrap();
        assert!(fixture.store.start_media_authorization_watcher().is_err());
    }
}

#[test]
fn native_flow_phase_and_join_projection_preserve_permissions_but_control_mutations_need_write() {
    let fixture = Fixture::new("boundary-flow");
    let cid = "conv_boundary_flow";
    let sid = "session_boundary_flow";
    let writer = fixture
        .store
        .acquire_authority(
            &[
                fixture.scope(AuthorityMode::Write),
                AuthorityScope::conversation(cid, AuthorityMode::Write).unwrap(),
            ],
            &crate::AuthorityWait::new(std::time::Duration::from_secs(2)),
        )
        .unwrap();
    {
        let c = fixture.store.connection_with_authority(&writer).unwrap();
        c.execute("INSERT INTO conversations(id) VALUES(?1)", [cid])
            .unwrap();
        c.execute("INSERT INTO participants(conversation_id,identity_id,participant_id,role) VALUES(?1,?2,'part_flow','participant')", rusqlite::params![cid, fixture.identity]).unwrap();
        c.execute("INSERT INTO sessions(id,conversation_id,initiator_id,participant_id,state,media,created_at,updated_at) VALUES(?1,?2,?3,?3,'accepted','[\"audio\"]',100,100)", rusqlite::params![sid,cid,fixture.identity]).unwrap();
        c.execute("INSERT INTO session_participants(session_id,conversation_id,tenant_id,participant_id,membership_epoch,role,state) VALUES(?1,?2,'tnt_default','part_flow',1,'participant','present')", rusqlite::params![sid,cid]).unwrap();
    }
    drop(writer);
    let read = fixture
        .store
        .read_authority(
            &fixture.credential,
            Some(cid),
            &crate::AuthorityWait::new(std::time::Duration::from_secs(1)),
        )
        .unwrap();
    {
        let c = fixture.store.connection_with_authority(&read).unwrap();
        // The same statement as verified public flow's Session observation. No
        // media/codec, membership, topology or terminal grant changes here.
        c.execute("UPDATE sessions SET state='active',updated_at=101 WHERE id=?1 AND state IN ('accepted','ready')", [sid]).unwrap();
        // Exercise the actual metadata-only join helper, including its original
        // revision, fact and journal writes. This is not physical IO evidence.
        let before_revision: u64 = c
            .query_row(
                "SELECT state_revision FROM sessions WHERE id=?1",
                [sid],
                |r| r.get(0),
            )
            .unwrap();
        crate::session_participant_lifecycle::observed_join(
            &c,
            &fixture.store,
            cid,
            sid,
            "part_flow",
            1,
            "original_observed_connection",
            chrono::Utc::now().timestamp_millis(),
        )
        .unwrap();
        let (state, revision): (String, u64) = c
            .query_row(
                "SELECT state,state_revision FROM sessions WHERE id=?1",
                [sid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, "active");
        assert_eq!(revision, before_revision + 1);
        for sql in [
            "UPDATE sessions SET state='ended' WHERE id=?1",
            "UPDATE sessions SET state='accepted' WHERE id=?1",
            "UPDATE sessions SET media='[\"video\"]' WHERE id=?1",
            "UPDATE sessions SET topology_revision=topology_revision+1 WHERE id=?1",
        ] {
            assert!(
                c.execute(sql, [sid]).is_err(),
                "control mutation borrowed an observation exception: {sql}"
            );
        }
    }
    drop(read);
}
