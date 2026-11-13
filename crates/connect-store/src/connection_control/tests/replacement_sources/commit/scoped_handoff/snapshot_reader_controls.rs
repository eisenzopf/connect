//! Genuine opaque offer retirement while an authorized, disjoint public
//! snapshot transaction remains live. No fake media grant or external Writer.
use super::*;

#[test]
fn snapshot_read_transaction_does_not_reserve_cleanup_writer_but_affected_reader_still_fences() {
    let (setup, call, affected) = prepare(Kind::OriginalCleanup);
    let m = &setup.movement;
    let Call::Original(lease) = call else {
        panic!()
    };
    let connect_protocol::Outcome::Conversation(conversation) =
        m.f.store
            .execute(
                &m.f.keys[0],
                &Operation::Create {
                    request_id: "snapshot-disjoint-conversation".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![]),
                },
            )
            .unwrap()
    else {
        panic!()
    };
    let query = connect_protocol::events::SnapshotQuery::default();
    let request = crate::PeerAuthorityRequest {
        credentials: vec![m.f.keys[0].clone()],
        device: crate::PeerDeviceClaim {
            id: "device-0".into(),
            kind: "desktop".into(),
            platform: "isolated-store-fixture".into(),
        },
        key_id: "native-key-0".into(),
        algorithm: "EdDSA".into(),
        command: crate::PeerCommand::Snapshot {
            conversation_id: conversation.id.clone(),
            query: query.clone(),
        },
    };
    let proof = m.f.admit(&request);
    let baseline =
        m.f.store
            .conversation_snapshot_with_peer(
                &m.f.keys[0],
                &conversation.id,
                query.clone(),
                Some(&proof),
            )
            .unwrap();
    assert!(!baseline.items.is_empty());
    assert_ne!(conversation.id, affected.conversation_id());
    for scope in [
        crate::AuthorityScope::connection(affected.connection_id(), crate::AuthorityMode::Read)
            .unwrap(),
        crate::AuthorityScope::record(lease.record_id(), crate::AuthorityMode::Read).unwrap(),
    ] {
        assert!(
            !m.f.store
                .authority_covers(Store::peer_scopes(&proof), &[scope]),
            "snapshot peer must not itself retain the affected cleanup resource"
        );
    }

    let absence = Absent {
        origin: &affected,
        probe: None,
    };
    // Actual affected media reader must still exclude retirement, irrespective
    // of which SQL connection projects unrelated authorized snapshots.
    let affected_read = m.f.store.try_native_media_read(&affected, None).unwrap();
    let denied = m.f.store.try_settle_native_offer_cleanup(&lease, &absence);
    drop(affected_read);
    assert!(matches!(&denied, Err(error) if error.code == "authority-busy"));
    let (entered, observed) = mpsc::sync_channel(1);
    let (release, resumed) = mpsc::sync_channel(1);
    let (entered_actual_snapshot, retired, snapshot) = std::thread::scope(|workers| {
        let attempt = workers.spawn(|| {
            crate::events::snapshot_test_probe::with_checkpoint(
                move || {
                    let _ = entered.send(());
                    let _ = resumed.recv_timeout(Duration::from_secs(3));
                },
                || {
                    m.f.store.conversation_snapshot_with_peer(
                        &m.f.keys[0],
                        &conversation.id,
                        query,
                        Some(&proof),
                    )
                },
            )
        });
        let entered_actual_snapshot = observed.recv_timeout(Duration::from_secs(1)).is_ok();
        let retired = if entered_actual_snapshot {
            m.f.store.settle_native_offer_cleanup_with_wait(
                &lease,
                &absence,
                &crate::AuthorityWait::new(Duration::from_secs(1)),
            )
        } else {
            Err(Error::new(
                "counter-unqualified",
                "snapshot checkpoint not reached",
            ))
        };
        // Original SQL reader and snapshot worker always settle before oracle.
        let _ = release.send(());
        (entered_actual_snapshot, retired, attempt.join().unwrap())
    });
    let snapshot = snapshot.unwrap();
    assert!(entered_actual_snapshot);
    assert_eq!(snapshot.items, baseline.items);
    assert!(proof.current());
    // Prove actual fixture/absence viability even on the old condition. This
    // uses the SAME opaque retirement lease after the original reader joined.
    // The saved first result remains the intended continuity oracle below.
    m.f.store
        .settle_native_offer_cleanup_with_wait(
            &lease,
            &absence,
            &crate::AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    let current: (String, u64) =
        m.f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT state,generation FROM connections WHERE id=?1",
                [affected.connection_id()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
    assert_eq!(current.0, "retired");
    assert!(current.1 > affected.generation());
    assert!(
        retired.is_ok(),
        "snapshot SELECT transaction reserved the cleanup Writer: {retired:?}"
    );
}
