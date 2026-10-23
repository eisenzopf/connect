//! Telephone policy and phase custody; no SIP/PSTN network or media claim.
mod runtime;
use super::*;
use crate::{
    NativeReplacementOrigin, NativeReplacementPreparationLease, TelephoneReplacementPolicy,
};

fn policy() -> TelephoneReplacementPolicy {
    TelephoneReplacementPolicy {
        route_reference: "config_sip_outbound".into(),
        sip_authority: "carrier.example:5060".into(),
        confirmation_digits: "42#".into(),
        timeout_seconds: 30,
    }
}
fn setup(
    dial: bool,
) -> (
    Fixture,
    NativeReplacementOrigin,
    Vec<std::sync::Arc<crate::NativeMediaOrigin>>,
    String,
) {
    let (f, origin, sources, endpoint, _) = setup_policy(dial, false);
    (f, origin, sources, endpoint)
}
fn setup_runtime(
    dial: bool,
) -> (
    Fixture,
    NativeReplacementOrigin,
    Vec<std::sync::Arc<crate::NativeMediaOrigin>>,
    String,
    String,
) {
    setup_policy(dial, true)
}
fn setup_policy(
    dial: bool,
    lifecycle_permissions: bool,
) -> (
    Fixture,
    NativeReplacementOrigin,
    Vec<std::sync::Arc<crate::NativeMediaOrigin>>,
    String,
    String,
) {
    let (f, _, _, mut command, sources) = fixture_with_sources();
    let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let ProvisioningOutcome::Identity {
        identity_id: route_owner,
        ..
    } = f
        .store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::Identity {
                operation_id: "create-callback-route-owner".into(),
                alias: "callback-route-owner".into(),
                kind: IdentityKind::System,
                display_name: None,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let route_admin = f
        .store
        .issue(&route_owner, Scope::operator(), crate::now() + 600)
        .unwrap()
        .id;
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .store
        .execute_provisioning(
            &route_admin,
            ProvisioningCommand::Endpoint {
                operation_id: "provision-callback".into(),
                identity_id: identity.clone(),
                device: DeviceSpec {
                    id: "telephone-device".into(),
                    kind: "telephone".into(),
                    platform: "isolated-policy-fixture".into(),
                },
                transport: EndpointTransport::Telephone,
                address: "+14155550123".into(),
                capabilities: vec!["audio".into()],
                secret_ref: Some("config_sip_outbound".into()),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    f.store
        .execute_provisioning(
            &route_admin,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: "verify-callback".into(),
                endpoint_id: endpoint_id.clone(),
                expected_revision: 0,
                evidence_ref: "isolated-policy".into(),
            },
        )
        .unwrap();
    command.intent = TransitionIntent::Replace {
        endpoint_id: Some(endpoint_id.clone()),
        candidate_connid: None,
        retained_connid: sources[1].connection_id().into(),
    };
    command.payload_extensions.insert(
        "destination".into(),
        json!("sip:+19999999999@attacker.invalid"),
    );
    command
        .payload_extensions
        .insert("confirmation_digits".into(), json!("1"));
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.push("connection.replace".into());
    if lifecycle_permissions {
        scope
            .operations
            .extend(["operation.cancel".into(), "operation.execute".into()]);
    }
    if !dial {
        scope.operations.retain(|op| op != "sip_dial");
    }
    let key = f
        .store
        .issue(&identity, scope, crate::now() + 600)
        .unwrap()
        .id;
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let origin = f.store.native_replacement_origin(&key, command).unwrap();
    (f, origin, sources, endpoint_id, route_admin)
}
fn prepare(
    f: &Fixture,
    origin: &NativeReplacementOrigin,
    sources: &[std::sync::Arc<crate::NativeMediaOrigin>],
    policy: &TelephoneReplacementPolicy,
) -> Result<Option<NativeReplacementPreparationLease>> {
    let read = f.store.try_native_replacement_preparation_read(
        origin,
        &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
        &sources[1],
    )?;
    f.store
        .prepare_native_replacement(read, policy.request("telephone-next".into(), 1))
}
#[test]
fn telephone_replacement_policy_binds_verified_destination_trunk_and_configured_confirmation() {
    let (f, origin, sources, endpoint) = setup(true);
    let before = f.count("connections");
    let lease = prepare(&f, &origin, &sources, &policy()).unwrap().unwrap();
    let plan = lease.plan().unwrap();
    assert_eq!(plan["telephone_route"]["endpoint_id"], endpoint);
    assert_eq!(
        plan["telephone_route"]["destination"],
        "sip:+14155550123@carrier.example:5060"
    );
    assert_eq!(
        plan["telephone_route"]["route_reference"],
        "config_sip_outbound"
    );
    assert_eq!(
        plan["confirmation"],
        json!({"type":"answered-dtmf","digits":"42#"})
    );
    assert_eq!(plan["timeout_seconds"], 30);
    assert_eq!(
        prepare(&f, &origin, &sources, &policy())
            .unwrap()
            .unwrap()
            .plan()
            .unwrap(),
        plan
    );
    let read = f
        .store
        .native_replacement_preparation_attempt_read(&lease)
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_native_replacement_preparation(&lease, read)
        .unwrap();
    drop(read);
    // A lost begin result cannot create another preparation or callback attempt.
    assert!(prepare(&f, &origin, &sources, &policy()).unwrap().is_none());
    assert_eq!(attempt.plan().unwrap(), plan);
    assert_eq!(f.count("connections"), before);
    assert_eq!(f.count("sip_legs"), 0);
    assert_eq!(f.count("native_replacement_steps"), 1);
}
#[test]
fn telephone_replacement_requires_independent_current_dial_permission() {
    let (f, origin, sources, _) = setup(false);
    let effects = f.count("operation_effects");
    assert!(prepare(&f, &origin, &sources, &policy()).is_err());
    assert_eq!(f.count("native_replacements"), 0);
    assert_eq!(f.count("operation_effects"), effects);
}
#[test]
fn telephone_replacement_rejects_missing_policy_and_cross_trunk_without_effects() {
    let (f, origin, sources, _) = setup(true);
    let effects = f.count("operation_effects");
    let mut wrong = policy();
    wrong.route_reference = "another-trunk".into();
    assert!(prepare(&f, &origin, &sources, &wrong).is_err());
    let read = f
        .store
        .try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1],
        )
        .unwrap();
    assert!(f
        .store
        .prepare_native_replacement(read, custody::request())
        .is_err());
    let read = f
        .store
        .try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1],
        )
        .unwrap();
    let mut altered = policy().request("telephone-next".into(), 1);
    altered.confirmation = crate::ReplacementConfirmation::AnsweredDtmf { digits: "1".into() };
    assert!(f.store.prepare_native_replacement(read, altered).is_err());
    assert_eq!(f.count("operation_effects"), effects);
    assert_eq!(f.count("native_replacements"), 0);
}
#[test]
fn telephone_replacement_preparation_rejects_changed_policy_and_revoked_endpoint() {
    let (f, origin, sources, endpoint) = setup(true);
    let lease = prepare(&f, &origin, &sources, &policy()).unwrap().unwrap();
    for change in ["digits", "timeout", "authority"] {
        let mut changed = policy();
        match change {
            "digits" => changed.confirmation_digits = "7#".into(),
            "timeout" => changed.timeout_seconds = 20,
            _ => changed.sip_authority = "other.example".into(),
        }
        assert!(prepare(&f, &origin, &sources, &changed).is_err());
    }
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::RevokeEndpoint {
                operation_id: "revoke-callback".into(),
                endpoint_id: endpoint,
                expected_revision: 1,
            },
        )
        .unwrap();
    assert!(f
        .store
        .native_replacement_preparation_attempt_read(&lease)
        .is_err());
    assert_eq!(f.count("native_replacement_steps"), 1);
    assert_eq!(f.count("sip_legs"), 0);
}
#[test]
fn telephone_replacement_policy_rejects_ambiguous_routing_and_confirmation() {
    for authority in [
        "",
        "user@carrier.example",
        "carrier.example;lr",
        "carrier.example?x=1",
        "carrier.example/path",
        "carrier.example\r\nInjected:x",
        "carrier.example:0",
        "carrier.example:65536",
        "carrier.example:+5060",
        "carrier.example:",
        "carrier.example:5060:5061",
        "-carrier.example",
        "carrier..example",
        "Carrier.example",
    ] {
        let mut p = policy();
        p.sip_authority = authority.into();
        assert!(p.validate().is_err(), "{authority:?}");
    }
    for authority in [
        "carrier.example",
        "localhost:5060",
        "127.0.0.1:5060",
        "[::1]:5060",
    ] {
        let mut p = policy();
        p.sip_authority = authority.into();
        p.validate().unwrap();
    }
    for digits in ["", "x", "12345678901234567", "42\n"] {
        let mut p = policy();
        p.confirmation_digits = digits.into();
        assert!(p.validate().is_err());
    }
    for seconds in [0, 61, u32::MAX] {
        let mut p = policy();
        p.timeout_seconds = seconds;
        assert!(p.validate().is_err());
    }
    assert_eq!(
        format!("{:?}", policy()),
        "TelephoneReplacementPolicy(redacted)"
    );
}

#[test]
fn telephone_replacement_route_owner_is_fenced_and_rechecked_before_begin() {
    let (f, origin, sources, _) = setup(true);
    let lease = prepare(&f, &origin, &sources, &policy()).unwrap().unwrap();
    let read = f
        .store
        .native_replacement_preparation_attempt_read(&lease)
        .unwrap();
    let owner = read.context().target_route_owner_id.clone();
    assert_ne!(owner, read.caller().actor.identity_id);
    assert_ne!(owner, read.source().session().actor.identity_id);
    assert_ne!(owner, read.retained().target().session.actor.identity_id);
    let scope = crate::AuthorityScope::identity(&owner, crate::AuthorityMode::Write).unwrap();
    assert!(f.store.try_acquire_authority(&[scope.clone()]).is_err());
    drop(read);
    let writer = f.store.try_acquire_authority(&[scope]).unwrap();
    assert!(f
        .store
        .native_replacement_preparation_attempt_read(&lease)
        .is_err());
    drop(writer);
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::RevokeIdentity {
                operation_id: "revoke-route-owner".into(),
                identity_id: owner,
                expected_revision: 0,
            },
        )
        .unwrap();
    assert!(f
        .store
        .native_replacement_preparation_attempt_read(&lease)
        .is_err());
    let started: bool = f
        .store
        .connection()
        .unwrap()
        .query_row("SELECT started FROM native_replacement_steps", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(!started);
    assert_eq!(f.count("sip_legs"), 0);
}

#[test]
fn telephone_replacement_unknown_preparation_cannot_reenter_or_claim_a_call() {
    let (f, origin, sources, _) = setup(true);
    let lease = prepare(&f, &origin, &sources, &policy()).unwrap().unwrap();
    let read = f
        .store
        .native_replacement_preparation_attempt_read(&lease)
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_native_replacement_preparation(&lease, read)
        .unwrap();
    drop(read);
    f.store
        .native_replacement_preparation_uncertain(attempt)
        .unwrap();
    assert!(prepare(&f, &origin, &sources, &policy()).is_err());
    let fact: (String, u64) = f.store.connection().unwrap().query_row(
        "SELECT e.state,e.attempt FROM operation_effects e JOIN native_replacement_steps s ON s.effect_id=e.id",[],|r|Ok((r.get(0)?,r.get(1)?))
    ).unwrap();
    assert_eq!(fact, ("unknown".into(), 1));
    assert_eq!(f.count("native_replacements"), 1);
    assert_eq!(f.count("connections"), 2);
    assert_eq!(f.count("sip_legs"), 0);
}

#[test]
fn telephone_replacement_policy_cannot_be_applied_to_a_native_endpoint() {
    let (f, _, _, origin, sources) = custody::setup();
    let effects = f.count("operation_effects");
    assert!(prepare(&f, &origin, &sources, &policy()).is_err());
    assert_eq!(f.count("operation_effects"), effects);
    assert_eq!(f.count("native_replacements"), 0);
}
