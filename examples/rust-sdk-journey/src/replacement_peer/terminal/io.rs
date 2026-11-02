//! Actual public-client end IO adapter for the final physical peer.
//! The caller stops media at its original programme boundary before invoking it.
use super::end::{End, Progress};
use connect_client::canonical::{CanonicalClient, CommandScope, ReplyBatch, Scope};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};
use tokio::{io::AsyncWriteExt, time::Instant};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
async fn save(path: &Path, value: &Value) -> Result<()> {
    let body = serde_json::to_vec(value)?;
    if body.len() > 65536 {
        return Err("original end private record exceeds bound".into());
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = tokio::fs::File::from_std(options.open(path)?);
    file.write_all(&body).await?;
    file.sync_all().await?;
    std::fs::File::open(
        path.parent()
            .ok_or("private original end directory missing")?,
    )?
    .sync_all()?;
    Ok(())
}
fn now(start: Instant) -> Result<u64> {
    Ok(u64::try_from(
        Instant::now().saturating_duration_since(start).as_millis(),
    )?)
}
async fn batch(
    end: &mut End,
    reply: ReplyBatch,
    reading: bool,
    start: Instant,
    output: &Path,
    ordinal: usize,
) -> Result<Progress> {
    if reading {
        end.bind_read(&reply.request_id, now(start)?)?;
    } else {
        end.bind_submission(&reply.request_id, now(start)?)?;
    }
    let raw = reply
        .frames
        .iter()
        .map(|frame| frame.original().clone())
        .collect::<Vec<_>>();
    save(
        &output.with_extension(format!("owner-end-batch-{ordinal}.json")),
        &json!({"request_id":reply.request_id,"read_only":reading,"frames":raw}),
    )
    .await?;
    let mut result = None;
    for frame in &reply.frames {
        // request_value owns correlation; unrelated verified hints supply no end
        // fact. Keep the raw batch for an independent post-join audit.
        if frame.scope(Scope::Reply) != Some(reply.request_id.as_str()) {
            continue;
        }
        if result.is_some() {
            return Err("original end batch continued after its terminal response".into());
        }
        result = end.observe(frame.original(), now(start)?)?;
    }
    result.ok_or_else(|| "original end batch omitted its correlated completion or status".into())
}
/// Called once by the actual final physical owner after the unchanged62s media
/// programme. The parent supplies an already durable distinct end operation ID.
/// Existing current peer/signing/Store authorization is enforced by the client.
pub async fn run(
    client: &mut CanonicalClient,
    current: &CommandScope,
    operation: &str,
    start: Instant,
    output: &Path,
) -> Result<Value> {
    let conversation = current
        .conversation_id
        .as_deref()
        .ok_or("original owner conversation missing")?;
    let session = current
        .session_id
        .as_deref()
        .ok_or("original owner session missing")?;
    let connection = current
        .connection_id
        .as_deref()
        .ok_or("original owner connection missing")?;
    if current.operation_id.as_deref() == Some(operation) {
        return Err("final end reused the replacement operation key".into());
    }
    let cutoff = start + Duration::from_millis(67000);
    let mut end = End::new([conversation, session, connection], operation, 62000)?;
    let original = end.intent();
    // Persist complete original bytes before possible IO. Saving them is not
    // media permission and cannot change the original five-second cutoff.
    save(&output.with_extension("owner-end-intent.json"), &original).await?;
    end.begin(true, now(start)?)?;
    client.start_signaling_reader()?;
    let mutation = CommandScope {
        conversation_id: Some(conversation.into()),
        session_id: Some(session.into()),
        connection_id: Some(connection.into()),
        operation_id: Some(operation.into()),
        ..Default::default()
    };
    let attempted = tokio::time::timeout_at(
        cutoff,
        client.request_value("connection.end", original["payload"].clone(), mutation),
    )
    .await;
    let mut progress = match attempted {
        Ok(Ok(reply)) => batch(&mut end, reply, false, start, output, 0).await?,
        Ok(Err(_)) => {
            end.submission_unknown(now(start)?)?;
            Progress::Unknown
        }
        Err(_) => {
            return Err("original end submission exceeded its five-second cutoff; no replay".into())
        }
    };
    for ordinal in 1..=128 {
        match progress {
            Progress::Completed => {
                if !end.completed_with_ack() {
                    return Err("current owner end omitted its own ACK".into());
                }
                let result = json!({"format":"connect/physical-owner-end-receipt/1","original":original,
                    "completed":true,"original_operation_id_preserved":true,"mutation_submissions":1,
                    "elapsed_since_programme_end_ms":now(start)?.saturating_sub(62000),
                    "control_only_after_programme":true,"media_programme_extended":false});
                save(&output.with_extension("owner-end-receipt.json"), &result).await?;
                if now(start)? >= 67000 {
                    return Err("original end durable receipt exceeded teardown cutoff".into());
                }
                return Ok(result);
            }
            Progress::Refused => {
                return Err("original physical owner end was refused; no replay".into())
            }
            Progress::Pending | Progress::Unknown | Progress::DeferredRead => {}
        }
        let read = CommandScope {
            conversation_id: Some(conversation.into()),
            session_id: Some(session.into()),
            connection_id: Some(connection.into()),
            ..Default::default()
        };
        let reply = tokio::time::timeout_at(
            cutoff,
            client.request_value("operation.get", end.status_payload(), read),
        )
        .await??;
        progress = batch(&mut end, reply, true, start, output, ordinal).await?;
        if progress != Progress::Completed {
            tokio::time::timeout_at(cutoff, tokio::time::sleep(Duration::from_millis(50))).await?;
        }
    }
    Err("original physical end metadata observation bound exceeded".into())
}
