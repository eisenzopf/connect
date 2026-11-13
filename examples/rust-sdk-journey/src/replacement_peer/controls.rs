//! Original video transfer observation and separately authorized Data controls.
use super::*;
use connect_client::{canonical::ReplyBatch, ClientError};
mod exchange;
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
    authoritative_pending: &mut bool,
) -> Result<Option<Admission>> {
    *authoritative_pending = false;
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
    let value = &updates[0].original()["payload"];
    let result = observed(
        value,
        command,
        root,
        candidate,
        device,
        own,
        minimum_revision,
    )?;
    *authoritative_pending =
        result.is_none() && matches!(value["state"].as_str(), Some("accepted" | "preparing"));
    Ok(result)
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
        // Uncertainty is not admission and cannot authorize another mutation.
        // Keep this original physical owner and observe only the same root
        // under the caller's unchanged programme/poll/cancellation bounds.
        Some("accepted" | "preparing" | "ready" | "committing" | "unknown") => Ok(None),
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
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options.open(path).await?;
    file.write_all(&serde_json::to_vec(value)?).await?;
    file.sync_all().await?;
    #[cfg(test)]
    durability_tests::before_directory_sync(path)?;
    tokio::fs::File::open(path.parent().ok_or("private control directory required")?)
        .await?
        .sync_all()
        .await?;
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
    incoming: Option<&video::Video>,
    output: &PathBuf,
    label: &str,
) -> Result<(Value, Option<Vec<selected::Binding>>)> {
    // Save the complete original mutation before any possible submission. An
    // unknown result propagates; it is never resubmitted with a fresh ID.
    save(
        &output.with_extension(format!("{label}-intent.json")),
        &json!({"format":"connect/replacement-video-control/1",
        "kind":kind,"scope":scope,"payload":input}),
    )
    .await?;
    let until = tokio::time::Instant::now() + Duration::from_secs(8);
    let (result, observed) = exchange::run(
        client,
        scope,
        kind,
        input,
        capabilities,
        own,
        incoming,
        until,
    )
    .await;
    if result.is_err() {
        // Preserve the actual original receipt on failure, without turning a
        // diagnostic write failure into another operation or masking the error.
        if let Some(receipt) = observed {
            let _ = save(
                &output.with_extension(format!("{label}-observed.json")),
                &receipt,
            )
            .await;
        }
    }
    result
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
) -> Result<Vec<selected::Binding>> {
    save(
        &output.with_extension("completed-root.json"),
        &admission.receipt,
    )
    .await?;
    // The exact camera and incoming receiver were negotiated before activation.
    // No video gate or subscription mutation is issued after completion.
    if !own
        .iter()
        .any(|b| b.id == video.stream_id && b.raw["direction"] == "recvonly")
        || !own
            .iter()
            .any(|b| b.id == video.receive_video_stream_id && b.raw["direction"] == "sendonly")
    {
        return Err("prepared video transfer lost its directional resources".into());
    }
    let publication = own
        .iter()
        .filter(|b| b.raw["kind"] == "data")
        .collect::<Vec<_>>();
    let mut revision = admission.revision;
    if !publication.is_empty() {
        let gate = child_scope(scope, root, "data_unmute", revision)?;
        revision = revision
            .checked_add(1)
            .ok_or("Data gate revision exhausted")?;
        let (opened, none) = control(client, &gate, "connection.update",
            json!({"action":"unmute","streams":publication.iter().map(|b| &b.id).collect::<Vec<_>>()}),
            capabilities, own, None, output, "data-unmute").await?;
        if none.is_some() {
            return Err("Data gate unexpectedly renegotiated resources".into());
        }
        publication_gate(&opened["result"], admission, revision, &publication)?;
        save(&output.with_extension("data-unmute-receipt.json"), &opened).await?;
    }
    if let Some(data) = &video.receive_data_stream_id {
        let subscription = child_scope(scope, root, "data_subscribe", revision)?;
        let (subscribed, incoming) = control(
            client,
            &subscription,
            "stream.subscribe",
            json!({"subscriptions":[{"strm_id":data}]}),
            capabilities,
            own,
            Some(video),
            output,
            "data-subscribe",
        )
        .await?;
        save(
            &output.with_extension("data-subscribe-receipt.json"),
            &subscribed,
        )
        .await?;
        return incoming.ok_or_else(|| "actual incoming Data binding missing".into());
    }
    Ok(Vec::new())
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
    fn unknown_original_root_stays_closed_until_same_root_completes() {
        let (command, root, own, pending) = fixture();
        let mut unknown = completed(pending.clone());
        unknown["state"] = json!("unknown");
        let mut revision = 3;
        assert!(observed(
            &unknown,
            &command,
            &root,
            "candidate",
            "target-device",
            &own,
            &mut revision
        )
        .unwrap()
        .is_none());
        assert_eq!(revision, 4);
        unknown["revision"] = json!(5);
        assert!(observed(
            &unknown,
            &command,
            &root,
            "candidate",
            "target-device",
            &own,
            &mut revision
        )
        .unwrap()
        .is_none());
        let mut wrong = unknown.clone();
        wrong["record_id"] = json!("different-root");
        assert!(observed(
            &wrong,
            &command,
            &root,
            "candidate",
            "target-device",
            &own,
            &mut revision
        )
        .is_err());
        for state in ["failed", "cancelled"] {
            let mut terminal = unknown.clone();
            terminal["state"] = json!(state);
            assert!(observed(
                &terminal,
                &command,
                &root,
                "candidate",
                "target-device",
                &own,
                &mut revision
            )
            .is_err());
        }
        let mut completed = completed(pending);
        assert!(observed(
            &completed,
            &command,
            &root,
            "candidate",
            "target-device",
            &own,
            &mut revision
        )
        .is_err());
        completed["revision"] = json!(6);
        let admitted = observed(
            &completed,
            &command,
            &root,
            "candidate",
            "target-device",
            &own,
            &mut revision,
        )
        .unwrap()
        .unwrap();
        assert_eq!(admitted.generation, 2);
        assert_eq!(revision, 6);
    }
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
        // The current typed projection preserves journal identity. Missing
        // original fields must still be refused, never inferred from the root.
        let projected = serde_json::to_value(&root).unwrap();
        assert_eq!(projected["command_type"], "connection.replace");
        assert!(observed(
            &projected,
            &command,
            &root,
            "candidate",
            "target-device",
            &own,
            &mut 3
        )
        .unwrap()
        .is_none());
        let mut missing = projected;
        missing.as_object_mut().unwrap().remove("command_type");
        assert!(observed(
            &missing,
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
                _ => changed["state"] = json!("invalid-state"),
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

#[cfg(test)]
#[path = "controls/durability_tests.rs"]
mod durability_tests;
