//! Bounded existing appearance continuity; no connected/media/new-join fact.
use super::*;
use sha2::{Digest, Sha256};

/// Called only for an existing joined appearance whose last old Connection has
/// become terminal. A pending replacement can keep that logical appearance,
/// never create one or supply Core/current media authority.
pub(crate) fn preserves_joined_presence(
    c: &Connection,
    store: &Store,
    sid: &str,
    pid: &str,
    epoch: u64,
) -> Result<bool> {
    let joined:bool=c.query_row(
        "SELECT EXISTS(SELECT 1 FROM session_connected_presence WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3 AND state='joined')",
        params![sid,pid,epoch],|r|r.get(0),
    ).map_err(|_|Error::storage())?;
    if !joined {
        return Ok(false);
    }
    let owner = store.require_runtime_owner()?;
    let mut q = c.prepare_cached(
        "SELECT n.operation_record_id,n.credential_id,n.original_digest,n.plan_json,n.selection_json,e.observation_json FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id JOIN native_replacement_steps step ON step.operation_record_id=n.operation_record_id AND step.phase='start' AND step.started=1 AND step.observation_json IS NOT NULL JOIN operation_effects e ON e.id=step.effect_id AND e.operation_record_id=o.record_id AND e.provider='native-replacement-start' AND e.state='confirmed' AND e.attempt=1 AND e.submission_generation=1 WHERE o.session_id=?1 AND json_extract(n.plan_json,'$.expected_preparation.participant')=?2 AND json_extract(n.plan_json,'$.expected_preparation.membership_epoch')=?3 AND n.runtime_epoch=?4 AND o.command_type='connection.replace' AND o.state='preparing' AND o.cancel_requested=0 AND o.tombstoned=0 AND o.expires_at>?5 AND n.expires_at>?5 AND o.worker_runtime=?6 AND o.lease_owner=n.reservation_id AND o.lease_expires_at=n.expires_at AND o.digest=n.original_digest AND n.candidate_connection_id=json_extract(n.plan_json,'$.candidate_connection_id') AND n.candidate_runtime_id=json_extract(n.plan_json,'$.candidate_runtime_id') AND n.generation=json_extract(n.plan_json,'$.generation') AND e.correlation_key=e.id AND e.idempotency_mode='none' AND json_extract(step.observation_json,'$.candidate')=n.candidate_runtime_id AND json_extract(step.observation_json,'$.generation')=n.generation AND json_extract(step.observation_json,'$.stream_revision')=2 AND json_extract(step.observation_json,'$.resources_prepared')=1 AND json_extract(step.observation_json,'$.prepared_muted')=1 AND json_extract(step.observation_json,'$.media_ready')=0 AND json_extract(e.observation_json,'$.intent.connection_id')=n.candidate_connection_id AND json_extract(e.observation_json,'$.intent.session_id')=o.session_id AND json_extract(e.observation_json,'$.intent.conversation_id')=o.conversation_id AND json_extract(e.observation_json,'$.intent.operation_id')=o.operation_id ORDER BY o.record_id LIMIT 65",
    ).map_err(|_|Error::storage())?;
    let candidates = q
        .query_map(
            params![
                sid,
                pid,
                epoch,
                store.runtime_epoch(),
                crate::now(),
                store.transition_runtime
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            },
        )
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    if candidates.len() == 65 {
        return Err(Error::new(
            "capacity-exceeded",
            "Pending appearance claim inventory exceeds its bound",
        ));
    }
    for (record, credential, digest, plan, selection, descriptor) in candidates {
        let checked: Result<()> = (|| {
            let row = rows::for_authority(c, &record)?;
            let request = row.request()?;
            let origin = NativeReplacementOrigin {
                owner: Arc::downgrade(&owner),
                runtime: store.runtime_epoch().into(),
                record,
                credential,
                digest,
                request,
            };
            let plan: serde_json::Value =
                serde_json::from_str(&plan).map_err(|_| Error::storage())?;
            let selection: serde_json::Value =
                serde_json::from_str(&selection).map_err(|_| Error::storage())?;
            let descriptor: serde_json::Value =
                serde_json::from_str(&descriptor).map_err(|_| Error::storage())?;
            require_claim(
                c,
                store,
                &origin,
                &plan,
                &selection,
                &descriptor,
                sid,
                pid,
                epoch,
            )?;
            Ok(())
        })();
        match checked {
            Ok(()) => return Ok(owner.current()),
            Err(error)
                if matches!(
                    error.code.as_str(),
                    "forbidden"
                        | "unauthorized"
                        | "conflict"
                        | "not-ready"
                        | "reconciliation-required"
                ) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(false)
}
fn text<'a>(value: &'a serde_json::Value, key: &str) -> Result<&'a str> {
    value[key].as_str().ok_or_else(Error::forbidden)
}
fn binding(
    value: &serde_json::Value,
    kind: &str,
    platform: &str,
) -> Result<crate::PeerAuthorityRequest> {
    Ok(crate::PeerAuthorityRequest {
        credentials: vec![text(value, "credential_id")?.into()],
        device: crate::PeerDeviceClaim {
            id: text(value, "device_id")?.into(),
            kind: kind.into(),
            platform: platform.into(),
        },
        key_id: text(value, "key_id")?.into(),
        algorithm: text(value, "algorithm")?.into(),
        command: crate::PeerCommand::Binding,
    })
}
fn require_claim(
    c: &Connection,
    store: &Store,
    origin: &NativeReplacementOrigin,
    plan: &serde_json::Value,
    selection: &serde_json::Value,
    descriptor: &serde_json::Value,
    sid: &str,
    pid: &str,
    epoch: u64,
) -> Result<()> {
    let source = &selection["source"];
    let source_id = text(source, "connection_id")?;
    if origin.request.session_id != sid
        || origin.request.connection_id.as_deref() != Some(source_id)
        || source["participant_id"] != pid
        || source["membership_epoch"] != epoch
        || plan["expected_preparation"]["participant"] != pid
        || plan["expected_preparation"]["membership_epoch"] != epoch
        || plan["source_generation"] != source["generation"]
        || plan["expected_preparation"]["source"] != source["runtime_connection_id"]
    {
        return Err(Error::forbidden());
    }
    // Use the existing complete current operator/member/endpoint/original-cutoff
    // boundary, intentionally without requiring the retired source's old IO.
    let target_request = binding(
        descriptor,
        text(plan, "target_device_kind")?,
        text(plan, "target_device_platform")?,
    )?;
    let (target_actor, target_key, _, target_until) =
        crate::peer_authority::checked(store, c, &target_request)?;
    target_actor.actor.require("connection.offer")?;
    let (_, original_until) =
        origin.continuation_authority(store, c, &target_actor.actor.identity_id)?;
    let intent: connect_protocol::connection_control::OfferIntent =
        serde_json::from_value(descriptor["intent"].clone()).map_err(|_| Error::storage())?;
    if descriptor["public_key"] != serde_json::json!(target_key.public_key)
        || descriptor["device_id"] != target_key.device_id
        || intent.connection_id != plan["candidate_connection_id"]
        || intent.session_id != sid
        || intent.conversation_id != origin.request.conversation_id
        || intent.offer.get("private_consultation").is_some()
        || intent.offer.get("handoff_preparation").is_some()
    {
        return Err(Error::forbidden());
    }
    let target_endpoint:bool=c.query_row(
        "SELECT EXISTS(SELECT 1 FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.tenant_id=e.tenant_id AND d.revoked=0 WHERE e.id=?1 AND e.identity_id=?2 AND e.device_id=?3 AND e.verification_state='verified' AND e.revision=?4)",
        params![text(descriptor,"endpoint_id")?,target_actor.actor.identity_id,target_request.device.id,plan["target_endpoint_revision"].as_u64().ok_or_else(Error::forbidden)?],|r|r.get(0),
    ).map_err(|_|Error::storage())?;
    if !target_endpoint
        || descriptor["endpoint_id"] != plan["expected_preparation"]["endpoint"]
        || descriptor["device_id"] != plan["expected_preparation"]["device"]
    {
        return Err(Error::forbidden());
    }
    let target_member = crate::audience::member(
        c,
        &target_actor.actor.identity_id,
        &origin.request.conversation_id,
        true,
    )?;
    if target_member.participant_id != pid || target_member.membership_epoch != epoch {
        return Err(Error::forbidden());
    }
    let source_request = crate::PeerAuthorityRequest {
        credentials: vec![text(source, "credential_id")?.into()],
        device: crate::PeerDeviceClaim {
            id: text(source, "device_id")?.into(),
            kind: text(source, "device_kind")?.into(),
            platform: text(source, "device_platform")?.into(),
        },
        key_id: text(source, "key_id")?.into(),
        algorithm: text(source, "key_algorithm")?.into(),
        command: crate::PeerCommand::Binding,
    };
    let (source_actor, source_key, _, source_until) =
        crate::peer_authority::checked(store, c, &source_request)?;
    source_actor.actor.require("connection.offer")?;
    if source_actor.actor.identity_id != target_actor.actor.identity_id
        || source["public_key_sha256"] != hex::encode(Sha256::digest(&source_key.public_key))
    {
        return Err(Error::forbidden());
    }
    let exact:bool=c.query_row(
        "SELECT EXISTS(SELECT 1 FROM native_admission_sources n JOIN native_connection_lifetimes life ON life.operation_record_id=n.operation_record_id AND life.connection_id=n.connection_id AND life.generation=n.generation AND life.runtime_epoch=n.attempt_runtime JOIN operations o ON o.record_id=n.operation_record_id AND o.state='completed' AND o.cancel_requested=0 AND o.tombstoned=0 JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id AND e.state='confirmed' JOIN connections v ON v.id=n.connection_id JOIN connection_offers mapping ON mapping.connection_id=v.id WHERE n.connection_id=?1 AND n.session_id=?2 AND n.participant_id=?3 AND n.membership_epoch=?4 AND n.attempt_runtime=?5 AND n.generation=?6 AND n.credential_id=?7 AND n.key_id=?8 AND n.device_id=?9 AND n.endpoint_id=?10 AND n.source_expires_at=?11 AND n.effect_started=1 AND mapping.runtime_connection_id=?12 AND v.session_id=n.session_id AND v.participant_id=n.participant_id AND v.membership_epoch=n.membership_epoch AND v.endpoint_id=n.endpoint_id AND v.identity_id=?13 AND ((life.retired=0 AND v.generation=n.generation AND v.observed_runtime_id=n.attempt_runtime AND v.state IN ('ready','active','held')) OR (life.retired=1 AND v.generation=n.generation+1 AND v.state='retired' AND v.observed_runtime_id IS NULL AND v.observed_at IS NULL AND v.readiness_json IS NULL)) AND NOT EXISTS(SELECT 1 FROM native_media_credential_bindings b WHERE b.parent_record_id=n.operation_record_id AND b.runtime_epoch=n.attempt_runtime AND (b.state='uncertain' OR (b.state='prepared' AND b.started=1))))",
        params![source_id,sid,pid,epoch,store.runtime_epoch(),source["generation"].as_u64().ok_or_else(Error::forbidden)?,source_actor.actor.credential_id,source_request.key_id,source_request.device.id,text(source,"endpoint_id")?,source["physical_expires_at"].as_i64().ok_or_else(Error::forbidden)?,text(source,"runtime_connection_id")?,source_actor.actor.identity_id],|r|r.get(0),
    ).map_err(|_|Error::storage())?;
    let role:Option<String>=c.query_row("SELECT role FROM session_participants WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3 AND state='present'",params![sid,pid,epoch],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
    let original_role = text(plan, "source_role")?;
    if !exact
        || role.as_deref() != Some(original_role)
        || crate::now()
            >= original_until.min(source_until).min(target_until).min(
                source["physical_expires_at"]
                    .as_i64()
                    .ok_or_else(Error::forbidden)?,
            )
    {
        return Err(Error::forbidden());
    }
    for actor in [&source_actor.actor, &target_actor.actor] {
        for recipient in crate::connection_control::intent_resources(c, actor, &intent)? {
            if !actor.permits_recipient(&recipient) {
                return Err(Error::forbidden());
            }
        }
    }
    Ok(())
}
