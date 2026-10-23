use super::*;
fn request(f: &Fixture, name: &str, targets: Vec<String>) -> session_control::Request {
    let mut r = f.request.clone();
    r.operation_id = format!("gateway-{name}");
    r.session_id = format!("gateway-session-{name}");
    let session_control::Action::Invite { participants, .. } = &mut r.action else {
        unreachable!()
    };
    *participants = targets;
    r
}
fn apply(f: &Fixture, r: &session_control::Request) -> Result<Outcome> {
    f.store.execute(
        &f.keys[0],
        &Operation::SessionControl { request: r.clone() },
    )
}
fn endpoint_for(
    f: &Fixture,
    id: &str,
    identity: usize,
    transport: &str,
    address: &str,
) -> InitialTelephonePolicy {
    let c = f.store.connection().unwrap();
    let device = format!("device-{id}");
    c.execute("INSERT INTO devices(id,identity_id,tenant_id,kind,platform) VALUES(?1,?2,'tnt_default','phone','isolated')",params![device,f.ids[identity]]).unwrap();
    c.execute("INSERT INTO endpoints(id,identity_id,tenant_id,device_id,transport,address,capabilities_json,verification_state,verification_evidence,verified_at,secret_ref,route_owner_id) VALUES(?1,?2,'tnt_default',?3,?4,?5,'[\"audio\"]','verified','fixture',?6,'initial-trunk',?7)",params![id,f.ids[identity],device,transport,address,crate::now(),f.ids[0]]).unwrap();
    let mut policy = f.policy.clone();
    policy.endpoint_id = id.into();
    policy.endpoint_credential_id = f.keys[identity].clone();
    policy
}
#[test]
fn configured_gateway_tel_and_identity_selectors_keep_original_member_and_once_only_intent() {
    let f = Fixture::new();
    let _gateway = f
        .store
        .install_initial_telephone_gateways(vec![f.policy.clone()])
        .unwrap();
    for (name, target) in [
        ("tel", "tel:+15555550111".into()),
        ("identity", f.ids[1].clone()),
        ("participant", f.parts[1].clone()),
    ] {
        let r = request(&f, name, vec![target]);
        let first = apply(&f, &r).unwrap();
        assert_eq!(apply(&f, &r).unwrap(), first);
        let members = f.store.initial_telephone_invitees(&f.keys[0], &r).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].participant_id, f.parts[1]);
        let origin = f
            .store
            .prepare_initial_telephone(&f.keys[0], r.clone(), &f.parts[1], f.policy.clone())
            .unwrap();
        assert_eq!(origin.source().request, r);
        assert_eq!(
            origin.source().destination,
            "sip:+15555550111@127.0.0.1:5099"
        );
        assert_eq!(
            f.store
                .prepare_initial_telephone(&f.keys[0], r, &f.parts[1], f.policy.clone())
                .unwrap()
                .operation_id(),
            origin.operation_id()
        );
    }
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM session_invitation_gateway_targets",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM initial_telephone_operations",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        3
    );
}
#[test]
fn configured_gateway_sip_uri_uses_only_verified_endpoint_and_exact_configured_authority() {
    let f = Fixture::new();
    let policy = endpoint_for(&f, "sip-gateway", 1, "sip", "sip:bob@127.0.0.1:5099");
    let mut wrong = policy.clone();
    wrong.sip_authority = "127.0.0.1:5100".into();
    assert_eq!(
        f.store
            .install_initial_telephone_gateways(vec![wrong])
            .unwrap_err()
            .code,
        "forbidden"
    );
    let _gateway = f
        .store
        .install_initial_telephone_gateways(vec![policy.clone()])
        .unwrap();
    let r = request(&f, "sip", vec!["sip:bob@127.0.0.1:5099".into()]);
    apply(&f, &r).unwrap();
    let origin = f
        .store
        .prepare_initial_telephone(&f.keys[0], r, &f.parts[1], policy)
        .unwrap();
    assert_eq!(origin.source().destination, "sip:bob@127.0.0.1:5099");
    for target in [
        "sip:bob@127.0.0.1:5100",
        "sip:other@127.0.0.1:5099",
        "sip:bob@example.com",
    ] {
        assert_eq!(
            apply(&f, &request(&f, "unknown", vec![target.into()]))
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
}
#[test]
fn gateway_selectors_refuse_current_credential_device_endpoint_host_and_recipient_withdrawal() {
    for case in [
        "missing-host",
        "closed",
        "dropped",
        "device",
        "endpoint",
        "endpoint-revision",
        "endpoint-key",
        "route-key",
        "ceiling",
        "member",
    ] {
        let mut f = Fixture::new();
        f.policy.route_credential_id = f
            .store
            .issue(&f.ids[0], crate::Scope::operator(), crate::now() + 3600)
            .unwrap()
            .id;
        let gateway = f
            .store
            .install_initial_telephone_gateways(vec![f.policy.clone()])
            .unwrap();
        if case == "closed" {
            gateway.close();
        }
        let _gateway = if matches!(case, "missing-host" | "dropped") {
            drop(gateway);
            None
        } else {
            Some(gateway)
        };
        {
            let c = f.store.connection().unwrap();
            match case {
                "device" => {
                    c.execute(
                        "UPDATE devices SET revoked=1 WHERE id='initial-phone-device'",
                        [],
                    )
                    .unwrap();
                }
                "endpoint" => {
                    c.execute("UPDATE endpoints SET verification_state='revoked',secret_ref=NULL WHERE id='initial-phone'",[]).unwrap();
                }
                "endpoint-revision" => {
                    c.execute("UPDATE endpoints SET revision=revision+1,verification_state='revoked',secret_ref=NULL WHERE id='initial-phone'",[]).unwrap();
                }
                "endpoint-key" => {
                    c.execute(
                        "UPDATE credentials SET revoked=1 WHERE id=?1",
                        [&f.policy.endpoint_credential_id],
                    )
                    .unwrap();
                }
                "route-key" => {
                    c.execute(
                        "UPDATE credentials SET revoked=1 WHERE id=?1",
                        [&f.policy.route_credential_id],
                    )
                    .unwrap();
                }
                "ceiling" => {
                    c.execute(
                        "UPDATE credentials SET recipients='[]' WHERE id=?1",
                        [&f.keys[0]],
                    )
                    .unwrap();
                }
                "member" => {
                    c.execute(
                        "UPDATE participants SET active=0 WHERE participant_id=?1",
                        [&f.parts[1]],
                    )
                    .unwrap();
                }
                _ => (),
            }
        }
        let r = request(&f, case, vec!["tel:+15555550111".into()]);
        assert!(apply(&f, &r).is_err(), "withdrawal case {case}");
        let c = f.store.connection().unwrap();
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM session_invitation_gateway_targets",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM initial_telephone_operations",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
    }
}
#[test]
fn uri_aliases_cannot_invite_one_participant_twice_or_resolve_an_outside_identity() {
    let f = Fixture::new();
    let _gateway = f
        .store
        .install_initial_telephone_gateways(vec![f.policy.clone()])
        .unwrap();
    let r = request(
        &f,
        "duplicate",
        vec![f.ids[1].clone(), "tel:+15555550111".into()],
    );
    assert_eq!(apply(&f, &r).unwrap_err().code, "invalid-request");
    let id = f.store.create_identity("outside-gateway").unwrap().id;
    let key = f
        .store
        .issue(&id, crate::Scope::operator(), crate::now() + 3600)
        .unwrap()
        .id;
    {
        let c = f.store.connection().unwrap();
        c.execute("INSERT INTO devices(id,identity_id,tenant_id,kind,platform) VALUES('outside-device',?1,'tnt_default','phone','isolated')",[&id]).unwrap();
        c.execute("INSERT INTO endpoints(id,identity_id,tenant_id,device_id,transport,address,capabilities_json,verification_state,verification_evidence,verified_at,secret_ref,route_owner_id) VALUES('outside-endpoint',?1,'tnt_default','outside-device','telephone','+15555550222','[\"audio\"]','verified','fixture',?2,'initial-trunk',?3)",params![id,crate::now(),f.ids[0]]).unwrap();
    }
    _gateway.close();
    let mut policy = f.policy.clone();
    policy.endpoint_id = "outside-endpoint".into();
    policy.endpoint_credential_id = key;
    let _outside = f
        .store
        .install_initial_telephone_gateways(vec![policy])
        .unwrap();
    assert_eq!(
        apply(&f, &request(&f, "outside", vec!["tel:+15555550222".into()]))
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn original_uri_receipt_cannot_retarget_after_endpoint_revocation_and_address_reassignment() {
    let f = Fixture::new();
    let old = f
        .store
        .install_initial_telephone_gateways(vec![f.policy.clone()])
        .unwrap();
    let r = request(&f, "frozen", vec!["tel:+15555550111".into()]);
    let first = apply(&f, &r).unwrap();
    {
        let c = f.store.connection().unwrap();
        assert!(c
            .execute(
                "UPDATE session_invitation_gateway_targets SET reference='tel:+15555550999'",
                []
            )
            .is_err());
        assert!(c
            .execute("DELETE FROM session_invitation_gateway_targets", [])
            .is_err());
        c.execute("UPDATE endpoints SET verification_state='revoked',secret_ref=NULL,revision=revision+1 WHERE id='initial-phone'",[]).unwrap();
    }
    old.close();
    let replacement = endpoint_for(&f, "reassigned-phone", 2, "telephone", "+15555550111");
    let _new = f
        .store
        .install_initial_telephone_gateways(vec![replacement.clone()])
        .unwrap();
    assert_eq!(apply(&f, &r).unwrap(), first);
    assert_eq!(
        f.store.initial_telephone_invitees(&f.keys[0], &r).unwrap()[0].participant_id,
        f.parts[1]
    );
    assert!(f
        .store
        .prepare_initial_telephone(&f.keys[0], r.clone(), &f.parts[1], f.policy.clone())
        .is_err());
    assert!(f
        .store
        .prepare_initial_telephone(&f.keys[0], r, &f.parts[2], replacement.clone())
        .is_err());
    let fresh = request(&f, "new-intent", vec!["tel:+15555550111".into()]);
    apply(&f, &fresh).unwrap();
    assert_eq!(
        f.store
            .prepare_initial_telephone(&f.keys[0], fresh, &f.parts[2], replacement)
            .unwrap()
            .source()
            .target
            .participant_id,
        f.parts[2]
    );
}
#[test]
fn gateway_resolution_and_invitation_rows_roll_back_together_then_same_intent_recovers() {
    let f = Fixture::new();
    let _gateway = f
        .store
        .install_initial_telephone_gateways(vec![f.policy.clone()])
        .unwrap();
    let r = request(&f, "storage", vec!["tel:+15555550111".into()]);
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER fail_gateway_capture BEFORE INSERT ON session_invitation_gateway_targets BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert_eq!(apply(&f, &r).unwrap_err().code, "storage-unavailable");
    {
        let c = f.store.connection().unwrap();
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM sessions WHERE id=?1",
                [&r.session_id],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM session_invitation_gateway_targets",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
        c.execute_batch("DROP TRIGGER fail_gateway_capture")
            .unwrap();
    }
    let first = apply(&f, &r).unwrap();
    assert_eq!(apply(&f, &r).unwrap(), first);
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM session_invitation_gateway_targets",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn frozen_uri_revision_cannot_adopt_a_later_verified_endpoint_revision() {
    let f = Fixture::new();
    let _gateway = f
        .store
        .install_initial_telephone_gateways(vec![f.policy.clone()])
        .unwrap();
    let original = request(&f, "old-revision", vec!["tel:+15555550111".into()]);
    let first = apply(&f, &original).unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE endpoints SET revision=revision+1 WHERE id='initial-phone'",
            [],
        )
        .unwrap();
    // Receipt replay is history; it cannot renew a pending physical target.
    assert_eq!(apply(&f, &original).unwrap(), first);
    assert_eq!(
        f.store
            .prepare_initial_telephone(&f.keys[0], original, &f.parts[1], f.policy.clone(),)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM initial_telephone_operations",
                [],
                |r| r.get::<_, u64>(0),
            )
            .unwrap(),
        0
    );
    let fresh = request(&f, "new-revision", vec!["tel:+15555550111".into()]);
    apply(&f, &fresh).unwrap();
    let origin = f
        .store
        .prepare_initial_telephone(&f.keys[0], fresh, &f.parts[1], f.policy.clone())
        .unwrap();
    assert_eq!(origin.source().endpoint_revision, 1);
}
