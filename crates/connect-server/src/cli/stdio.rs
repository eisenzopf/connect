//! Bounded canonical JSON-lines control, or an explicit named application API.
use super::{client_errors::client_error, request::canonical_exchange};
use connect_client::{
    application::{ApplicationClient, ApplicationClientConfig},
    canonical::{CanonicalClient, CanonicalClientConfig, Command, CommandScope},
};
use connect_protocol::{
    tasks::{TaskCommand, TaskOutcome},
    Error, Operation, Outcome, PROFILE,
};
use rvoip_uctp::v2::payloads::Payload;
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CanonicalInput {
    id: String,
    command: Payload,
    #[serde(default)]
    scope: CommandScope,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplicationInput {
    id: String,
    operation: Operation,
    #[serde(default)]
    task_binding: Option<String>,
}
enum Client {
    Canonical(CanonicalClient),
    Application(ApplicationClient),
}
enum Intent {
    Canonical(Command),
    Application(Operation, Option<String>),
}

/// One fixed authority and selected API for the complete helper lifetime.
/// Startup refusal is correlated with admitted input. Neither failure nor an
/// edited configuration file changes credentials, profile or transport.
pub async fn serve(path: &Path, application: bool) -> anyhow::Result<()> {
    let mut client = if application {
        match ApplicationClientConfig::load(path) {
            Ok(config) => ApplicationClient::open(config)
                .await
                .map(Client::Application)
                .map_err(client_error),
            Err(error) => Err(client_error(error)),
        }
    } else {
        match CanonicalClientConfig::load(path) {
            Ok(config) => CanonicalClient::open(config)
                .await
                .map(Client::Canonical)
                .map_err(client_error),
            Err(error) => Err(client_error(error)),
        }
    };
    let result = bridge(&mut client, application).await;
    if let Ok(Client::Canonical(client)) = client {
        // A verified reply remains valid even if the final drain times out.
        let _ = client.shutdown().await;
    }
    result
}

async fn bridge(client: &mut Result<Client, Error>, application: bool) -> anyhow::Result<()> {
    let mut input = tokio::io::BufReader::new(tokio::io::stdin());
    let mut output = tokio::io::stdout();
    let mut fenced: Option<Error> = None;
    loop {
        let mut line = Vec::new();
        loop {
            match input.read_u8().await {
                Ok(b'\n') => break,
                Ok(byte) => {
                    line.push(byte);
                    anyhow::ensure!(line.len() <= 65_536, "JSON line exceeds 64 KiB");
                }
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    if line.is_empty() {
                        return Ok(());
                    }
                    break;
                }
                Err(e) => return Err(e.into()),
            }
        }
        let (id, intent) = if application {
            let v: ApplicationInput = serde_json::from_slice(&line)
                .map_err(|_| anyhow::anyhow!("Invalid application JSON-lines request"))?;
            (v.id, Intent::Application(v.operation, v.task_binding))
        } else {
            let v: CanonicalInput = serde_json::from_slice(&line)
                .map_err(|_| anyhow::anyhow!("Invalid canonical JSON-lines request"))?;
            (
                v.id,
                Intent::Canonical(Command {
                    command: v.command,
                    scope: v.scope,
                }),
            )
        };
        anyhow::ensure!(!id.is_empty() && id.len() <= 128, "Invalid exchange ID");
        let result = if let Some(error) = &fenced {
            Err(error.clone())
        } else {
            match client {
                Ok(Client::Canonical(client)) => match intent {
                    Intent::Canonical(command) => canonical_exchange(client, command).await,
                    _ => unreachable!("input is parsed for the selected mode"),
                },
                Ok(Client::Application(client)) => match intent {
                    Intent::Application(operation, binding) => {
                        application_exchange(client, operation, binding).await
                    }
                    _ => unreachable!("input is parsed for the selected mode"),
                },
                Err(error) => Err(error.clone()),
            }
        };
        if let Err(error) = &result {
            if matches!(error.code.as_str(), "transport-uncertain" | "protocol") {
                fenced = Some(error.clone());
            }
        }
        let bytes = serde_json::to_vec(
            &serde_json::json!({"id":id,"response":{"profile":PROFILE,"result":result}}),
        )?;
        anyhow::ensure!(
            bytes.len() <= 2 * 1024 * 1024,
            "JSON response exceeds 2 MiB"
        );
        output.write_all(&bytes).await?;
        output.write_all(b"\n").await?;
        output.flush().await?;
    }
}

async fn application_exchange(
    client: &mut ApplicationClient,
    operation: Operation,
    task_binding: Option<String>,
) -> Result<Value, Error> {
    let outcome = if let Some(task_id) = task_binding {
        let exact_get = matches!(&operation, Operation::Task { request }
            if request.task_profile == connect_protocol::tasks::TASK_PROFILE
            && matches!(&request.command, TaskCommand::Get { task_id: id } if id == &task_id));
        if !exact_get {
            return Err(Error::new(
                "invalid-request",
                "Task bootstrap must read its exact bound task",
            ));
        }
        Outcome::Task(Box::new(TaskOutcome::Record(Box::new(
            client
                .negotiate_task(&task_id)
                .await
                .map_err(client_error)?,
        ))))
    } else {
        client
            .execute(operation.try_into().map_err(client_error)?)
            .await
            .map_err(client_error)?
    };
    serde_json::to_value(outcome)
        .map_err(|_| Error::new("invalid-response", "Application result encoding failed"))
}
