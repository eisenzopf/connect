//! Additional real authority/withdrawal controls; not included in OLD4 overlay.
use super::*;

#[test]
fn true_conversation_and_identity_writers_still_fence_original_av_epochs_without_restore() {
    for change in ["conversation", "source-identity", "target-identity"] {
        let setup = setup(true);
        let m = setup.movement;
        if change == "conversation" {
            // Conversation Close also rejects any nonterminal ledger operation,
            // not only live Sessions. Complete this real prepared replacement
            // through its existing controlled capture/publication BEFORE the
            // measured healthy publication and actual CID Writer announcement.
            let captured = activation_barrier::captured(&m);
            m.f.store
                .project_native_replacement_committed(&captured, &MetadataOnlyInstalled)
                .unwrap();
        }
        let admin = m.f.store.revalidate(&m.f.admin).unwrap();
        let source_identity = m.f.store.revalidate(&m.f.keys[1]).unwrap().identity_id;
        let target_identity = m.f.store.revalidate(&m.f.keys[2]).unwrap().identity_id;
        let revoked = if change == "source-identity" {
            m.f.keys[1].clone()
        } else {
            m.f.keys[2].clone()
        };
        let scope = match change {
            "conversation" => {
                crate::AuthorityScope::conversation(&m.f.cid, crate::AuthorityMode::Write).unwrap()
            }
            "source-identity" => {
                crate::AuthorityScope::identity(&source_identity, crate::AuthorityMode::Write)
                    .unwrap()
            }
            _ => crate::AuthorityScope::identity(&target_identity, crate::AuthorityMode::Write)
                .unwrap(),
        };
        let store = Arc::new(m.f.store);
        let snapshot = register_pair(&store, &setup.healthy_target, Some(&setup.healthy_source));
        let mut watcher = store.start_media_authorization_watcher().unwrap();
        let original = admit(&snapshot);
        let stamp = original.snapshot_stamp().unwrap();
        let epoch = original.packet_epoch().unwrap();
        assert!(original.current() && epoch.current() && original.permits_subscriptions());
        let first = store.try_acquire_authority(std::slice::from_ref(&scope));
        let first_code = first.err().map(|e| e.code);
        let fenced_before_shutdown = !epoch.current() && snapshot.admit_original(stamp).is_err();
        drop(original);
        let watcher_result = watcher.shutdown();
        let writer = store
            .acquire_authority(&[scope], &crate::AuthorityWait::new(Duration::from_secs(1)))
            .unwrap();
        drop(writer);
        // Supported ordinary authority mutations, never an invented epoch reset.
        if change == "conversation" {
            // Close requires no live Sessions. End both genuine Sessions through
            // their actual participants before the supported CID close.
            for (actor, sid) in [(0, "session"), (1, "isolation-av")] {
                let topology: u64 = store
                    .connection()
                    .unwrap()
                    .query_row(
                        "SELECT topology_revision FROM sessions WHERE id=?1",
                        [sid],
                        |r| r.get(0),
                    )
                    .unwrap();
                store
                    .execute(
                        &m.f.keys[actor],
                        &Operation::SessionControl {
                            request: Request {
                                operation_id: format!("isolation-end-before-close-{actor}"),
                                conversation_id: m.f.cid.clone(),
                                session_id: sid.into(),
                                by_participant: None,
                                expected_revision: Some(topology),
                                action: Action::End {
                                    reason_code: 200,
                                    reason: "scope-cid-close-control".into(),
                                },
                            },
                        },
                    )
                    .unwrap();
            }
            let outcome = store
                .execute(
                    &m.f.keys[0],
                    &Operation::Close {
                        request_id: "isolation-real-conversation-close".into(),
                        conversation_id: m.f.cid.clone(),
                        options: connect_protocol::conversations::CloseOptions {
                            reason_code: Some(200),
                            reason: Some("scoped-fence-control".into()),
                            ..Default::default()
                        },
                    },
                )
                .unwrap();
            assert!(matches!(outcome,connect_protocol::Outcome::Conversation(view) if view.closed));
        } else {
            store.revoke(&admin, &revoked).unwrap();
        }
        let fresh_refused = store
            .try_native_media_read(&setup.healthy_target, Some(&setup.healthy_source))
            .is_err();
        watcher_result.unwrap();
        assert_eq!(first_code.as_deref(), Some("authority-busy"));
        assert!(
            fenced_before_shutdown
                && fresh_refused
                && !epoch.current()
                && snapshot.admit_original(stamp).is_err(),
            "true authority mutation allowed an old epoch/stamp to revive"
        );
    }
}

#[test]
fn expired_cancelled_and_terminal_claims_cannot_preserve_ghost_joined_presence() {
    for withdrawal in ["cancel", "expire", "source-expiry", "terminal"] {
        let setup = match withdrawal {
            "expire" => setup_options(true, Some(6), false),
            "source-expiry" => setup_options_with_source_ttl(true, None, false, Some(6)),
            _ => setup(true),
        };
        let m = setup.movement;
        let affected = m.sources[0].clone();
        let lease = setup.original_lease.unwrap();
        let _capture = activation_barrier::captured(&m);
        if withdrawal == "cancel" {
            super::super::custody::cancel(&m.f, &m.key, &m.command);
        } else if matches!(withdrawal, "expire" | "source-expiry") {
            let until: i64 = {
                let c = m.f.store.connection().unwrap();
                if withdrawal == "expire" {
                    c.query_row(
                        "SELECT expires_at FROM credentials WHERE id=?1",
                        [&m.key],
                        |r| r.get(0),
                    )
                    .unwrap()
                } else {
                    c.query_row(
                        "SELECT expires_at FROM peer_signing_keys WHERE key_id='native-key-0'",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap()
                }
            };
            assert!(
                crate::now() < until,
                "actual original bounded credential/key expired before qualified capture"
            );
            let watchdog = Instant::now() + Duration::from_secs(8);
            while crate::now() < until && Instant::now() < watchdog {
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(
                crate::now() >= until,
                "actual original bound did not naturally expire"
            );
        } else {
            // Only this explicit terminal fact uses the existing complete
            // coordinated Writer; no persisted deadline or presence is forged.
            let writer = m.f.controlled_conversation_writer(0);
            m.f.store
                .connection_with_authority(&writer)
                .unwrap()
                .execute(
                    "UPDATE operations SET state='failed' WHERE operation_id=?1",
                    [&m.command.operation_id],
                )
                .unwrap();
        }
        let sid = m.command.session_id.clone();
        let pid = affected.participant_id().to_owned();
        let store = Arc::new(m.f.store);
        let before = lifecycle(&store, &sid);
        let snapshot = register_pair(&store, &setup.healthy_target, Some(&setup.healthy_source));
        let mut watcher = store.start_media_authorization_watcher().unwrap();
        let original = admit(&snapshot);
        let stamp = original.snapshot_stamp().unwrap();
        let epoch = original.packet_epoch().unwrap();
        assert!(original.current() && epoch.current());
        let first = store.settle_native_offer_cleanup_with_wait(
            &lease,
            &Absent {
                origin: &affected,
                probe: None,
            },
            &crate::AuthorityWait::new(Duration::from_millis(20)),
        );
        let broad_fallback_fenced = !epoch.current() && snapshot.admit_original(stamp).is_err();
        let first_code = first.err().map(|e| e.code);
        drop(original);
        let watcher_result = watcher.shutdown();
        Call::Original(lease)
            .settle_after_draining(&store, &affected)
            .unwrap();
        let after = lifecycle(&store, &sid);
        let c = rusqlite::Connection::open_with_flags(
            store.path(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let left:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM session_connected_presence WHERE session_id=?1 AND participant_id=?2 AND state='left')",params![sid,pid],|r|r.get(0)).unwrap();
        drop(c);
        let old_media_refused = store.try_native_media_read(&affected, None).is_err();
        watcher_result.unwrap();
        assert_eq!(first_code.as_deref(), Some("authority-busy"));
        assert!(broad_fallback_fenced&&left&&before!=after&&old_media_refused&&!epoch.current(),
            "withdrawn handoff retained ghost presence, bypassed broad lifecycle Writer, or restored original epoch");
    }
}

#[test]
fn foreign_store_cannot_use_original_handoff_or_media_read_as_authority() {
    let setup = setup(true);
    let m = setup.movement;
    let captured = activation_barrier::captured(&m);
    let foreign = Store::open_administration(m.f.store.path(), m.f.store.limits().clone()).unwrap();
    let read =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    assert!(foreign
        .try_native_replacement_commit_scene_write(&m.origin, &m.sources)
        .is_err());
    assert!(foreign
        .try_begin_native_replacement_commit(m.origin.clone(), &read)
        .is_err());
    assert!(foreign
        .project_native_replacement_committed(&captured, &MetadataOnlyInstalled)
        .is_err());
    assert!(foreign
        .try_native_media_read(&setup.healthy_target, Some(&setup.healthy_source))
        .is_err());
    assert!(foreign
        .settle_native_offer_cleanup_with_wait(
            setup.original_lease.as_ref().unwrap(),
            &Absent {
                origin: &m.sources[0],
                probe: None
            },
            &crate::AuthorityWait::new(Duration::from_millis(20))
        )
        .is_err());
    drop(read);
    assert_eq!(
        Store::inspect_snapshot(m.f.store.path()).unwrap(),
        before,
        "foreign owner changed original durable facts"
    );
}

fn presence(c: &rusqlite::Connection, sid: &str, pid: &str) -> bool {
    c.query_row("SELECT EXISTS(SELECT 1 FROM session_connected_presence WHERE session_id=?1 AND participant_id=?2 AND state='joined')",params![sid,pid],|r|r.get(0)).unwrap()
}
fn reconnect(c: &rusqlite::Connection, sid: &str) -> (Option<i64>, Option<i64>) {
    c.query_row("SELECT lost_at_millis,deadline_millis FROM session_reconnect_boundaries WHERE session_id=?1",[sid],|r|Ok((r.get(0)?,r.get(1)?))).unwrap()
}
fn page_contains(store: &Store, cid: &str, sid: &str) -> bool {
    let mut cursor = String::new();
    for _ in 0..8 {
        let (next, rows) = store.session_reconnect_page(&cursor).unwrap();
        if rows.iter().any(|(c, s)| c == cid && s == sid) {
            return true;
        }
        if next.is_empty() {
            return false;
        }
        assert!(next > cursor, "bounded scanner cursor did not advance");
        cursor = next;
    }
    panic!("bounded two-Session fixture exceeded 128 scanner rows")
}
#[test]
fn real_last_owner_pending_presence_expires_or_cancels_without_renewing_session_grace() {
    for others_live in [false, true] {
        for withdraw in ["expiry", "cancel"] {
            // Real principal TTL is configured BEFORE original admission.
            let setup = setup_options(true, Some(6), false);
            let m = setup.movement;
            let _capture = activation_barrier::captured(&m);
            let sid = m.command.session_id.clone();
            let pid = m.sources[0].participant_id().to_owned();
            let epoch = m.sources[0].membership_epoch();
            m.f.store
                .try_settle_native_offer_cleanup(
                    setup.original_lease.as_ref().unwrap(),
                    &Absent {
                        origin: &m.sources[0],
                        probe: None,
                    },
                )
                .unwrap();
            if !others_live {
                for (source, lease) in m.sources[1..].iter().zip(setup.other_primary_leases.iter())
                {
                    m.f.store
                        .try_settle_native_offer_cleanup(
                            lease,
                            &Absent {
                                origin: source,
                                probe: None,
                            },
                        )
                        .unwrap();
                }
            }
            let (boundary, cutoff) = {
                let c = m.f.store.connection().unwrap();
                assert_eq!(
                    crate::session_reconnect::connected_now(&c, &m.f.store, &sid).unwrap(),
                    others_live,
                    "real connected owner state differs from selected branch"
                );
                assert!(
                    presence(&c, &sid, &pid),
                    "real cleanup already withdrew the supposed valid pending appearance"
                );
                assert!(
                    crate::transitions::native_replacement_preserves_joined_presence(
                        &c, &m.f.store, &sid, &pid, epoch
                    )
                    .unwrap(),
                    "fixture lacks the real bounded pending claim"
                );
                let cutoff:i64=c.query_row("SELECT min(o.expires_at,n.expires_at,key.expires_at) FROM operations o JOIN native_replacements n ON n.operation_record_id=o.record_id JOIN credentials key ON key.id=n.credential_id WHERE o.operation_id=?1",[&m.command.operation_id],|r|r.get(0)).unwrap();
                assert!(
                    crate::now() < cutoff,
                    "claim expired before actual qualified pending checkpoint"
                );
                (reconnect(&c, &sid), cutoff)
            };
            if !others_live {
                assert!(
                    boundary.1.is_some_and(
                        |deadline| deadline > chrono::Utc::now().timestamp_millis() + 7000
                    ),
                    "original Session grace cannot cover this bounded fixture"
                );
            }
            assert!(
                !page_contains(&m.f.store, &m.f.cid, &sid),
                "valid bounded appearance unnecessarily entered the actual sweep"
            );
            if withdraw == "cancel" {
                super::super::custody::cancel(&m.f, &m.key, &m.command)
            } else {
                let watchdog = Instant::now() + Duration::from_secs(8);
                while crate::now() < cutoff && Instant::now() < watchdog {
                    std::thread::sleep(Duration::from_millis(5));
                }
                assert!(
                    crate::now() >= cutoff,
                    "actual original claim failed to expire within bound"
                );
            }
            let lingering_joined = {
                let c = m.f.store.connection().unwrap();
                assert!(
                    !crate::transitions::native_replacement_preserves_joined_presence(
                        &c, &m.f.store, &sid, &pid, epoch
                    )
                    .unwrap()
                );
                let joined = presence(&c, &sid, &pid);
                if withdraw == "expiry" {
                    assert!(
                        joined,
                        "clock-only expiry must exercise actual sweep eligibility"
                    );
                }
                // Actual cancellation may safely withdraw in its own TX.
                joined
            };
            if lingering_joined {
                assert!(page_contains(&m.f.store,&m.f.cid,&sid),"actual page omitted withdrawn joined appearance before Session grace or while other peers remain live");
            }
            assert!(
                !m.f.store
                    .reconcile_session_reconnect(&m.f.cid, &sid)
                    .unwrap(),
                "appearance withdrawal prematurely ended Session"
            );
            {
                let c = m.f.store.connection().unwrap();
                assert!(!presence(&c, &sid, &pid));
                assert_eq!(
                    crate::session_reconnect::connected_now(&c, &m.f.store, &sid).unwrap(),
                    others_live
                );
                assert_eq!(
                    reconnect(&c, &sid),
                    boundary,
                    "appearance sweep renewed the original reconnect budget"
                );
                assert_eq!(
                    c.query_row("SELECT state FROM sessions WHERE id=?1", [&sid], |r| r
                        .get::<_, String>(
                        0
                    ))
                    .unwrap(),
                    "active"
                );
            }
            assert!(!m
                .f
                .store
                .reconcile_session_reconnect(&m.f.cid, &sid)
                .unwrap());
            assert_eq!(reconnect(&m.f.store.connection().unwrap(), &sid), boundary);
        }
    }
}

#[test]
fn bounded_replacement_claim_never_creates_an_unobserved_joined_appearance() {
    // ONLY the negative omits source0's genuine flow observation. Other actors
    // still establish a real started/active Session through their real flows.
    let setup = setup_options(true, None, true);
    let m = setup.movement;
    let sid = m.command.session_id.clone();
    let pid = m.sources[0].participant_id().to_owned();
    let epoch = m.sources[0].membership_epoch();
    let _capture = activation_barrier::captured(&m);
    {
        let c = m.f.store.connection().unwrap();
        assert!(!presence(&c, &sid, &pid));
        assert!(
            !crate::transitions::native_replacement_preserves_joined_presence(
                &c, &m.f.store, &sid, &pid, epoch
            )
            .unwrap()
        );
    }
    m.f.store
        .try_settle_native_offer_cleanup(
            setup.original_lease.as_ref().unwrap(),
            &Absent {
                origin: &m.sources[0],
                probe: None,
            },
        )
        .unwrap();
    assert!(!m
        .f
        .store
        .reconcile_session_reconnect(&m.f.cid, &sid)
        .unwrap());
    assert!(!presence(&m.f.store.connection().unwrap(), &sid, &pid));
}
