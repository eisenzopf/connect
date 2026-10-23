//! Deterministic metadata-only outcome from the authoritative terminal intent.
use super::*;
use crate::Service;
use connect_protocol::{Error, Result as ConnectResult};
use rvoip_core::{
    ids::ParticipantId,
    records::{
        RecordAudience, RecordCategory, RecordCompleteness, RecordProvenance, RecordRetention,
        RecordSource, RecordTarget,
    },
};
use serde_json::{json, Value};

fn array<'a>(value: &'a Value, key: &str) -> ConnectResult<&'a [Value]> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(Error::storage)
}
fn audience(
    values: &[Value],
    group: &str,
    epochs: &mut Vec<durable::RecordSourceEpoch>,
) -> ConnectResult<Vec<ParticipantId>> {
    let mut participants = std::collections::BTreeSet::new();
    for value in values {
        let participant = value
            .get("participant_id")
            .and_then(Value::as_str)
            .ok_or_else(Error::storage)?;
        let epoch = value
            .get("membership_epoch")
            .and_then(Value::as_u64)
            .ok_or_else(Error::storage)?;
        epochs.push(durable::RecordSourceEpoch {
            source_group: group.into(),
            participant_id: participant.into(),
            membership_epoch: epoch,
        });
        participants.insert(participant.to_owned());
    }
    Ok(participants
        .into_iter()
        .map(ParticipantId::from_string)
        .collect())
}
fn source(
    id: &str,
    target: RecordTarget,
    audience: RecordAudience,
    policy: &str,
    retention: RecordRetention,
) -> RecordSource {
    RecordSource::new(
        id.into(),
        1,
        target,
        RecordCategory::Metadata,
        audience,
        policy.into(),
        retention,
        retention,
    )
    .with_provenance(RecordProvenance {
        completeness: Some(RecordCompleteness::Partial),
        capture_complete: false,
        ..Default::default()
    })
}
fn encode(
    intent: &durable::RecordIntent,
) -> ConnectResult<(durable::RecordBytes, Vec<durable::RecordSourceEpoch>)> {
    if intent.policy != "connect-session-metadata-v1" || intent.snapshot["version"] != 1 {
        return Err(Error::storage());
    }
    let snapshot = &intent.snapshot;
    let ended = snapshot["ended_at"]
        .as_i64()
        .and_then(|t| chrono::DateTime::<chrono::Utc>::from_timestamp(t, 0))
        .ok_or_else(Error::storage)?;
    intent.retention.validate()?;
    let retention = match intent.retention.metadata.seconds() {
        None => RecordRetention::UntilDeleted,
        Some(seconds) => RecordRetention::Until(
            ended
                .checked_add_signed(chrono::Duration::seconds(i64::from(seconds)))
                .ok_or_else(Error::storage)?,
        ),
    };
    // This producer emits metadata only. The other category limits are frozen
    // with the intent, without implying that transcript/media capture exists.
    let source =
        |id: &str, target, audience, policy: &str| source(id, target, audience, policy, retention);
    let appearances = array(snapshot, "appearances")?;
    let connections = array(snapshot, "connections")?;
    if appearances.len() > 256 || connections.len() > 512 {
        return Err(Error::storage());
    }
    let mut epochs = vec![];
    audience(array(snapshot, "audience")?, "session", &mut epochs)?;
    let mut rules = vec![
        source(
            "document",
            RecordTarget::Document,
            RecordAudience::Policy,
            &intent.policy,
        ),
        source(
            "session.outcome",
            RecordTarget::Analysis(0),
            RecordAudience::Policy,
            &intent.policy,
        ),
    ];
    let parties=appearances.iter().enumerate().map(|(index,p)| {
        rules.push(source(&format!("appearance.{index}"),RecordTarget::Party(index as u32),RecordAudience::Policy,&intent.policy));
        json!({"connect_participant_id":p["participant_id"],"membership_epoch":p["membership_epoch"],"role":p["role"],"state_at_terminal_intent":p["state"]})
    }).collect::<Vec<_>>();
    rules.push(source(
        "session.capture",
        RecordTarget::Analysis(1),
        RecordAudience::Policy,
        "connect-session-capture-diagnostics-v1",
    ));
    let mut analysis = vec![
        json!({"type":"session-outcome","vendor":"connect","encoding":"json","mediatype":"application/json","body":{
            "kind":"session-outcome","state":snapshot["state"],"media":snapshot["media"],"started_at":snapshot["started_at"],"ended_at":snapshot["ended_at"],"reason":snapshot["reason"],"reason_code":snapshot["reason_code"],
            "capture_policy":intent.policy,"retention_policy":intent.retention.name,"capture_complete":false,"recorded_audio":false,"transcript":"unavailable","raw_capture":"disabled"}}),
        json!({"type":"capture-diagnostics","vendor":"connect","encoding":"json","mediatype":"application/json","body":{
            "kind":"capture-diagnostics",
            "appearance_omissions":snapshot["appearance_count"].as_u64().ok_or_else(Error::storage)?.saturating_sub(appearances.len() as u64),
            "connection_omissions":snapshot["connection_count"].as_u64().ok_or_else(Error::storage)?.saturating_sub(connections.len() as u64),
            "audience_omissions":snapshot["audience_count"].as_u64().ok_or_else(Error::storage)?.saturating_sub(array(snapshot,"audience")?.len() as u64),
            "reason_truncated":snapshot["reason_truncated"].as_u64().filter(|v|*v<=1).map(|v|v==1).ok_or_else(Error::storage)?}}),
    ];
    for (index, connection) in connections.iter().enumerate() {
        let group = format!("connection.{index}");
        // The named installation policy selects only this Connection's original
        // owner epoch for its lineage metadata. No shared Session grant selects
        // the source, and this never captures media/provider/context content.
        let owner = audience(std::slice::from_ref(connection), &group, &mut epochs)?;
        let dependencies = appearances
            .iter()
            .enumerate()
            .filter(|(_, appearance)| {
                appearance["participant_id"] == connection["participant_id"]
                    && appearance["membership_epoch"] == connection["membership_epoch"]
            })
            .map(|(index, _)| format!("appearance.{index}"))
            .collect();
        rules.push(
            source(
                &group,
                RecordTarget::Analysis(analysis.len() as u32),
                RecordAudience::Participants(owner),
                "connect-connection-owner-metadata-v1",
            )
            .with_references(dependencies),
        );
        analysis.push(json!({"type":"connection-lineage","vendor":"connect","encoding":"json","mediatype":"application/json","body":{"kind":"connection-lineage","connection_id":connection["connection_id"],"participant_id":connection["participant_id"],"membership_epoch":connection["membership_epoch"],"generation_at_terminal_intent":connection["generation"],"replaces":connection["replaces"],"state_at_terminal_intent":connection["state"]}}));
    }
    let body=serde_json::to_vec(&json!({"vcon":"0.4.0","extensions":["connect-session-metadata"],"uuid":intent.uuid,"created_at":ended,"parties":parties,"analysis":analysis})).map_err(|_|Error::storage())?;
    let manifest =
        RecordManifest::new(intent.policy.clone(), rules).map_err(|_| Error::storage())?;
    let value = VconRecord::new(body.into(), manifest).map_err(|_| Error::storage())?;
    Ok((bytes(&value).map_err(|_| Error::storage())?, epochs))
}
impl Service {
    /// Trusted local finalization of at most 32 original terminal intents. It
    /// exposes counts only, grants no record access and performs no provider IO.
    /// A storage error leaves unconfirmed intents durable for the next attempt.
    pub async fn finalize_record_outcomes(
        &self,
        limit: usize,
    ) -> ConnectResult<durable::RecordIntentBatch> {
        let store = self.store.clone();
        let result = tokio::task::spawn_blocking(move || {
            let pending = store.pending_record_intents(limit)?;
            let mut batch = durable::RecordIntentBatch {
                examined: pending.len(),
                stored: 0,
                failed: 0,
            };
            for id in pending {
                match store.finalize_record_intent(&id, encode) {
                    Ok(true) => batch.stored += 1,
                    Ok(false) => (),
                    Err(_) => batch.failed += 1,
                }
            }
            Ok::<_, Error>(batch)
        })
        .await
        .map_err(|_| Error::storage())?;
        if result.as_ref().is_ok_and(|b| b.stored > 0) {
            self.notify_changed();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn maximum_metadata_profile_retains_all_sources_within_manifest_bound() {
        let pid = |n: usize| format!("part_{n:04}_{}", "a".repeat(118));
        let appearances=(0..256).map(|n|json!({"participant_id":pid(n),"membership_epoch":1,"role":"participant","state":"present"})).collect::<Vec<_>>();
        let connections=(0..512).map(|n|json!({"connection_id":format!("conn_{n:04}_{}","b".repeat(118)),"participant_id":pid(n%256),"membership_epoch":1,"generation":1,"replaces":null,"state":"active"})).collect::<Vec<_>>();
        let audience = (0..256)
            .map(|n| json!({"participant_id":pid(n),"membership_epoch":1}))
            .collect::<Vec<_>>();
        let mut id = *uuid::Uuid::new_v4().as_bytes();
        id[6] = (id[6] & 15) | 128;
        let intent = durable::RecordIntent {
            uuid: uuid::Uuid::from_bytes(id).to_string(),
            scope: durable::RecordScope {
                tenant_id: "tnt_default".into(),
                conversation_id: "conv_profile".into(),
                session_id: "sess_profile".into(),
            },
            policy: "connect-session-metadata-v1".into(),
            retention: Default::default(),
            snapshot: json!({"version":1,"state":"ended","started_at":100,"ended_at":200,"reason":"normal","reason_code":200,"reason_truncated":0,"media":["audio"],"appearances":appearances,"appearance_count":256,"connections":connections,"connection_count":512,"audience":audience,"audience_count":256}),
        };
        let (bytes, epochs) = encode(&intent).unwrap();
        assert!(bytes.manifest.len() <= 1024 * 1024);
        assert_eq!(epochs.len(), 256 + 512);
        let record = record(&bytes).unwrap();
        assert_eq!(record.manifest().sources().len(), 3 + 256 + 512);
        let body: Value = serde_json::from_slice(record.body()).unwrap();
        assert_eq!(body["analysis"][1]["body"]["appearance_omissions"], 0);
        assert_eq!(body["analysis"][1]["body"]["connection_omissions"], 0);
    }
}
