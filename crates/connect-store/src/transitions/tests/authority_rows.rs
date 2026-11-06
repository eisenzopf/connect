//! Actual current SQL/authorization; no media IO or performance pass is claimed.
use super::*;

fn equivalent(f: &Fixture, record: &str) {
    let c = f.store.connection().unwrap();
    let actor = actor_from_connection(&c, &f.keys[0]).unwrap();
    let public = rows::by_record(&c, record).unwrap();
    let current = rows::for_authority(&c, record).unwrap();
    assert_eq!(current.digest(), public.digest);
    assert_eq!(current.request().unwrap(), rows::request(&public).unwrap());
    assert_eq!(current.state(), public.record.state);
    assert_eq!(current.worker(), public.worker.as_deref());
    assert_eq!(current.effects(), public.record.effects.as_slice());
    assert_eq!(current.is_ledger(), public.ledger);
    assert_eq!(current.is_native_replacement(), public.native_replacement);
    assert_eq!(current.tombstoned(), public.tombstoned);
    assert_eq!(current.cancel_requested(), public.record.cancel_requested);
    assert_eq!(current.expires_at(), public.record.expires_at);
    assert_eq!(
        current.require_read(&c, &actor).map_err(|e| e.code),
        authority::read(&c, &actor, &public).map_err(|e| e.code)
    );
    assert_eq!(
        current
            .require_execution(&c, &actor, &f.store)
            .map_err(|e| e.code),
        authority::execution(&c, &actor, &public, &f.store).map_err(|e| e.code)
    );
    // Loading domain facts cannot alter the fully rendered public response.
    assert_eq!(rows::by_record(&c, record).unwrap().record, public.record);
}

#[test]
fn authority_rows_preserve_effects_uncertainty_and_full_public_status() {
    let f = Fixture::new();
    let accepted = f.admit("authority-row-history");
    assert!(!accepted.causal_event_ids.is_empty());
    equivalent(&f, &accepted.record_id);
    let lease = f.claim("authority-row-history");
    let effect = f.effect(&lease, "original-effect");
    equivalent(&f, &accepted.record_id);
    let observed = f.observe(
        "authority-row-history",
        &effect,
        "original-uncertain",
        EffectState::Unknown,
    );
    assert!(observed
        .effects
        .iter()
        .any(|e| e.state == EffectState::Unknown));
    equivalent(&f, &accepted.record_id);
    let c = f.store.connection().unwrap();
    let current = rows::for_authority(&c, &accepted.record_id).unwrap();
    assert!(current
        .effects()
        .iter()
        .any(|e| e.state == EffectState::Unknown));
    assert_eq!(current.effects().len(), observed.effects.len());
}

#[test]
fn authority_rows_do_not_render_the_public_journal_page() {
    let f = Fixture::new();
    let accepted = f.admit("authority-row-display-independence");
    let c = f.store.connection().unwrap();
    let actor = actor_from_connection(&c, &f.keys[0]).unwrap();
    let before = rows::by_record(&c, &accepted.record_id).unwrap().record;
    assert!(!before.causal_event_ids.is_empty());
    // Poison only this connection's display view. All real identity, intent,
    // member, endpoint, scope, expiry and effect tables remain available.
    c.execute_batch("CREATE TEMP VIEW visible_journal AS SELECT * FROM missing_status_page_source")
        .unwrap();
    let current = rows::for_authority(&c, &accepted.record_id).unwrap();
    current.require_read(&c, &actor).unwrap();
    current.require_execution(&c, &actor, &f.store).unwrap();
    assert!(
        rows::by_record(&c, &accepted.record_id).is_err(),
        "public display lookup must actually touch its original journal view"
    );
    c.execute_batch("DROP VIEW temp.visible_journal").unwrap();
    assert_eq!(
        rows::by_record(&c, &accepted.record_id).unwrap().record,
        before
    );
}

#[test]
fn authority_rows_still_refuse_current_scope_and_connection_epoch_changes() {
    for change in ["permission", "epoch", "cancel", "expiry"] {
        let f = Fixture::new();
        let accepted = f.admit("authority-row-current-loss");
        {
            let c = f.store.connection().unwrap();
            let actor = actor_from_connection(&c, &f.keys[0]).unwrap();
            rows::for_authority(&c, &accepted.record_id)
                .unwrap()
                .require_execution(&c, &actor, &f.store)
                .unwrap();
        }
        let c = f.store.connection().unwrap();
        match change {
            "permission" => {
                c.execute(
                    "UPDATE credentials SET operations='[\"operation.get\"]' WHERE id=?1",
                    [&f.keys[0]],
                )
                .unwrap();
            }
            "epoch" => {
                c.execute(
                    "UPDATE connections SET generation=generation+1 WHERE id='conn-bob'",
                    [],
                )
                .unwrap();
            }
            "cancel" => {
                c.execute(
                    "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
                    [&accepted.record_id],
                )
                .unwrap();
            }
            "expiry" => {
                c.execute(
                    "UPDATE operations SET expires_at=?2 WHERE record_id=?1",
                    params![accepted.record_id, crate::now() - 1],
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        let actor = actor_from_connection(&c, &f.keys[0]).unwrap();
        let public = rows::by_record(&c, &accepted.record_id).unwrap();
        let current = rows::for_authority(&c, &accepted.record_id).unwrap();
        let old = authority::execution(&c, &actor, &public, &f.store).map_err(|e| e.code);
        let new = current
            .require_execution(&c, &actor, &f.store)
            .map_err(|e| e.code);
        assert!(new.is_err(), "{change}");
        assert_eq!(new, old, "{change}");
    }
    let f = Fixture::new();
    let accepted = f.admit("authority-row-current-revocation");
    let admin = f.store.revalidate(&f.keys[1]).unwrap();
    f.store.revoke(&admin, &f.keys[0]).unwrap();
    let c = f.store.connection().unwrap();
    assert!(actor_from_connection(&c, &f.keys[0]).is_err());
    assert!(
        rows::for_authority(&c, &accepted.record_id).is_ok(),
        "facts alone are not a current actor grant"
    );
    drop(c);
    assert!(f
        .store
        .execute_operation_action(
            &f.keys[0],
            OperationAction::Get {
                operation_id: "authority-row-current-revocation".into()
            }
        )
        .is_err());
}
