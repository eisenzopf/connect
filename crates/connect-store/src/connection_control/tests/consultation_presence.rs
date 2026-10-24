//! A physical custodian must not mistake unavailable pending signal IO for EOF.
use super::*;

#[test]
fn native_private_custody_survives_quarantine_and_preparation_without_signal_authority() {
    let (f, sources, _, root) = consultation_reads::fixture();
    let candidate = consultation_audiences::candidate(&f, &root);
    let signal = || {
        f.store
            .native_signal_facts_at(&f.store.connection().unwrap(), &candidate)
    };
    assert!(signal().is_err(), "Preparation is not a signal grant");
    assert!(f.store.try_native_connection_current(&candidate).unwrap());
    let lease = consultation_audiences::prepare(&f, &sources, &candidate, &root);
    assert!(
        signal().is_err(),
        "An unsubmitted plan is not a signal grant"
    );
    assert!(f.store.try_native_connection_current(&candidate).unwrap());
    assert!(f
        .store
        .native_consultation_attempt_read(&lease)
        .unwrap()
        .current());
    drop(
        f.store
            .try_acquire_authority(&[crate::AuthorityScope::conversation(
                &f.cid,
                crate::AuthorityMode::Write,
            )
            .unwrap()])
            .unwrap(),
    );
    let c = f.store.connection().unwrap();
    assert!(c
        .query_row(
            "SELECT state='ready' AND generation=1 FROM connections WHERE id=?1",
            [candidate.connection_id()],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
}

#[test]
fn pending_native_private_custody_still_requires_original_current_physical_and_execution_facts() {
    for prepared in [false, true] {
        for fault in 0..5 {
            let (f, sources, _, root) = consultation_reads::fixture();
            let candidate = consultation_audiences::candidate(&f, &root);
            if prepared {
                consultation_audiences::prepare(&f, &sources, &candidate, &root);
            }
            let (sql, key) = match fault {
                0 => (
                    "UPDATE credentials SET revoked=1 WHERE id=?1",
                    f.keys[0].as_str(),
                ),
                1 => (
                    "UPDATE credentials SET revoked=1 WHERE id=?1",
                    f.keys[2].as_str(),
                ),
                2 => ("UPDATE devices SET revoked=1 WHERE id=?1", "device-2"),
                3 => (
                    "UPDATE credentials SET recipients='[]' WHERE id=?1",
                    f.keys[2].as_str(),
                ),
                _ => (
                    "UPDATE connections SET generation=generation+1 WHERE id=?1",
                    candidate.connection_id(),
                ),
            };
            f.store.connection().unwrap().execute(sql, [key]).unwrap();
            assert!(
                f.store.try_native_connection_current(&candidate).is_err(),
                "prepared={prepared} fault={fault}"
            );
        }
    }
}
