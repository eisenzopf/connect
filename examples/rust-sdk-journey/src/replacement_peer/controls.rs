//! Original journal observations and distinct, durable, current-authorized video controls.
use super::*;
use connect_client::{canonical::ReplyBatch, ClientError};
#[derive(Clone)]
pub(super) struct Admission {
    pub revision: u64,
    pub generation: u64,
    pub receipt: Value,
}
pub(super) async fn completion(
    client: &mut CanonicalClient,
    command: &TransitionCommand,
    root: &payloads::OperationUpdated,
    candidate: &str,
    device: &str,
    own: &[selected::Binding],
    minimum_revision: &mut u64,
) -> Result<Option<Admission>> {
    let batch = client
        .request_value(
            "operation.get",
            json!({"operation_id":command.operation_id}),
            CommandScope {
                conversation_id: Some(command.conversation_id.clone()),
                session_id: Some(command.session_id.clone()),
                connection_id: command.connection_id.clone(),
                ..Default::default()
            },
        )
        .await?;
    if completion_read_pending(&batch)? {
        // Only this completed, signed read was refused. Keep the physical
        // owner and original root; the caller's existing programme/poll bound
        // governs the next observation. No mutation or control is retried.
        emit(json!({"phase":"original-completion-read-deferred"})).await?;
        return Ok(None);
    }
    let updates = batch
        .frames
        .iter()
        .filter(|frame| {
            frame.kind() == "operation.updated"
                && frame.scope(Scope::Reply) == Some(batch.request_id.as_str())
        })
        .collect::<Vec<_>>();
    if updates.len() != 1 {
        return Err("original operation read omitted its unique correlated result".into());
    }
    observed(
        &updates[0].original()["payload"],
        command,
        root,
        candidate,
        device,
        own,
        minimum_revision,
    )
}
fn completion_read_pending(batch: &ReplyBatch) -> Result<bool> {
    match batch.ack() {
        Ok(_) => Ok(false),
        Err(ClientError::Application(problem)) if problem.code == "storage-unavailable" => Ok(true),
        Err(error) => Err(error.into()),
    }
}
fn observed(
    value: &Value,
    command: &TransitionCommand,
    root: &payloads::OperationUpdated,
    candidate: &str,
    device: &str,
    own: &[selected::Binding],
    minimum_revision: &mut u64,
) -> Result<Option<Admission>> {
    // The typed operation projection intentionally drops journal extensions.
    // Validate the original signed payload for command/precondition identity.
    let original = serde_json::to_value(root)?;
    let revision = value["revision"]
        .as_u64()
        .ok_or("replacement status omitted revision")?;
    if value["record_id"] != original["record_id"]
        || value["operation_id"] != command.operation_id
        || value["command_type"] != "connection.replace"
        || value["expected_revision"] != command.expected_revision
        || revision < *minimum_revision
    {
        return Err("replacement status changed original operation identity".into());
    }
    *minimum_revision = revision;
    match value["state"].as_str() {
        Some("accepted" | "preparing" | "ready" | "committing") => Ok(None),
        Some("completed") => {
            let result = &value["result"];
            if result["connection_id"] != candidate
                || result["device_id"] != device
                || result["replaces"].as_str() != command.connection_id.as_deref()
            {
                return Err("completed replacement changed its intended physical target".into());
            }
            let generation = result["generation"]
                .as_u64()
                .filter(|v| *v >= 2)
                .ok_or("replacement generation missing")?;
            let revision = result["readiness"]["stream_revision"]
                .as_u64()
                .filter(|v| *v > 0)
                .ok_or("replacement stream revision missing")?;
            let ids = result["readiness"]["streams"]
                .as_array()
                .ok_or("replacement resource identities missing")?;
            let actual = ids
                .iter()
                .map(Value::as_str)
                .collect::<Option<std::collections::BTreeSet<_>>>()
                .ok_or("invalid replacement resource identity")?;
            let expected = own
                .iter()
                .map(|v| v.id.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            if actual.len() != ids.len() || actual != expected {
                return Err("completed replacement lost a negotiated resource identity".into());
            }
            Ok(Some(Admission {
                revision,
                generation,
                receipt: value.clone(),
            }))
        }
        _ => Err("original replacement did not complete; video remains closed".into()),
    }
}
async fn save(path: &PathBuf, value: &Value) -> Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = tokio::fs::File::from_std(options.open(path)?);
    file.write_all(&serde_json::to_vec(value)?).await?;
    file.sync_all().await?;
    std::fs::File::open(path.parent().ok_or("private control directory required")?)?.sync_all()?;
    Ok(())
}
fn child_scope(
    scope: &CommandScope,
    root: &payloads::OperationUpdated,
    suffix: &str,
    revision: u64,
) -> Result<CommandScope> {
    let record = root
        .record_id
        .as_ref()
        .ok_or("original replacement record required")?
        .as_str();
    let operation = format!("{suffix}_{record}");
    if !video::id(&operation) {
        return Err("video control operation identity exceeds bound".into());
    }
    let mut result = scope.clone();
    result.operation_id = Some(operation);
    result.expected_revision = Some(revision);
    Ok(result)
}
async fn control(
    client: &mut CanonicalClient,
    scope: &CommandScope,
    kind: &str,
    input: Value,
    capabilities: &Value,
    own: &[selected::Binding],
    incoming: Option<&str>,
    output: &PathBuf,
    label: &str,
) -> Result<(Value, Option<selected::Binding>)> {
    // Save the complete original mutation before any possible submission. An
    // unknown result propagates; it is never resubmitted with a fresh ID.
    save(
        &output.with_extension(format!("{label}-intent.json")),
        &json!({"format":"connect/replacement-video-control/1",
        "kind":kind,"scope":scope,"payload":input}),
    )
    .await?;
    let request = client.send_value(kind, input, scope.clone()).await?;
    let until = tokio::time::Instant::now() + Duration::from_secs(8);
    let mut result = None;
    let mut received = None;
    tokio::time::timeout_at(until, async {
        for _ in 0..128 {
            let frame = client.receive().await?;
            if frame.scope(Scope::Reply).is_none()
                && matches!(
                    frame.kind(),
                    "operation.updated"
                        | "connection.ready"
                        | "stream.opened"
                        | "stream.closed"
                        | "connection.quality"
                        | "session.started"
                        | "session.ended"
                        | "session.participant.joined"
                        | "session.participant.left"
                        | "recording.vcon-ready"
                )
            {
                continue;
            }
            if !scope_matches(&frame, scope) {
                return Err("video control changed its original scope".into());
            }
            match frame.kind() {
                "connection.offer" if incoming.is_some() && received.is_none() => {
                    let payload = &frame.original()["payload"];
                    let binding = selected::subscription(payload, own, incoming.unwrap())?;
                    client
                        .answer_connection_offer(
                            &frame,
                            selected::answer(payload, capabilities)?,
                            scope,
                        )
                        .await?;
                    received = Some(binding);
                }
                "operation.updated"
                    if frame.scope(Scope::Reply) == Some(request.as_str()) && result.is_none() =>
                {
                    let payload = &frame.original()["payload"];
                    if payload["state"] != "completed"
                        || payload["operation_id"].as_str() != scope.operation_id.as_deref()
                    {
                        return Err("original video control did not complete".into());
                    }
                    result = Some(payload.clone());
                }
                "ack"
                    if frame.scope(Scope::Reply) == Some(request.as_str()) && result.is_some() =>
                {
                    if incoming.is_some() && received.is_none() {
                        return Err("video subscription omitted actual negotiation".into());
                    }
                    return Ok((result.take().unwrap(), received.take()));
                }
                _ => {
                    return Err("unexpected video control response; original is not retried".into())
                }
            }
        }
        Err("video control hint bound exceeded".into())
    })
    .await?
}
pub(super) async fn open(
    client: &mut CanonicalClient,
    scope: &CommandScope,
    root: &payloads::OperationUpdated,
    admission: &Admission,
    video: &video::Video,
    own: &[selected::Binding],
    capabilities: &Value,
    output: &PathBuf,
) -> Result<selected::Binding> {
    let opened_revision = admission
        .revision
        .checked_add(1)
        .ok_or("video gate stream revision exhausted")?;
    save(
        &output.with_extension("completed-root.json"),
        &admission.receipt,
    )
    .await?;
    let gate = child_scope(scope, root, "video_unmute", admission.revision)?;
    // Every publication was muted during resource preparation. Preserve Data
    // as a distinct reliable stream and open it only after the original root's
    // completed resource identities have been checked by completion().
    let publication = own
        .iter()
        .filter(|binding| binding.id == video.stream_id || binding.raw["kind"] == "data")
        .collect::<Vec<_>>();
    if !publication
        .iter()
        .any(|binding| binding.id == video.stream_id)
    {
        return Err("original video binding missing".into());
    }
    let (opened, none) = control(
        client,
        &gate,
        "connection.update",
        json!({"action":"unmute","streams":publication.iter().map(|binding| &binding.id).collect::<Vec<_>>()}),
        capabilities,
        own,
        None,
        output,
        "video-unmute",
    )
    .await?;
    if none.is_some() {
        return Err("publication gate returned unexpected subscription resources".into());
    }
    publication_gate(&opened["result"], admission, opened_revision, &publication)?;
    save(&output.with_extension("video-unmute-receipt.json"), &opened).await?;
    let subscription = child_scope(scope, root, "video_subscribe", opened_revision)?;
    let (subscribed, incoming) = control(
        client,
        &subscription,
        "stream.subscribe",
        json!({"subscriptions":[
        {"strm_id":video.receive_audio_stream_id},{"strm_id":video.receive_video_stream_id}]}),
        capabilities,
        own,
        Some(&video.receive_video_stream_id),
        output,
        "video-subscribe",
    )
    .await?;
    save(
        &output.with_extension("video-subscribe-receipt.json"),
        &subscribed,
    )
    .await?;
    incoming.ok_or_else(|| "actual incoming video binding missing".into())
}

fn publication_gate(
    result: &Value,
    admission: &Admission,
    revision: u64,
    expected: &[&selected::Binding],
) -> Result<()> {
    let gates = result["gate_streams"]
        .as_array()
        .filter(|gates| gates.len() == expected.len())
        .ok_or("publication gate omitted an original resource")?;
    if result["resources_changed"] != false
        || result["connection_generation"] != admission.generation
        || result["stream_revision"] != revision
    {
        return Err("publication gate changed original resource generation or revision".into());
    }
    for binding in expected {
        let found = gates
            .iter()
            .filter(|gate| gate["binding"]["stream_id"] == binding.id)
            .collect::<Vec<_>>();
        if found.len() != 1
            || found[0]["source_muted"] != false
            || found[0]["binding"]["stream_local_id"] != binding.handle
        {
            return Err("publication gate changed an original Data or video binding".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_root_and_publication_gate_cannot_drop_or_rebind_data() {
        let (command, root, mut own, pending) = fixture();
        own.push(selected::Binding {
            id: "data".into(),
            handle: 10,
            pt: None,
            raw: json!({"kind":"data"}),
        });
        let mut receipt = completed(pending.clone());
        assert!(observed(
            &receipt,
            &command,
            &root,
            "candidate",
            "target-device",
            &own,
            &mut 3
        )
        .is_err());
        receipt["result"]["readiness"]["streams"] = json!(["data", "camera", "audio"]);
        let admission = observed(
            &receipt,
            &command,
            &root,
            "candidate",
            "target-device",
            &own,
            &mut 3,
        )
        .unwrap()
        .unwrap();
        let publication = vec![&own[1], &own[2]];
        let result = json!({"resources_changed":false,"connection_generation":2,"stream_revision":6,
            "gate_streams":[{"source_muted":false,"binding":{"stream_id":"data","stream_local_id":10}},
                {"source_muted":false,"binding":{"stream_id":"camera","stream_local_id":8}}]});
        publication_gate(&result, &admission, 6, &publication).unwrap();
        for change in 0..6 {
            let mut wrong = result.clone();
            match change {
                0 => {
                    wrong["gate_streams"].as_array_mut().unwrap().remove(0);
                }
                1 => wrong["gate_streams"][0]["binding"]["stream_id"] = json!("camera"),
                2 => wrong["gate_streams"][0]["binding"]["stream_local_id"] = json!(11),
                3 => wrong["gate_streams"][0]["source_muted"] = json!(true),
                4 => wrong["connection_generation"] = json!(3),
                _ => wrong["stream_revision"] = json!(7),
            }
            assert!(publication_gate(&wrong, &admission, 6, &publication).is_err());
        }
        assert!(observed(
            &pending,
            &command,
            &root,
            "candidate",
            "target-device",
            &own,
            &mut 3
        )
        .unwrap()
        .is_none());
    }
    fn reply(kind: &str, payload: Value) -> ReplyBatch {
        ReplyBatch {
            request_id: "original-status-read".into(),
            frames: vec![Envelope::from_value(json!({"v":2,"id":"status-response",
                "ts":"2026-11-02T00:00:00Z","type":kind,
                "in_reply_to":"original-status-read","payload":payload}))
            .unwrap()],
        }
    }
    #[test]
    fn temporary_original_status_read_is_pending_and_never_admission() {
        let unavailable = reply(
            "error",
            json!({"code":503,"category":"storage",
            "reason":"storage_unavailable"}),
        );
        assert!(completion_read_pending(&unavailable).unwrap());
        let observed = reply("ack", json!({"details":{"reply_complete":true}}));
        assert!(!completion_read_pending(&observed).unwrap());
        // An ACK only permits inspection of the correlated original journal
        // fields; existing identity/resource checks still decide admission.
    }
    #[test]
    fn authority_resource_capacity_and_protocol_refusals_are_not_read_deferrals() {
        for code in [400, 401, 403, 404, 409, 429, 488, 500, 501] {
            let refused = reply(
                "error",
                json!({"code":code,"category":"server",
                "reason":"original_read_refused"}),
            );
            assert!(completion_read_pending(&refused).is_err(), "{code}");
        }
        let empty = ReplyBatch {
            request_id: "original-status-read".into(),
            frames: vec![],
        };
        assert!(completion_read_pending(&empty).is_err());
    }
    fn fixture() -> (
        TransitionCommand,
        payloads::OperationUpdated,
        Vec<selected::Binding>,
        Value,
    ) {
        let command = serde_json::from_value(json!({
            "operation_id":"original-move","conversation_id":"room","session_id":"session",
            "connection_id":"old","expected_revision":9,
            "intent":{"type":"connection.replace","endpoint_id":"endpoint",
                "candidate_connid":null,"retained_connid":"retained"}
        }))
        .unwrap();
        let value = json!({"operation_id":"original-move","record_id":"original-record",
            "command_type":"connection.replace","expected_revision":9,"revision":3,"state":"preparing"});
        let root = serde_json::from_value(value.clone()).unwrap();
        let own = ["audio", "camera"]
            .iter()
            .enumerate()
            .map(|(i, id)| selected::Binding {
                id: (*id).into(),
                handle: i as u16 + 7,
                pt: Some(96),
                raw: json!({}),
            })
            .collect();
        (command, root, own, value)
    }
    fn completed(mut value: Value) -> Value {
        value["revision"] = json!(4);
        value["state"] = json!("completed");
        value["result"] = json!({"connection_id":"candidate","device_id":"target-device",
            "replaces":"old","generation":2,"readiness":{"stream_revision":5,"streams":["camera","audio"]}});
        value
    }
    #[test]
    fn original_signed_journal_fields_are_required_and_pending_states_never_open_video() {
        let (command, root, own, value) = fixture();
        // The real public typed projection omits these journal extensions.
        let projected = serde_json::to_value(&root).unwrap();
        assert!(projected.get("command_type").is_none());
        assert!(observed(
            &projected,
            &command,
            &root,
            "candidate",
            "target-device",
            &own,
            &mut 3
        )
        .is_err());
        for state in ["accepted", "preparing", "ready", "committing"] {
            let mut value = value.clone();
            value["state"] = json!(state);
            assert!(observed(
                &value,
                &command,
                &root,
                "candidate",
                "target-device",
                &own,
                &mut 3
            )
            .unwrap()
            .is_none());
        }
        let value = completed(value);
        let admission = observed(
            &value,
            &command,
            &root,
            "candidate",
            "target-device",
            &own,
            &mut 3,
        )
        .unwrap()
        .unwrap();
        assert_eq!(admission.revision, 5);
        assert_eq!(admission.generation, 2);
        assert_eq!(admission.receipt, value);
    }
    #[test]
    fn changed_original_root_or_physical_resources_cannot_authorize_video() {
        let (command, root, own, value) = fixture();
        let value = completed(value);
        for field in 0..11 {
            let mut changed = value.clone();
            match field {
                0 => changed["record_id"] = json!("other-record"),
                1 => changed["operation_id"] = json!("other-operation"),
                2 => changed["command_type"] = json!("session.handoff"),
                3 => changed["expected_revision"] = json!(10),
                4 => changed["revision"] = json!(2),
                5 => changed["result"]["connection_id"] = json!("other-candidate"),
                6 => changed["result"]["device_id"] = json!("other-device"),
                7 => changed["result"]["replaces"] = json!("other-source"),
                8 => changed["result"]["readiness"]["streams"] = json!(["audio"]),
                9 => changed["result"]["readiness"]["streams"] = json!(["audio", "audio"]),
                _ => changed["state"] = json!("unknown"),
            }
            assert!(
                observed(
                    &changed,
                    &command,
                    &root,
                    "candidate",
                    "target-device",
                    &own,
                    &mut 3
                )
                .is_err(),
                "{field}"
            );
        }
    }
}
