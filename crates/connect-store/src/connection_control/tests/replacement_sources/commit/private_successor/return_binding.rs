//! SQL provenance controls over real controlled Store replacement captures.
//! These are metadata/authorization tests, not decoded-media evidence.
use super::*;
use crate::NativeConsultationReturnOrigin;

fn return_command(
    m: &Movement,
    parent: &str,
    target: &NativeMediaOrigin,
) -> (String, TransitionCommand) {
    let identity = m.f.store.revalidate(&m.key).unwrap().identity_id;
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope
        .operations
        .extend(["session.consult.return".into(), "stream.subscribe".into()]);
    let key =
        m.f.store
            .issue(&identity, scope, crate::now() + 600)
            .unwrap()
            .id;
    let revision =
        m.f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT topology_revision FROM sessions WHERE id='session'",
                [],
                |r| r.get(0),
            )
            .unwrap();
    let command = TransitionCommand {
        operation_id: "return-current-private-successor".into(),
        conversation_id: m.f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(target.connection_id().into()),
        expected_revision: revision,
        intent: TransitionIntent::ConsultReturn {
            consultation_record_id: parent.into(),
            retained_connid: m.sources[2].connection_id().into(),
        },
        payload_extensions: Default::default(),
    };
    (key, command)
}

fn admitted_return(
    m: &Movement,
    parent: &str,
    target: &Arc<NativeMediaOrigin>,
) -> (String, NativeConsultationReturnOrigin) {
    let (key, command) = return_command(m, parent, target);
    let receipt =
        m.f.store
            .admit_transition_command(&key, command.clone())
            .unwrap();
    assert_eq!(receipt.state, OperationState::Accepted);
    assert!(receipt.effects.is_empty() && receipt.result.is_none());
    let origin =
        m.f.store
            .native_consultation_return_origin(&key, command)
            .unwrap();
    let mut actual = m.sources[1..].to_vec();
    actual.push(target.clone());
    assert!(m
        .f
        .store
        .try_native_return_read(&origin, &actual, &[])
        .unwrap()
        .current());
    (key, origin)
}

fn history(m: &Movement, parent: &str) -> (String, String, String) {
    m.f.store
        .connection()
        .unwrap()
        .query_row(
            "SELECT n.selection_json,v.selection_json,b.audience_json
             FROM native_consultation_attempts n
             JOIN native_private_lifetimes v ON v.operation_record_id=n.operation_record_id
             JOIN consultation_candidate_bindings b ON b.operation_record_id=n.operation_record_id
             WHERE n.operation_record_id=?1",
            [parent],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
}

fn context(m: &Movement, origin: &NativeConsultationReturnOrigin) -> (Value, String) {
    let (selection, audience): (String, String) = m
        .f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT context_selection_json,audience_json FROM native_consultation_returns WHERE operation_record_id=?1",
            [origin.record_id()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    (serde_json::from_str(&selection).unwrap(), audience)
}

/// Exercise the SQL trigger independently of the Rust admission validator.
/// The original accepted return supplies the actor/runtime and bounded root
/// deadline; this disposable row has a consistent typed request/digest. Every
/// probe rolls back before its verdict and grants no resource or media custody.
fn probe(
    m: &Movement,
    origin: &NativeConsultationReturnOrigin,
    selection: &Value,
    audience: &str,
    parent: &str,
    primary: &str,
    accepted: bool,
) {
    let before = (
        m.f.count("operations"),
        m.f.count("native_consultation_returns"),
        m.f.count("operation_effects"),
    );
    let mut request = origin.request().clone();
    request.operation_id = "private-return-trigger-probe-operation".into();
    request.connection_id = Some(primary.into());
    let TransitionIntent::ConsultReturn {
        consultation_record_id,
        ..
    } = &mut request.intent
    else {
        panic!("probe requires original return intent")
    };
    *consultation_record_id = parent.into();
    let mut request_value = serde_json::to_value(&request).unwrap();
    request_value
        .as_object_mut()
        .unwrap()
        .remove("operation_id");
    let (digest, intent) = crate::transitions::digest(&request_value).unwrap();
    let c = m.f.store.connection().unwrap();
    c.execute_batch("SAVEPOINT private_return_trigger_probe")
        .unwrap();
    assert_eq!(
        c.execute(
            "INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,
             conversation_id,session_id,connection_id,expected_revision,intent_json,state,
             created_at,updated_at,expires_at,ledger_version)
             SELECT 'private-return-trigger-probe-record',tenant_id,owner_id,?2,?3,command_type,
             conversation_id,session_id,?4,expected_revision,?5,'accepted',created_at,updated_at,expires_at,ledger_version
             FROM operations WHERE record_id=?1",
            params![origin.record_id(), request.operation_id, digest, primary, intent],
        )
        .unwrap(),
        1
    );
    let result = c.execute(
        "INSERT INTO native_consultation_returns(operation_record_id,consultation_record_id,
         credential_id,runtime_epoch,original_digest,context_selection_json,audience_json)
         SELECT 'private-return-trigger-probe-record',?2,credential_id,runtime_epoch,?3,?4,?5
         FROM native_consultation_returns WHERE operation_record_id=?1",
        params![
            origin.record_id(),
            parent,
            digest,
            crate::provisioning::canonical_json(selection).unwrap(),
            audience
        ],
    );
    c.execute_batch(
        "ROLLBACK TO private_return_trigger_probe; RELEASE private_return_trigger_probe",
    )
    .unwrap();
    drop(c);
    assert_eq!(
        before,
        (
            m.f.count("operations"),
            m.f.count("native_consultation_returns"),
            m.f.count("operation_effects")
        )
    );
    if accepted {
        assert_eq!(result.unwrap(), 1);
    } else {
        match result.unwrap_err() {
            rusqlite::Error::SqliteFailure(code, Some(message)) => {
                assert_eq!(code.code, rusqlite::ErrorCode::ConstraintViolation);
                assert_eq!(message, "return context has no original typed application");
            }
            error => panic!("unexpected failure before return binding: {error}"),
        }
    }
}

#[test]
fn private_successor_return_trigger_accepts_only_exact_current_primary_substitution() {
    let (m, parent) = private_scene();
    let saved = history(&m, &parent);
    let target = complete(&m, &parent);
    let (_, origin) = admitted_return(&m, &parent, &target);
    let (selected, audience) = context(&m, &origin);
    let index = selected["private"]
        .as_array()
        .unwrap()
        .iter()
        .position(|v| v["connection_id"] == target.connection_id())
        .unwrap();
    probe(
        &m,
        &origin,
        &selected,
        &audience,
        &parent,
        target.connection_id(),
        true,
    );
    for mutation in [
        "connection",
        "runtime",
        "generation",
        "credential",
        "key",
        "device",
        "participant",
        "epoch",
        "expiry",
        "key-hash",
        "extra-field",
        "wrong-slot",
        "remote",
        "partner",
        "outside",
        "stale-original",
        "audience",
        "wrong-parent",
    ] {
        let mut changed = selected.clone();
        let mut changed_audience = audience.clone();
        let mut changed_parent = parent.as_str();
        match mutation {
            "connection" => {
                changed["private"][index]["connection_id"] = json!(m.sources[2].connection_id())
            }
            "runtime" => {
                changed["private"][index]["runtime_connection_id"] = json!("unobserved-core-owner")
            }
            "generation" => {
                changed["private"][index]["generation"] = json!(target.generation() + 1)
            }
            "credential" => changed["private"][index]["credential_id"] = json!(m.f.keys[0]),
            "key" => changed["private"][index]["key_id"] = json!("native-key-0"),
            "device" => changed["private"][index]["device_id"] = json!("device-0"),
            "participant" => {
                changed["private"][index]["participant_id"] = json!(m.sources[2].participant_id())
            }
            "epoch" => changed["private"][index]["membership_epoch"] = json!(999),
            "expiry" => {
                changed["private"][index]["physical_expires_at"] = json!(crate::now() + 1200)
            }
            "key-hash" => changed["private"][index]["public_key_sha256"] = json!("0".repeat(64)),
            "extra-field" => changed["private"][index]["unrecorded"] = json!(true),
            "wrong-slot" => changed["private"].as_array_mut().unwrap().swap(0, 1),
            "remote" => changed["remote"]["connection_id"] = json!(m.sources[1].connection_id()),
            "partner" => changed["private"][1 - index]["generation"] = json!(999),
            "outside" => changed["outside"] = json!([]),
            "stale-original" => changed = serde_json::from_str(&saved.0).unwrap(),
            "audience" => changed_audience = "[]".into(),
            // A real committed replacement root is not its original typed
            // consultation parent. The exact binding failure precedes the FK.
            "wrong-parent" => changed_parent = target.offer_record_id(),
            _ => unreachable!(),
        }
        assert!(
            changed != selected || changed_audience != audience || changed_parent != parent,
            "mutation must alter the actual capture: {mutation}"
        );
        probe(
            &m,
            &origin,
            &changed,
            &changed_audience,
            changed_parent,
            target.connection_id(),
            false,
        );
        assert_eq!(history(&m, &parent), saved, "{mutation}");
    }
    assert_private(&m, &target);
}

#[test]
fn private_successor_return_uses_latest_generation_without_rewriting_original_context() {
    let (m, parent) = private_scene();
    let saved = history(&m, &parent);
    let first = complete(&m, &parent);
    let first_id = first.connection_id().to_owned();
    let original_primary = m.sources[0].connection_id().to_owned();
    let first_selection = {
        let c = m.f.store.connection().unwrap();
        replacement_lineage::resolve(
            &c,
            &parent,
            &saved.0,
            &original_primary,
            m.f.store.runtime_epoch(),
        )
        .unwrap()
        .selection
    };
    let m = second_scene(m, first);
    let second = complete(&m, &parent);
    let (_, origin) = admitted_return(&m, &parent, &second);
    let (selected, audience) = context(&m, &origin);
    assert_ne!(selected, first_selection);
    probe(
        &m,
        &origin,
        &selected,
        &audience,
        &parent,
        second.connection_id(),
        true,
    );
    // Use the old source ID in the disposable operation too, so this checks
    // latest-capture selection rather than merely a request/context mismatch.
    probe(
        &m,
        &origin,
        &first_selection,
        &audience,
        &parent,
        &first_id,
        false,
    );
    probe(
        &m,
        &origin,
        &serde_json::from_str(&saved.0).unwrap(),
        &audience,
        &parent,
        &original_primary,
        false,
    );
    assert_eq!(history(&m, &parent), saved);
    assert_private(&m, &second);
}

#[test]
fn private_successor_return_keeps_current_actor_and_physical_authority() {
    for revoke in ["actor", "physical-key"] {
        let (m, parent) = private_scene();
        let target = complete(&m, &parent);
        let saved = history(&m, &parent);
        let (key, command) = return_command(&m, &parent, &target);
        let before_effects = m.f.count("operation_effects");
        if revoke == "actor" {
            let before_operations = m.f.count("operations");
            m.f.store
                .connection()
                .unwrap()
                .execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&key])
                .unwrap();
            assert!(m.f.store.admit_transition_command(&key, command).is_err());
            assert_eq!(m.f.count("operations"), before_operations);
            assert_eq!(m.f.count("native_consultation_returns"), 0);
        } else {
            m.f.store
                .admit_transition_command(&key, command.clone())
                .unwrap();
            let origin =
                m.f.store
                    .native_consultation_return_origin(&key, command)
                    .unwrap();
            let mut actual = m.sources[1..].to_vec();
            actual.push(target.clone());
            assert!(m
                .f
                .store
                .try_native_return_read(&origin, &actual, &[])
                .unwrap()
                .current());
            m.f.store
                .connection()
                .unwrap()
                .execute(
                    "UPDATE peer_signing_keys SET revoked=1 WHERE key_id=?1",
                    [&m.target_request.key_id],
                )
                .unwrap();
            assert!(m
                .f
                .store
                .try_native_return_read(&origin, &actual, &[])
                .is_err());
        }
        assert_eq!(m.f.count("operation_effects"), before_effects);
        assert_eq!(history(&m, &parent), saved);
    }
}

#[test]
fn private_successor_return_requires_recorded_departure_of_the_latest_physical_owner() {
    for observed in [false, true] {
        let (m, parent) = private_scene();
        let target = complete(&m, &parent);
        let saved = history(&m, &parent);
        let (key, command) = return_command(&m, &parent, &target);
        let c = m.f.store.connection().unwrap();
        c.execute(
            "UPDATE connections SET state='retired',generation=generation+1 WHERE id=?1",
            [target.connection_id()],
        )
        .unwrap();
        if observed {
            c.execute(
                "UPDATE native_connection_lifetimes SET retired=1 WHERE connection_id=?1",
                [target.connection_id()],
            )
            .unwrap();
        }
        drop(c);
        let before = (m.f.count("operations"), m.f.count("operation_effects"));
        let outcome = m.f.store.admit_transition_command(&key, command);
        if observed {
            let accepted = outcome.unwrap();
            assert_eq!(accepted.state, OperationState::Accepted);
            assert!(accepted.effects.is_empty() && accepted.result.is_none());
            let selected: String = m.f.store.connection().unwrap().query_row(
                "SELECT context_selection_json FROM native_consultation_returns WHERE operation_record_id=?1",
                [&accepted.record_id], |r| r.get(0),
            ).unwrap();
            let selected: Value = serde_json::from_str(&selected).unwrap();
            assert!(selected["private"]
                .as_array()
                .unwrap()
                .iter()
                .any(|slot| slot["connection_id"] == target.connection_id()
                    && slot["generation"] == target.generation()));
        } else {
            assert_eq!(outcome.unwrap_err().code, "not-ready");
            assert_eq!(m.f.count("operations"), before.0);
        }
        assert_eq!(m.f.count("operation_effects"), before.1);
        assert_eq!(history(&m, &parent), saved);
    }
}
