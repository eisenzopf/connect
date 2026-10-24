//! Current committed SIP outsiders and historical audience outcomes, without IO.
use super::*;
use crate::SipAudienceOrigin;
mod retained;

#[test]
fn established_sip_captured_origin_keeps_route_authority_separate_and_owns_no_idle_reader() {
    let (f, phone, _, route) = established();
    drop(f.store.try_telephone_media_read(phone.clone()).unwrap());
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &f.keys[0]).unwrap();
    let binding = {
        let c = f.store.connection().unwrap();
        let (_, binding, _) = f.store.telephone_audience_facts(&c, &phone).unwrap();
        assert!(f
            .store
            .sip_audience_snapshot_facts(
                &c,
                &binding,
                &phone.snapshot().conversation_id,
                &phone.snapshot().session_id
            )
            .is_ok());
        binding
    };
    let writer = f
        .store
        .try_acquire_authority(&[crate::AuthorityScope::identity(
            &phone.snapshot().route_owner_id,
            crate::AuthorityMode::Write,
        )
        .unwrap()])
        .unwrap();
    assert!(f.store.try_telephone_media_read(phone.clone()).is_err());
    drop(writer);
    drop(f.store.try_telephone_media_read(phone.clone()).unwrap());
    permissions(&f, &route, &["dtmf.send", "dtmf.receive"]);
    let c = f.store.connection().unwrap();
    assert!(f
        .store
        .sip_audience_snapshot_facts(
            &c,
            &binding,
            &phone.snapshot().conversation_id,
            &phone.snapshot().session_id
        )
        .is_err());
}

#[test]
fn established_sip_captured_origin_refuses_foreign_dialog_and_retired_generation() {
    let (f, phone, _, _) = established();
    drop(f.store.try_telephone_media_read(phone.clone()).unwrap());
    let c = f.store.connection().unwrap();
    let (_, binding, _) = f.store.telephone_audience_facts(&c, &phone).unwrap();
    for (key, value) in [
        ("sip_call_id", json!("foreign-call")),
        ("generation", json!(3)),
        (
            "endpoint_credential_id",
            json!("invented-initial-credential"),
        ),
    ] {
        let mut forged = binding.clone();
        forged[key] = value;
        assert!(
            f.store
                .sip_audience_snapshot_facts(
                    &c,
                    &forged,
                    &phone.snapshot().conversation_id,
                    &phone.snapshot().session_id
                )
                .is_err(),
            "{key}"
        );
    }
    c.execute(
        "UPDATE telephone_connection_lifetimes SET retired=1 WHERE operation_record_id=?1",
        [phone.record_id()],
    )
    .unwrap();
    assert!(!Store::recorded_sip_departure(&c, &binding, f.store.runtime_epoch()).unwrap());
    drop(c);
    f.store
        .settle_telephone_connection_cleanup(
            &phone,
            &Absent {
                id: phone.snapshot().runtime_connection_id.clone(),
                generation: phone.snapshot().generation,
            },
        )
        .unwrap();
    let c = f.store.connection().unwrap();
    assert!(Store::recorded_sip_departure(&c, &binding, f.store.runtime_epoch()).unwrap());
    let mut foreign = binding.clone();
    foreign["sip_call_id"] = json!("foreign-call");
    assert!(!Store::recorded_sip_departure(&c, &foreign, f.store.runtime_epoch()).unwrap());
    assert!(f
        .store
        .sip_audience_snapshot_facts(
            &c,
            &binding,
            &phone.snapshot().conversation_id,
            &phone.snapshot().session_id
        )
        .is_err());
}
#[test]
fn established_sip_audience_uses_current_route_without_retired_native_credentials() {
    let (f, phone, _, route) = established();
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &f.keys[0]).unwrap();
    let c = f.store.connection().unwrap();
    let (scopes, binding, until) = f.store.telephone_audience_facts(&c, &phone).unwrap();
    assert!(!scopes.is_empty() && until <= crate::now() + 5);
    assert_eq!(binding["source_kind"], "established-sip");
    assert_eq!(binding["generation"], 2);
    assert_eq!(binding["sip_call_id"], phone.snapshot().media.call_id);
    assert_eq!(binding["route_credential_id"], route);
    assert!(binding.get("endpoint_credential_id").is_none());
    assert!(binding.get("key_id").is_none());
    let _: crate::peer_authority::SipSnapshot = serde_json::from_value(binding.clone()).unwrap();
    let mut forged = binding.clone();
    forged["source_kind"] = json!("initial-sip");
    assert!(serde_json::from_value::<crate::peer_authority::SipSnapshot>(forged).is_err());
    let mut forged = binding.clone();
    forged["key_id"] = json!("invented-native-key");
    assert!(serde_json::from_value::<crate::peer_authority::SipSnapshot>(forged).is_err());
    drop(c);
    permissions(&f, &route, &["dtmf.send", "dtmf.receive"]);
    assert!(f
        .store
        .telephone_audience_facts(&f.store.connection().unwrap(), &phone)
        .is_err());
}
#[test]
fn established_sip_audience_requires_complete_current_nonduplicated_scene() {
    let (f, phone, _, _) = established();
    let c = f.store.connection().unwrap();
    let source = SipAudienceOrigin::Established(phone.clone());
    let facts = |sources: &[SipAudienceOrigin], native: Vec<String>| {
        f.store.sip_audience_facts(
            &c,
            &phone.snapshot().conversation_id,
            &phone.snapshot().session_id,
            sources,
            native,
        )
    };
    assert!(facts(&[source.clone()], vec![]).is_ok());
    assert_eq!(
        facts(&[], vec![]).unwrap_err().code,
        "authority-unavailable"
    );
    assert!(facts(&[source.clone(), source.clone()], vec![]).is_err());
    assert!(facts(&[source.clone()], vec![source.connection_id().into()]).is_err());
    c.execute(
        "UPDATE telephone_connection_lifetimes SET retired=1 WHERE operation_record_id=?1",
        [phone.record_id()],
    )
    .unwrap();
    assert!(facts(&[source], vec![]).is_err());
    assert!(facts(&[], vec![]).is_ok());
}
#[test]
fn established_sip_audience_captured_revision_never_rebinds_call_or_rewrites_later_state() {
    let (f, phone, _, _) = established();
    let c = f.store.connection().unwrap();
    let (_, binding, _) = f.store.telephone_audience_facts(&c, &phone).unwrap();
    let revision = || {
        c.query_row(
            "SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",
            [&phone.snapshot().connection_id],
            |r| r.get::<_, u64>(0),
        )
        .unwrap()
    };
    let original = revision();
    let mut forged = binding.clone();
    forged["sip_call_id"] = json!("foreign-dialog");
    f.store
        .record_sip_audience_revision(&c, &forged, f.store.runtime_epoch(), original, original + 1)
        .unwrap();
    assert_eq!(revision(), original);
    f.store
        .record_sip_audience_revision(&c, &binding, "foreign-runtime", original, original + 1)
        .unwrap();
    assert_eq!(revision(), original);
    f.store
        .record_sip_audience_revision(
            &c,
            &binding,
            f.store.runtime_epoch(),
            original,
            original + 1,
        )
        .unwrap();
    assert_eq!(revision(), original + 1);
    f.store
        .record_sip_audience_revision(
            &c,
            &binding,
            f.store.runtime_epoch(),
            original,
            original + 1,
        )
        .unwrap();
    assert_eq!(revision(), original + 1);
    assert!(f
        .store
        .record_sip_audience_revision(
            &c,
            &binding,
            f.store.runtime_epoch(),
            original + 1,
            original + 3
        )
        .is_err());
    c.execute(
        "UPDATE telephone_connection_lifetimes SET retired=1 WHERE operation_record_id=?1",
        [phone.record_id()],
    )
    .unwrap();
    f.store
        .record_sip_audience_revision(
            &c,
            &binding,
            f.store.runtime_epoch(),
            original + 1,
            original + 2,
        )
        .unwrap();
    assert_eq!(revision(), original + 1);
}
