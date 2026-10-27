//! Metadata completion must not exclude current packet readers while waiting on SQL.
use super::*;
use crate::{AuthorityMode, AuthorityScope, NativeConsultationApplied, NativeMediaOrigin};
use connect_protocol::transitions::OperationState;
use std::{sync::Arc, time::Duration};

struct Pending {
    f: Arc<Fixture>,
    primary: Arc<NativeMediaOrigin>,
    candidate: Arc<NativeMediaOrigin>,
    remote: Arc<NativeMediaOrigin>,
    proof: Arc<NativeConsultationApplied>,
    record: String,
}
impl Pending {
    fn new() -> Self {
        let (f, sources, candidate, origin, _, proof) =
            consultation_audiences::established_fixture();
        f.store.record_native_consultation_decoded(&proof).unwrap();
        Self {
            f: Arc::new(f),
            primary: sources[0].clone(),
            remote: sources[1].clone(),
            candidate,
            proof: Arc::new(proof),
            record: origin.record_id().into(),
        }
    }
    fn packet_authority(&self) -> (bool, bool) {
        let current = |target, source| {
            self.f
                .store
                .try_native_media_read(target, Some(source))
                .is_ok_and(|read| read.current())
        };
        (
            current(&self.primary, &self.candidate) && current(&self.candidate, &self.primary),
            !current(&self.remote, &self.candidate),
        )
    }
    fn finish_once(&self) {
        let done = self
            .f
            .store
            .complete_native_consultation(&self.proof)
            .unwrap()
            .unwrap();
        assert_eq!(done.state, OperationState::Completed);
        assert_eq!(done.record_id, self.record);
        assert_eq!(done.result.as_ref().unwrap()["session_revision"], 4);
        let replay = self
            .f
            .store
            .complete_native_consultation(&self.proof)
            .unwrap()
            .unwrap();
        assert_eq!(replay, done);
        let c = self.f.store.connection().unwrap();
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM native_private_lifetimes WHERE operation_record_id=?1",
                [&self.record],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM native_consultation_attempts WHERE operation_record_id=?1",
                [&self.record],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM operation_effects WHERE operation_record_id=?1 AND state='confirmed'", [&self.record], |r|r.get::<_, u64>(0)).unwrap(),
            1
        );
    }
}

#[test]
fn completion_sql_mutex_contention_releases_the_writer_for_current_private_packet_readers() {
    let pending = Pending::new();
    assert_eq!(pending.packet_authority(), (true, true));
    std::thread::scope(|scope| {
        // Hold only the Conversation part of a packet interval so the writer
        // reaches this exact queued scope after acquiring earlier Identity scopes.
        let read = pending
            .f
            .store
            .try_acquire_authority(&[AuthorityScope::conversation(
                &pending.f.cid,
                AuthorityMode::Read,
            )
            .unwrap()])
            .unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let pending = &pending;
        let worker = scope.spawn(move || {
            sender
                .send(pending.f.store.complete_native_consultation(&pending.proof))
                .unwrap();
        });
        // Observe the actual queued Conversation writer, then occupy SQL before
        // releasing the preceding Conversation reader. No production pause hook.
        crate::authority_fences::test_wait_for_writer(
            &pending.f.store,
            &AuthorityScope::conversation(&pending.f.cid, AuthorityMode::Write).unwrap(),
        );
        let sql = pending.f.store.connection().unwrap();
        drop(read);
        let early = receiver.recv_timeout(Duration::from_millis(200));
        let returned_while_sql_held = early.is_ok();
        let packets = pending.packet_authority();
        let state: String = sql
            .query_row(
                "SELECT state FROM operations WHERE record_id=?1",
                [&pending.record],
                |r| r.get(0),
            )
            .unwrap();
        drop(sql);
        let outcome =
            early.unwrap_or_else(|_| receiver.recv_timeout(Duration::from_secs(3)).unwrap());
        worker.join().unwrap();
        eprintln!("controlled consultation SQL mutex: early={returned_while_sql_held} packet_current={} outside_refused={}", packets.0, packets.1);
        assert!(
            returned_while_sql_held,
            "completion queued on SQL while excluding current packet readers"
        );
        assert_eq!(outcome.unwrap_err().code, "authority-busy");
        assert_eq!(packets, (true, true));
        assert_eq!(state, "preparing");
    });
    pending.finish_once();
}

#[test]
fn completion_sqlite_writer_contention_preserves_private_authority_and_original_completion() {
    let pending = Pending::new();
    assert_eq!(pending.packet_authority(), (true, true));
    std::thread::scope(|scope| {
        let external = rusqlite::Connection::open(pending.f.store.path()).unwrap();
        external.execute_batch("BEGIN IMMEDIATE").unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let pending = &pending;
        let worker = scope.spawn(move || {
            sender
                .send(pending.f.store.complete_native_consultation(&pending.proof))
                .unwrap();
        });
        let early = receiver.recv_timeout(Duration::from_millis(200));
        let returned_while_sql_held = early.is_ok();
        let packets = pending.packet_authority();
        external.execute_batch("ROLLBACK").unwrap();
        let outcome =
            early.unwrap_or_else(|_| receiver.recv_timeout(Duration::from_secs(3)).unwrap());
        worker.join().unwrap();
        eprintln!("controlled consultation SQLite writer: early={returned_while_sql_held} packet_current={} outside_refused={}", packets.0, packets.1);
        assert!(
            returned_while_sql_held,
            "completion waited on SQLite while excluding current packet readers"
        );
        assert_eq!(outcome.unwrap_err().code, "storage-unavailable");
        assert_eq!(packets, (true, true));
        let c = pending.f.store.connection().unwrap();
        assert_eq!(
            c.query_row(
                "SELECT state FROM operations WHERE record_id=?1",
                [&pending.record],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "preparing"
        );
    });
    pending.finish_once();
}
