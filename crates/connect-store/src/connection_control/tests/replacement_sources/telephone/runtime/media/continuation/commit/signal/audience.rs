//! Current committed SIP outsiders and historical audience outcomes, without IO.
use super::*;
use crate::SipAudienceOrigin;
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
