//! Controlled trusted-observer boundary, not a physical SIP/Core qualification.
use super::*;
use crate::{
    InitialTelephoneMediaOrigin, InitialTelephoneReadyObservation,
    PrivateInitialTelephoneRuntimeObservation,
};
use connect_protocol::transitions::OperationState;
mod scheduling;
mod source;

struct Captured {
    value: InitialTelephoneReadyObservation,
    changed: bool,
    calls: std::sync::atomic::AtomicUsize,
}
impl PrivateInitialTelephoneRuntimeObservation for Captured {
    fn observe_private(&self) -> Result<InitialTelephoneReadyObservation> {
        let mut value = self.value.clone();
        if self.changed && self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0 {
            value.stream_revision += 1;
        }
        Ok(value)
    }
}
impl crate::InitialTelephoneRuntimeObservation for Captured {
    fn observe(&self) -> Result<InitialTelephoneReadyObservation> {
        Ok(self.value.clone())
    }
}
fn consent() -> (
    Fixture,
    NativeConsultationOrigin,
    InitialTelephonePolicy,
    InitialTelephoneOrigin,
    Arc<InitialTelephoneMediaOrigin>,
    Captured,
    Arc<InitialTelephoneSignaling>,
) {
    let (f, parent, policy, _, initial) = prepared();
    let signal = signaling(&f, &initial);
    let capture = signal.capture_answer(signal.sip_call_id(), "PCMU").unwrap();
    let media = f
        .store
        .record_private_initial_telephone_answer(&capture)
        .unwrap()
        .media
        .unwrap();
    let observed = Captured {
        value: InitialTelephoneReadyObservation {
            runtime_connection_id: signal.runtime_connection_id().into(),
            call_id: signal.sip_call_id().into(),
            participant_id: initial.source().target.participant_id.clone(),
            membership_epoch: initial.source().target.membership_epoch,
            generation: 1,
            stream_revision: 2,
            participant_revision: 2,
            streams: vec!["telephone-audio".into()],
        },
        changed: false,
        calls: Default::default(),
    };
    (f, parent, policy, initial, media, observed, signal)
}
fn public_state(f: &Fixture) -> Vec<String> {
    let c = f.store.connection().unwrap();
    ["SELECT json_group_array(json_array(state,topology_revision)) FROM sessions", "SELECT json_group_array(json_array(session_id,participant_id,membership_epoch,state,role)) FROM session_participants", "SELECT json_group_array(json_array(session_id,participant_id,membership_epoch,capabilities_json)) FROM session_presence_capabilities"].into_iter().map(|sql|c.query_row(sql,[],|r|r.get(0)).unwrap()).collect()
}

#[test]
fn private_admission_uses_own_observed_codec_without_public_codec_or_media_authority() {
    let (f, _, _, _, media, _, _) = consent();
    let before = public_state(&f);
    let read = f
        .store
        .try_private_initial_telephone_admission_read(media.clone())
        .unwrap();
    assert!(read.current() && read.is_private_admission() && !read.is_connected());
    assert_eq!(read.accepted_capabilities(), &json!({"audio":["g.711-mu"]}));
    drop(read);
    assert_eq!(public_state(&f), before);
    assert!(f
        .store
        .try_initial_telephone_media_read(media.clone())
        .is_err());
    assert!(f
        .store
        .try_initial_telephone_signal_read(
            media,
            crate::InitialTelephoneSignalDirection::FromTelephone
        )
        .is_err());
}

#[test]
fn private_ready_preserves_root_and_public_presence_and_emits_only_to_original_private_members() {
    let (f, parent, _, initial, media, observed, _) = consent();
    let before = public_state(&f);
    let cache = f.store.session_ready_origins.lock().unwrap().len();
    assert!(f
        .store
        .record_initial_telephone_connected(media.clone(), &observed)
        .is_err());
    for _ in 0..2 {
        let record = f
            .store
            .record_private_initial_telephone_connected(media.clone(), &observed)
            .unwrap();
        assert_eq!(record.state, OperationState::Completed);
        let read = f
            .store
            .try_private_initial_telephone_admission_read(media.clone())
            .unwrap();
        assert!(read.is_connected() && read.is_private_admission());
    }
    assert_eq!(public_state(&f), before);
    assert_eq!(f.store.session_ready_origins.lock().unwrap().len(), cache);
    assert!(f
        .store
        .try_initial_telephone_media_read(media.clone())
        .is_err());
    let c = f.store.connection().unwrap();
    let root: (String, bool) = c
        .query_row(
            "SELECT state,lease_owner IS NULL FROM operations WHERE record_id=?1",
            [parent.record_id()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(root, ("accepted".into(), true));
    let audience=c.prepare("SELECT recipient FROM journal WHERE operation_record_id=?1 AND kind='connection_ready' ORDER BY recipient").unwrap().query_map([initial.record_id()],|r|r.get::<_,String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    let mut expected = initial
        .source()
        .audience
        .iter()
        .map(|m| m.identity_id.clone())
        .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(audience, expected);
    assert_eq!(c.query_row("SELECT count(*) FROM journal WHERE operation_record_id=?1 AND kind IN ('session_started','session_participant_joined')",[initial.record_id()],|r|r.get::<_,u64>(0)).unwrap(),0);
    let details: String = c
        .query_row(
            "SELECT connected_json FROM initial_telephone_answers WHERE operation_record_id=?1",
            [initial.record_id()],
            |r| r.get(0),
        )
        .unwrap();
    let details: serde_json::Value = serde_json::from_str(&details).unwrap();
    assert_eq!(details["private_audio"], true);
    assert_eq!(
        details["private_consultation_record_id"],
        parent.record_id()
    );
}

#[test]
fn private_admission_interval_fences_original_execution_and_independent_route_then_releases() {
    let (f, _, policy, _, media, _, _) = consent();
    let read = f
        .store
        .try_private_initial_telephone_admission_read(media)
        .unwrap();
    let identities = [
        f.keys[0].as_str(),
        f.keys[1].as_str(),
        f.keys[2].as_str(),
        policy.route_credential_id.as_str(),
    ]
    .map(|key| f.store.revalidate(key).unwrap().identity_id);
    for identity in &identities {
        assert!(f
            .store
            .try_acquire_authority(&[
                AuthorityScope::identity(identity, AuthorityMode::Write).unwrap()
            ])
            .is_err());
    }
    drop(read);
    for identity in &identities {
        drop(
            f.store
                .try_acquire_authority(&[
                    AuthorityScope::identity(identity, AuthorityMode::Write).unwrap()
                ])
                .unwrap(),
        );
    }
}

#[test]
fn private_admission_rechecks_original_authority_and_cannot_outlive_root_claim() {
    for fault in 0..9 {
        let (f, parent, policy, initial, media, observed, _) = consent();
        let (sql,key)=match fault {
            0=>("UPDATE credentials SET revoked=1 WHERE id=?1",f.keys[0].as_str()),
            1=>("UPDATE credentials SET revoked=1 WHERE id=?1",policy.endpoint_credential_id.as_str()),
            2=>("UPDATE credentials SET revoked=1 WHERE id=?1",policy.route_credential_id.as_str()),
            3=>("UPDATE endpoints SET revision=revision+1 WHERE id=?1",policy.endpoint_id.as_str()),
            4=>("UPDATE participants SET active=0 WHERE participant_id=?1",f.pids[1].as_str()),
            5=>("UPDATE operations SET cancel_requested=1 WHERE record_id=?1",parent.record_id()),
            6=>("UPDATE operations SET lease_owner='claimed',state='preparing' WHERE record_id=?1",parent.record_id()),
            7=>("UPDATE operations SET cancel_requested=1 WHERE record_id=?1",initial.record_id()),
            _=>("UPDATE participants SET active=1,membership_epoch=membership_epoch+1 WHERE participant_id=?1",f.pids[2].as_str()),
        };
        if fault == 8 {
            f.store
                .connection()
                .unwrap()
                .execute(
                    "UPDATE participants SET active=0 WHERE participant_id=?1",
                    [key],
                )
                .unwrap();
        }
        f.store.connection().unwrap().execute(sql, [key]).unwrap();
        assert!(
            f.store
                .try_private_initial_telephone_admission_read(media.clone())
                .is_err(),
            "fault {fault}"
        );
        assert!(
            f.store
                .record_private_initial_telephone_connected(media, &observed)
                .is_err(),
            "fault {fault}"
        );
        assert_eq!(f.store.connection().unwrap().query_row("SELECT connected_at IS NOT NULL FROM initial_telephone_answers WHERE operation_record_id=?1",[initial.record_id()],|r|r.get::<_,bool>(0)).unwrap(),false);
    }
}

#[test]
fn private_ready_rejects_wrong_physical_capture_and_rolls_back_changed_observation() {
    let (f, _, _, initial, media, mut observed, _) = consent();
    let original = observed.value.clone();
    for fault in 0..7 {
        observed.value = original.clone();
        match fault {
            0 => observed.value.runtime_connection_id = "other-core".into(),
            1 => observed.value.call_id = "other-call".into(),
            2 => observed.value.membership_epoch += 1,
            3 => observed.value.generation += 1,
            4 => observed.value.stream_revision = 1,
            5 => observed.value.streams = vec!["other-audio".into()],
            _ => observed.value.participant_revision = 0,
        }
        assert!(
            f.store
                .record_private_initial_telephone_connected(media.clone(), &observed)
                .is_err(),
            "fault {fault}"
        );
    }
    observed.value = original;
    observed.changed = true;
    let before = f.count("journal");
    assert!(f
        .store
        .record_private_initial_telephone_connected(media.clone(), &observed)
        .is_err());
    assert_eq!(f.count("journal"), before);
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT state FROM connections WHERE id=?1",
                [initial.connection_id()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "preparing"
    );
    assert!(!f
        .store
        .try_private_initial_telephone_admission_read(media)
        .unwrap()
        .is_connected());
}

#[test]
fn private_ready_sql_refuses_public_shape_or_wrong_original_quarantine_binding() {
    let (f, parent, _, initial, _, observed, _) = consent();
    let v = observed.value;
    let details = json!({"connection_id":initial.connection_id(),"participant_id":v.participant_id,"membership_epoch":v.membership_epoch,"generation":v.generation,"endpoint_id":initial.source().endpoint_id,"device_id":initial.source().device_id,"stream_revision":2,"participant_revision":2,"streams":v.streams,"codec":"g.711-mu","private_audio":true,"private_consultation_record_id":parent.record_id(),"runtime_connection_id":v.runtime_connection_id,"call_id":v.call_id});
    let c = f.store.connection().unwrap();
    for fault in 0..6 {
        let mut bad = details.clone();
        match fault {
            0 => {
                bad.as_object_mut().unwrap().remove("private_audio");
            }
            1 => bad["private_consultation_record_id"] = json!(initial.record_id()),
            2 => bad["runtime_connection_id"] = json!("other-core"),
            3 => bad["stream_revision"] = json!(1),
            4 => bad["codec"] = json!("opus"),
            _ => bad["membership_epoch"] = json!(999),
        }
        assert!(c.execute("UPDATE initial_telephone_answers SET connected_json=?2,connected_at=?3 WHERE operation_record_id=?1",params![initial.record_id(),bad.to_string(),crate::now()]).is_err(),"fault {fault}");
    }
}

#[test]
fn private_ready_cleanup_records_original_absence_after_revocation_and_cannot_reopen() {
    let (f, _, policy, _, media, observed, signal) = consent();
    f.store
        .record_private_initial_telephone_connected(media.clone(), &observed)
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET revoked=1 WHERE id=?1",
            [policy.endpoint_credential_id],
        )
        .unwrap();
    assert!(f
        .store
        .try_private_initial_telephone_admission_read(media.clone())
        .is_err());
    f.store
        .settle_initial_telephone_cleanup(&signal, &ControlledAbsent(true))
        .unwrap();
    assert!(f
        .store
        .try_private_initial_telephone_admission_read(media)
        .is_err());
}

#[test]
fn private_admission_expiry_requires_cleanup_instead_of_continuing_execution_authority() {
    let (f, parent, mut policy) = fixture();
    policy.timeout_seconds = 3;
    let reservation = f
        .store
        .reserve_sip_consultation_leg(&parent, policy)
        .unwrap();
    let initial = f
        .store
        .prepare_private_initial_telephone(&reservation)
        .unwrap();
    let signal = signaling(&f, &initial);
    let capture = signal.capture_answer(signal.sip_call_id(), "PCMU").unwrap();
    let media = f
        .store
        .record_private_initial_telephone_answer(&capture)
        .unwrap()
        .media
        .unwrap();
    let read = f
        .store
        .try_private_initial_telephone_admission_read(media.clone())
        .unwrap();
    assert!(read.expires_at() <= initial.expires_at());
    drop(read);
    let until = (initial.expires_at() - crate::now() + 1).max(1) as u64;
    std::thread::sleep(std::time::Duration::from_secs(until));
    assert!(f
        .store
        .try_private_initial_telephone_admission_read(media)
        .is_err());
    f.store
        .settle_initial_telephone_cleanup(&signal, &ControlledAbsent(true))
        .unwrap();
}

#[test]
fn private_ready_is_not_public_reconnect_coverage_when_public_connections_are_lost() {
    let (f, _, _, initial, media, observed, _) = consent();
    f.store
        .record_private_initial_telephone_connected(media, &observed)
        .unwrap();
    let c = f.store.connection().unwrap();
    c.execute(
        "UPDATE connections SET state='interrupted' WHERE id!=?1",
        [initial.connection_id()],
    )
    .unwrap();
    c.execute("UPDATE sessions SET state='active' WHERE id='session'", [])
        .unwrap();
    drop(c);
    let (_, pending) = f.store.session_reconnect_page("").unwrap();
    assert!(
        pending.contains(&(f.cid.clone(), "session".into())),
        "a quarantined private candidate cannot keep a lost public bridge connected"
    );
}

#[test]
fn private_ready_backup_preserves_quarantine_capture_but_refuses_old_runtime_readiness() {
    let (f, _, _, initial, media, observed, _) = consent();
    f.store
        .record_private_initial_telephone_connected(media.clone(), &observed)
        .unwrap();
    let backup = f._dir.path().join("private-ready-backup.sqlite3");
    let snapshot = f.store.backup(&backup).unwrap();
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), snapshot);
    let restored = Store::open(&backup, Limits::default()).unwrap();
    assert_ne!(restored.runtime_epoch(), f.store.runtime_epoch());
    assert!(restored
        .try_private_initial_telephone_admission_read(media.clone())
        .is_err());
    assert!(restored
        .record_private_initial_telephone_connected(media, &observed)
        .is_err());
    let c = restored.connection().unwrap();
    let (private,revision,retired):(bool,u64,bool)=c.query_row("SELECT json_extract(connected_json,'$.private_audio'),json_extract(connected_json,'$.stream_revision'),retired_at IS NOT NULL FROM initial_telephone_answers WHERE operation_record_id=?1",[initial.record_id()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert!(private);
    assert_eq!(revision, 2);
    // Historical capture remains useful without granting readiness in this runtime.
    let _ = retired;
}
