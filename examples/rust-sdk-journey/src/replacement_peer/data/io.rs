//! One actual original Data exchange alongside the unchanged media programme.
use super::{
    outgoing::{Outgoing, Progress},
    ExpectedData,
};
use connect_client::canonical::{CanonicalClient, CommandScope, Scope};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{path::Path, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    time::Instant,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in super::super) struct Original {
    pub operation_id: String,
    pub message_id: String,
    pub stream_id: String,
    pub from_participant: String,
    pub to_participant: String,
    pub body_base64: String,
}
impl Original {
    fn expected(&self, scope: [&str; 3]) -> Result<ExpectedData> {
        Ok(ExpectedData::new(
            scope,
            [&self.operation_id, &self.message_id, &self.stream_id],
            [&self.from_participant, &self.to_participant],
            &self.body_base64,
        )?)
    }
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in super::super) struct Contract {
    pub incoming: Original,
    pub outgoing: Original,
}
impl Contract {
    pub(in super::super) fn validate(&self) -> Result<()> {
        self.incoming
            .expected(["conversation", "session", "candidate"])?;
        self.outgoing
            .expected(["conversation", "session", "candidate"])?;
        if self.incoming.to_participant != self.outgoing.from_participant
            || self.incoming.operation_id == self.outgoing.operation_id
            || self.incoming.message_id == self.outgoing.message_id
            || self.incoming.stream_id == self.outgoing.stream_id
            || self.incoming.body_base64 == self.outgoing.body_base64
        {
            return Err("physical Data originals changed their distinct ring contract".into());
        }
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    format: String,
    root_operation_id: String,
    connection_id: String,
    incoming_operation_id: String,
    outgoing_operation_id: String,
    armed_programme_ms: u64,
    cutoff_programme_ms: u64,
}
impl Start {
    fn validate(&self, root: &str, connection: &str, contract: &Contract, now: u64) -> Result<()> {
        if self.format != "connect/physical-data-start/1"
            || self.root_operation_id != root
            || self.connection_id != connection
            || self.incoming_operation_id != contract.incoming.operation_id
            || self.outgoing_operation_id != contract.outgoing.operation_id
            || self.armed_programme_ms > now.saturating_add(10)
            || self.cutoff_programme_ms <= self.armed_programme_ms
            || self.cutoff_programme_ms > self.armed_programme_ms.saturating_add(8000)
            || self.cutoff_programme_ms > 62000
            || self.cutoff_programme_ms <= now
            || self.cutoff_programme_ms - now > 8000
        {
            return Err("physical Data marker changed original identities or cutoff".into());
        }
        Ok(())
    }
}
async fn marker(path: &Path) -> Result<Option<Start>> {
    let info = match tokio::fs::symlink_metadata(path).await {
        Ok(info) => info,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !info.is_file() || info.len() > 2048 {
        return Err("physical Data marker exceeds bound".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if info.permissions().mode() & 0o077 != 0 {
            return Err("physical Data marker is not private".into());
        }
    }
    let mut bytes = vec![];
    tokio::fs::File::open(path)
        .await?
        .take(2049)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > 2048 {
        return Err("physical Data marker read exceeds bound".into());
    }
    Ok(Some(serde_json::from_slice(&bytes)?))
}
async fn save(path: &Path, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("physical Data private record exceeds bound".into());
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let temporary = path.with_file_name(format!(
        "{}.pending",
        path.file_name()
            .ok_or("physical Data file name missing")?
            .to_string_lossy()
    ));
    let mut file = tokio::fs::File::from_std(options.open(&temporary)?);
    file.write_all(&bytes).await?;
    file.sync_all().await?;
    drop(file);
    tokio::fs::hard_link(&temporary, path).await?;
    tokio::fs::remove_file(temporary).await?;
    std::fs::File::open(path.parent().ok_or("physical Data directory missing")?)?.sync_all()?;
    Ok(())
}
pub(in super::super) struct Exchange {
    contract: Contract,
    incoming: ExpectedData,
    outgoing: Outgoing,
    scope: CommandScope,
    root: String,
    until: Option<Instant>,
    cutoff_ms: u64,
    response_until: Option<Instant>,
    next_read: Instant,
    wires: Vec<Value>,
    wire_bytes: usize,
    receipt: Option<Value>,
}
impl Exchange {
    pub(in super::super) fn new(
        contract: Contract,
        scope: &CommandScope,
        root: &str,
        participant: &str,
    ) -> Result<Self> {
        contract.validate()?;
        let connection = scope
            .connection_id
            .as_deref()
            .ok_or("physical Data connection missing")?;
        let conversation = scope
            .conversation_id
            .as_deref()
            .ok_or("physical Data conversation missing")?;
        let session = scope
            .session_id
            .as_deref()
            .ok_or("physical Data Session missing")?;
        if contract.incoming.to_participant != participant
            || contract.outgoing.from_participant != participant
            || [
                &contract.incoming.operation_id,
                &contract.outgoing.operation_id,
            ]
            .iter()
            .any(|operation| operation.as_str() == root)
        {
            return Err("physical Data changed its actual admitted participant or root key".into());
        }
        Ok(Self {
            incoming: contract
                .incoming
                .expected([conversation, session, connection])?,
            outgoing: Outgoing::new(contract.outgoing.expected([
                conversation,
                session,
                connection,
            ])?),
            contract,
            scope: CommandScope {
                operation_id: None,
                expected_revision: None,
                ..scope.clone()
            },
            root: root.into(),
            until: None,
            cutoff_ms: 0,
            response_until: None,
            next_read: Instant::now(),
            wires: vec![],
            wire_bytes: 0,
            receipt: None,
        })
    }
    pub(in super::super) fn completed(&self) -> bool {
        self.receipt.is_some()
    }
    pub(in super::super) fn receipt(&self) -> Option<&Value> {
        self.receipt.as_ref()
    }
    /// The single owned signaling reader was started before entering media.
    /// Zero-duration foreground waits inspect already queued verified frames;
    /// cancellation cannot discard a partially received wire frame.
    pub(in super::super) async fn tick(
        &mut self,
        client: &mut CanonicalClient,
        root_completed: bool,
        programme_start: Instant,
        output: &Path,
    ) -> Result<()> {
        if self.until.is_none() {
            if let Some(armed) = marker(&output.with_extension("data-start.json")).await? {
                if !root_completed {
                    return Err("physical Data cannot start before original root completion".into());
                }
                let elapsed = u64::try_from(programme_start.elapsed().as_millis())?;
                armed.validate(
                    &self.root,
                    self.scope.connection_id.as_deref().unwrap(),
                    &self.contract,
                    elapsed,
                )?;
                self.cutoff_ms = armed.cutoff_programme_ms;
                let until = programme_start + Duration::from_millis(self.cutoff_ms);
                self.until = Some(until);
                let original = self.outgoing.intent();
                save(&output.with_extension("data-original-send.json"), &original).await?;
                self.outgoing.begin(root_completed)?;
                let scope = CommandScope {
                    operation_id: Some(self.contract.outgoing.operation_id.clone()),
                    ..self.scope.clone()
                };
                match tokio::time::timeout_at(
                    until,
                    client.send_value("message.send", original["payload"].clone(), scope),
                )
                .await
                {
                    Ok(Ok(request)) => self.outgoing.bind_submission(&request)?,
                    Ok(Err(_)) => self.outgoing.submission_unknown()?,
                    Err(_) => {
                        return Err("original physical Data send exceeded cutoff; no replay".into())
                    }
                }
                self.response_until = Some(Instant::now() + Duration::from_millis(250));
            }
        }
        for _ in 0..16 {
            let frame = match tokio::time::timeout(Duration::ZERO, client.receive()).await {
                Ok(frame) => frame?,
                Err(_) => break,
            };
            let reply = frame.scope(Scope::Reply);
            if frame.kind() == "message.send" {
                if self.until.is_none() {
                    return Err("physical Data arrived before its original exchange began".into());
                }
                self.incoming.observe(frame.original(), root_completed)?;
            } else if reply.is_some() && reply == self.outgoing.read_id() {
                if self.outgoing.observe_read(frame.original())? == Progress::Refused {
                    return Err("original physical Data status refused; no replay".into());
                }
            } else if self.outgoing.submission_pending()
                && reply.is_some()
                && reply == self.outgoing.submission_id()
            {
                if self.outgoing.observe_submission(frame.original())? == Progress::Refused {
                    return Err("original physical Data command refused; no replay".into());
                }
            } else {
                // Unrelated verified hints and late replies grant no acceptance.
                continue;
            }
            self.wire_bytes += serde_json::to_vec(frame.original())?.len();
            if self.wires.len() >= 512 || self.wire_bytes > 1024 * 1024 {
                return Err("physical Data verified wire observation exceeds bound".into());
            }
            self.wires.push(frame.original().clone());
        }
        let Some(until) = self.until else {
            return Ok(());
        };
        if self.completed() {
            return Ok(());
        }
        if Instant::now() >= until {
            return Err("original physical Data exchange exceeded its fixed cutoff".into());
        }
        if self.incoming.received() && self.outgoing.accepted_core() {
            let receipt = json!({"format":"connect/physical-data-exchange/1","root_operation_id":self.root,
                "conversation_id":self.scope.conversation_id,"session_id":self.scope.session_id,"connection_id":self.scope.connection_id,
                "incoming_operation_id":self.contract.incoming.operation_id,"incoming_message_id":self.contract.incoming.message_id,
                "outgoing_operation_id":self.contract.outgoing.operation_id,"outgoing_message_id":self.contract.outgoing.message_id,
                "incoming_exact_body_base64":self.contract.incoming.body_base64,
                "incoming_received":true,"outgoing_core_queue_admission":true,"outgoing_mutation_submissions":1,
                "outgoing_recovered_read_only":self.outgoing.recovered_read_only(),"original_root_completed_observed":root_completed,
                "cutoff_programme_ms":self.cutoff_ms,"observed_programme_ms":programme_start.elapsed().as_millis(),
                "remote_delivery_inferred":false});
            save(
                &output.with_extension("data-wire.json"),
                &json!({"frames":self.wires}),
            )
            .await?;
            save(&output.with_extension("data-receipt.json"), &receipt).await?;
            if Instant::now() >= until {
                return Err("physical Data durable receipt exceeded original cutoff".into());
            }
            self.receipt = Some(receipt);
            return Ok(());
        }
        if self
            .response_until
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            if self.outgoing.submission_pending() {
                self.outgoing.submission_unknown()?;
            }
            if self.outgoing.read_id().is_some() {
                self.outgoing.read_unknown()?;
            }
            self.response_until = None;
        }
        if self.outgoing.can_read() && Instant::now() >= self.next_read {
            // New physical read correlation, same original logical operation.
            let request = tokio::time::timeout_at(
                until,
                client.send_value(
                    "operation.get",
                    self.outgoing.status_payload(),
                    self.scope.clone(),
                ),
            )
            .await??;
            self.outgoing.bind_read(&request)?;
            self.response_until = Some(Instant::now() + Duration::from_millis(250));
            self.next_read = Instant::now() + Duration::from_millis(50);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};
    fn contract() -> Contract {
        Contract {
            incoming: Original {
                operation_id: "incoming-operation".into(),
                message_id: "incoming-message".into(),
                stream_id: "incoming-stream".into(),
                from_participant: "upstream".into(),
                to_participant: "local".into(),
                body_base64: STANDARD.encode(vec![7; 65536]),
            },
            outgoing: Original {
                operation_id: "outgoing-operation".into(),
                message_id: "outgoing-message".into(),
                stream_id: "outgoing-stream".into(),
                from_participant: "local".into(),
                to_participant: "downstream".into(),
                body_base64: STANDARD.encode(vec![8; 65536]),
            },
        }
    }
    #[test]
    fn original_marker_binds_actual_root_connection_and_fixed_eight_second_programme_cutoff() {
        let original = json!({"format":"connect/physical-data-start/1","root_operation_id":"root","connection_id":"candidate",
            "incoming_operation_id":"incoming-operation","outgoing_operation_id":"outgoing-operation","armed_programme_ms":2000,"cutoff_programme_ms":10000});
        serde_json::from_value::<Start>(original.clone())
            .unwrap()
            .validate("root", "candidate", &contract(), 2000)
            .unwrap();
        for (field, value) in [
            ("root_operation_id", json!("foreign")),
            ("connection_id", json!("foreign")),
            ("incoming_operation_id", json!("foreign")),
            ("outgoing_operation_id", json!("foreign")),
            ("armed_programme_ms", json!(2100)),
            ("cutoff_programme_ms", json!(10001)),
            ("cutoff_programme_ms", json!(2000)),
            ("cutoff_programme_ms", json!(62001)),
        ] {
            let mut bad = original.clone();
            bad[field] = value;
            assert!(
                serde_json::from_value::<Start>(bad)
                    .unwrap()
                    .validate("root", "candidate", &contract(), 2000)
                    .is_err(),
                "{field}"
            );
        }
        assert!(serde_json::from_value::<Start>(original)
            .unwrap()
            .validate("root", "candidate", &contract(), 10000)
            .is_err());
    }
    #[test]
    fn actual_admitted_participant_and_distinct_root_keys_are_required_before_io() {
        let scope = CommandScope {
            conversation_id: Some("room".into()),
            session_id: Some("session".into()),
            connection_id: Some("candidate".into()),
            ..Default::default()
        };
        assert!(Exchange::new(contract(), &scope, "root", "local").is_ok());
        let mut prepared = scope.clone();
        prepared.operation_id = Some("root".into());
        prepared.expected_revision = Some(0);
        let exchange = Exchange::new(contract(), &prepared, "root", "local").unwrap();
        assert!(exchange.scope.operation_id.is_none());
        assert!(exchange.scope.expected_revision.is_none());
        assert!(Exchange::new(contract(), &scope, "root", "foreign").is_err());
        assert!(Exchange::new(contract(), &scope, "incoming-operation", "local").is_err());
        for field in 0..4 {
            let mut bad = contract();
            match field {
                0 => bad.outgoing.from_participant = "foreign".into(),
                1 => bad.outgoing.operation_id = bad.incoming.operation_id.clone(),
                2 => bad.outgoing.body_base64 = bad.incoming.body_base64.clone(),
                _ => bad.incoming.body_base64 = STANDARD.encode(vec![7; 65535]),
            };
            assert!(bad.validate().is_err());
        }
    }
}
