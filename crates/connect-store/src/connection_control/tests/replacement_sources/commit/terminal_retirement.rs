//! Exact historical replacement cleanup after public Session generation fencing.
use super::*;
mod fair_admission;

struct Gone(String, u64);
impl crate::NativeOfferRuntimeObservation for Gone {
    fn observe(&self) -> crate::Result<crate::NativeOfferObservation> {
        Ok(crate::NativeOfferObservation::Retired {
            runtime_connection_id: self.0.clone(),
            generation: self.1,
        })
    }
}

fn installed(
    m: &Movement,
) -> (
    crate::NativeReplacementCommitted,
    Arc<crate::NativeMediaOrigin>,
) {
    let capture = super::activation_barrier::captured(m);
    let origin =
        m.f.store
            .project_native_replacement_committed(&capture, &MetadataOnlyInstalled)
            .unwrap();
    (capture, origin)
}

fn depart(m: &Movement, leave: bool) {
    let topology_revision = crate::session_control::view(
        &m.f.store.connection().unwrap(),
        &m.f.cid,
        &m.command.session_id,
    )
    .unwrap()
    .topology_revision;
    m.f.store
        .execute(
            &m.f.keys[0],
            &Operation::SessionControl {
                request: Request {
                    operation_id: "retire-established-replacement-session".into(),
                    conversation_id: m.f.cid.clone(),
                    session_id: m.command.session_id.clone(),
                    by_participant: None,
                    expected_revision: Some(topology_revision),
                    action: if leave {
                        Action::Leave {
                            participant_id: m.f.pids[0].clone(),
                        }
                    } else {
                        Action::End {
                            reason_code: 1000,
                            reason: "controlled original replacement cleanup boundary".into(),
                        }
                    },
                },
            },
        )
        .unwrap();
}

fn state(m: &Movement, origin: &crate::NativeMediaOrigin) -> (String, u64, bool) {
    m.f.store
        .connection()
        .unwrap()
        .query_row(
            "SELECT c.state,c.generation,l.retired FROM connections c
         JOIN native_connection_lifetimes l ON l.connection_id=c.id
         AND l.operation_record_id=?2
         WHERE c.id=?1",
            params![origin.connection_id(), origin.offer_record_id()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
}

fn settle_after_transient_contention(
    m: &Movement,
    origin: &crate::NativeMediaOrigin,
    absent: &Gone,
) -> crate::Result<()> {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match m.f.store.settle_native_replacement_cleanup(origin, absent) {
            Err(error)
                if matches!(
                    error.code.as_str(),
                    "authority-busy" | "storage-unavailable"
                ) && std::time::Instant::now() < until =>
            {
                std::thread::sleep(std::time::Duration::from_micros(100));
            }
            result => return result,
        }
    }
}

#[test]
fn replacement_cleanup_defers_without_blocking_other_current_packet_reads() {
    let m = fixture(true);
    let (_, origin) = installed(&m);
    let absent = Gone(origin.runtime_connection_id().into(), origin.generation());
    assert_passive_read_deferral(&m, || {
        m.f.store
            .settle_native_replacement_cleanup(&origin, &absent)
    });
    m.f.store
        .settle_native_replacement_cleanup(&origin, &absent)
        .unwrap();
    assert_eq!(
        state(&m, &origin),
        ("retired".into(), origin.generation() + 1, true)
    );
    let settled = Store::inspect_snapshot(m.f.store.path()).unwrap();
    m.f.store
        .settle_native_replacement_cleanup(&origin, &absent)
        .unwrap();
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), settled);
}

#[test]
fn replacement_cleanup_releases_writer_when_sql_or_transaction_is_busy() {
    let m = fixture(true);
    let (_, origin) = installed(&m);
    let absent = Gone(origin.runtime_connection_id().into(), origin.generation());
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let foreign = rusqlite::Connection::open(m.f.store.path()).unwrap();
    foreign.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = std::time::Instant::now();
    let refused =
        m.f.store
            .settle_native_replacement_cleanup(&origin, &absent);
    let elapsed = started.elapsed();
    let packet = m.f.store.try_native_media_read(&m.sources[1], None);
    let packet_current = packet.as_ref().is_ok_and(|read| read.current());
    drop(packet);
    foreign.execute_batch("ROLLBACK").unwrap();
    let timeout: u64 =
        m.f.store
            .connection()
            .unwrap()
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
    // Preserve both actual SQL observations even when the old primary-mutex
    // behavior fails below. All original scopes and foreign SQL are released.
    let refusal_code = refused
        .as_ref()
        .err()
        .map(|error| error.code.as_str())
        .unwrap_or("ok");
    eprintln!("passive retirement foreign SQL: code={refusal_code} elapsed_us={} packet_current={packet_current} restored_timeout_ms={timeout}", elapsed.as_micros());
    assert_passive_sql_reacquire_defers(&m, || {
        m.f.store
            .settle_native_replacement_cleanup(&origin, &absent)
    });
    assert_eq!(refused.unwrap_err().code, "storage-unavailable");
    assert!(
        elapsed < std::time::Duration::from_millis(100),
        "passive retirement waited on a foreign SQLite writer"
    );
    assert!(
        packet_current,
        "busy transaction retained an authority writer"
    );
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    assert_eq!(
        timeout, 2000,
        "immediate SQL guard must restore its original timeout"
    );
    m.f.store
        .settle_native_replacement_cleanup(&origin, &absent)
        .unwrap();
    assert_eq!(
        state(&m, &origin),
        ("retired".into(), origin.generation() + 1, true)
    );
}

#[test]
fn replacement_cleanup_finishes_original_fenced_session_end_and_leave_after_target_revoke() {
    for leave in [false, true] {
        let m = fixture(true);
        let (capture, origin) = installed(&m);
        let original =
            m.f.store
                .record_native_replacement_committed(&capture)
                .unwrap();
        assert_eq!(original.events_omitted, 0);
        depart(&m, leave);
        assert_eq!(
            state(&m, &origin),
            ("retiring".into(), origin.generation() + 1, false),
        );
        let admin = m.f.store.revalidate(&m.f.admin).unwrap();
        m.f.store
            .revoke(&admin, &m.target_request.credentials[0])
            .unwrap();
        assert!(m.f.store.try_native_media_read(&origin, None).is_err());
        let unauthorized = m.f.store.execute(
            &m.target_request.credentials[0],
            &Operation::SessionControl {
                request: Request {
                    operation_id: "revoked-target-cannot-start-another-mutation".into(),
                    conversation_id: m.f.cid.clone(),
                    session_id: m.command.session_id.clone(),
                    by_participant: None,
                    expected_revision: None,
                    action: Action::End {
                        reason_code: 1000,
                        reason: "must remain refused".into(),
                    },
                },
            },
        );
        assert_eq!(unauthorized.unwrap_err().code, "unauthorized");

        let absent = Gone(origin.runtime_connection_id().into(), origin.generation());
        std::thread::scope(|threads| {
            let first = threads.spawn(|| settle_after_transient_contention(&m, &origin, &absent));
            let second = threads.spawn(|| settle_after_transient_contention(&m, &origin, &absent));
            first.join().unwrap().unwrap();
            second.join().unwrap().unwrap();
        });
        assert_eq!(
            state(&m, &origin),
            ("retired".into(), origin.generation() + 2, true),
        );
        let mut retired =
            m.f.store
                .record_native_replacement_committed(&capture)
                .unwrap();
        let (previous_events, cleanup_events): (Vec<_>, Vec<_>) = retired
            .causal_event_ids
            .iter()
            .partition(|id| original.causal_event_ids.contains(id));
        assert_eq!(
            previous_events,
            original.causal_event_ids.iter().collect::<Vec<_>>(),
            "retirement must preserve every original causal ID in its original order",
        );
        assert_eq!(
            cleanup_events.len(),
            1,
            "one new owner-visible cleanup fact"
        );
        let (kind, root, conversation, revision, body, provenance): (
            String,
            String,
            String,
            u64,
            String,
            String,
        ) =
            m.f.store
                .connection()
                .unwrap()
                .query_row(
                    "SELECT j.kind,j.operation_record_id,j.conversation_id,
                        j.resource_revision,j.data_json,j.source_json
                 FROM journal j JOIN operations o
                 ON o.record_id=?2 AND o.owner_id=j.recipient
                 WHERE j.id=?1",
                    params![cleanup_events[0], origin.offer_record_id()],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                        ))
                    },
                )
                .unwrap();
        assert_eq!(kind, "connection_retired");
        assert_eq!(root, original.record_id);
        assert_eq!(conversation, origin.conversation_id());
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["operation_id"], original.operation_id);
        assert_eq!(body["source_kind"], "session");
        assert_eq!(body["source_id"], origin.session_id());
        assert_eq!(body["session_id"], origin.session_id());
        assert_eq!(body["connection_id"], origin.connection_id());
        assert_eq!(body["participant_id"], origin.participant_id());
        assert_eq!(body["connection"]["connection_id"], origin.connection_id());
        assert_eq!(body["connection"]["session_id"], origin.session_id());
        assert_eq!(
            body["connection"]["membership_epoch"],
            origin.membership_epoch()
        );
        assert_eq!(body["connection"]["generation"], origin.generation() + 2);
        assert_eq!(body["connection"]["state"], "retired");
        assert_eq!(body["connection"]["revision"], revision);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&provenance).unwrap(),
            json!({"kind":"connect-native-runtime","correlation_id":origin.connection_id()}),
        );
        // The causal page includes the later cleanup fact. Every field of the
        // original completion itself must still compare exactly.
        retired
            .causal_event_ids
            .clone_from(&original.causal_event_ids);
        assert_eq!(retired, original);
        let settled = Store::inspect_snapshot(m.f.store.path()).unwrap();
        m.f.store
            .settle_native_replacement_cleanup(&origin, &absent)
            .unwrap();
        assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), settled);
    }
}

#[test]
fn replacement_cleanup_refuses_wrong_absence_and_unexplained_local_fences_without_changes() {
    for terminal_session in [false, true] {
        let m = fixture(true);
        let (_, origin) = installed(&m);
        if terminal_session {
            depart(&m, false);
        }
        let absent = Gone(origin.runtime_connection_id().into(), origin.generation());
        for wrong in [
            Gone("foreign-physical-runtime".into(), origin.generation()),
            Gone(
                origin.runtime_connection_id().into(),
                origin.generation() + 1,
            ),
        ] {
            let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
            assert_eq!(
                m.f.store
                    .settle_native_replacement_cleanup(&origin, &wrong)
                    .unwrap_err()
                    .code,
                "forbidden",
            );
            assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
        }

        // Controlled invalid states test the refusal branch. The success case
        // above reaches the fenced state through the public Session command.
        let writer = m.f.controlled_conversation_writer(0);
        let sql = m.f.store.connection_with_authority(&writer).unwrap();
        sql.execute(
            "UPDATE connections SET state='retiring',generation=?2,
             observed_runtime_id=NULL,observed_at=NULL,readiness_json=NULL WHERE id=?1",
            params![
                origin.connection_id(),
                origin.generation() + if terminal_session { 2 } else { 1 },
            ],
        )
        .unwrap();
        drop(sql);
        drop(writer);
        let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
        assert_eq!(
            m.f.store
                .settle_native_replacement_cleanup(&origin, &absent)
                .unwrap_err()
                .code,
            "forbidden",
        );
        assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
        assert!(!state(&m, &origin).2);
    }
}
