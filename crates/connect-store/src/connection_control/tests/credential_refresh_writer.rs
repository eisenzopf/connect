//! Real Store admission and exact row transitions; controlled Core revision
//! observations do not qualify a physical credential handshake or media cell.
use super::*;
use crate::{AuthorityWait, PeerCommand};
use std::{collections::BTreeMap, time::Duration};

#[test]
fn credential_epoch_mutation_waits_for_original_media_and_reuses_its_exact_writer() {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let f = Fixture::new();
    let original_request = f.native_request_for(0, 600, "credential-writer");
    let source = consultation_reads::ready_request(&f, 0, original_request.clone());
    let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let next = f
        .store
        .issue(
            &identity,
            Scope::collaboration(vec!["*".into()]),
            crate::now() + 600,
        )
        .unwrap()
        .id;
    let key = f
        .store
        .resolve_peer_signing_key(&f.keys[0], "tnt_default", &original_request.key_id, "EdDSA")
        .unwrap();
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::SigningKey {
                operation_id: "bind-next-credential-writer".into(),
                identity_id: identity,
                credential_id: next.clone(),
                device_id: original_request.device.id.clone(),
                key_id: original_request.key_id.clone(),
                algorithm: SigningAlgorithm::EdDSA,
                public_key: URL_SAFE_NO_PAD.encode(&key.public_key),
                not_before: key.not_before,
                expires_at: key.expires_at,
                expected_revision: Some(0),
            },
        )
        .unwrap();
    let mut request = original_request;
    request.credentials.push(next);
    request.command = PeerCommand::Binding;
    // Prove the request is valid before creating the actual conflicting reader.
    let preflight = f
        .store
        .admit_native_credential_refresh(
            &request,
            &[source.clone()],
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    drop(preflight);
    let reader = f.store.try_native_media_read(&source, None).unwrap();
    let blocked = f.store.admit_native_credential_refresh(
        &request,
        &[source.clone()],
        &AuthorityWait::new(Duration::from_millis(20)),
    );
    let blocked_code = blocked.as_ref().err().map(|error| error.code.clone());
    // Join/drop every original guard before the causal old-condition assertion.
    drop(blocked);
    drop(reader);
    assert_eq!(
        blocked_code.as_deref(),
        Some("authority-busy"),
        "credential epoch writer overlapped an original media interval"
    );

    let (writer, plan) = f
        .store
        .admit_native_credential_refresh(
            &request,
            &[source.clone()],
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    assert_eq!(
        f.store
            .try_native_media_read(&source, None)
            .err()
            .expect("original writer must exclude a new media interval")
            .code,
        "authority-busy"
    );
    let attempt = f
        .store
        .prepare_native_credential_refresh(&plan, &writer, "original-credential-writer")
        .unwrap();
    f.store
        .begin_native_credential_refresh(&attempt, &plan, &writer)
        .unwrap();
    let next_origin = plan.targets()[0].next().clone();
    let revisions = BTreeMap::from([(
        source.offer_record_id().to_owned(),
        plan.targets()[0].expected_credential_revision() + 1,
    )]);
    let receipt = attempt.applied(revisions).unwrap();
    // The real original writer remains held: a nested lock acquisition here
    // would fail or wait on itself. Historical recording must reuse that owner.
    f.store
        .record_native_credential_refresh_applied(&receipt)
        .unwrap();
    drop(receipt);
    drop(plan);
    drop(writer);
    assert!(f.store.try_native_media_read(&source, None).is_err());
    assert!(f.store.try_native_media_read(&next_origin, None).is_ok());
}
