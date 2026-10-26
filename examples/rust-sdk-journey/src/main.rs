//! An independent public-crate consumer driven by isolated production fixtures.
use connect_client::sdk::{self, Client, ClientConfig, MutationOptions, RequestOptions};
use connect_client::transitions::TransitionCommand;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{path::PathBuf, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
mod media;
mod transition;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum Journey {
    Media {
        publisher: ClientConfig,
        subscriber: ClientConfig,
        output: PathBuf,
    },
    Transition {
        config: ClientConfig,
        command: TransitionCommand,
        consultation: Option<Value>,
    },
    Recover {
        config: ClientConfig,
        command: TransitionCommand,
        reference: Value,
    },
}
fn options() -> RequestOptions {
    RequestOptions::new(Duration::from_secs(35)).expect("fixed bounded timeout")
}
fn mutation(id: &str) -> sdk::Result<MutationOptions> {
    Ok(MutationOptions {
        operation_id: id.parse()?,
        request: options(),
        extensions: None,
    })
}
async fn emit(value: Value) -> Result<()> {
    let mut line = serde_json::to_vec(&value)?;
    line.push(b'\n');
    let mut out = tokio::io::stdout();
    out.write_all(&line).await?;
    out.flush().await?;
    Ok(())
}
async fn proceed() -> Result<()> {
    let mut line = String::new();
    tokio::io::BufReader::new(tokio::io::stdin())
        .read_line(&mut line)
        .await?;
    if line.trim() != "continue" {
        return Err("missing fixture continuation".into());
    }
    Ok(())
}
#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("external public SDK journey failed: {e}");
        std::process::exit(1);
    }
}
async fn run() -> Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("expected private input file")?;
    // Load the complete, durably saved original case before any network IO.
    let input: Journey = serde_json::from_slice(&std::fs::read(path)?)?;
    match input {
        Journey::Media {
            publisher,
            subscriber,
            output,
        } => media::run(publisher, subscriber, output).await,
        Journey::Transition {
            config,
            command,
            consultation,
        } => transition::run(config, command, consultation).await,
        Journey::Recover {
            config,
            command,
            reference,
        } => transition::recover(config, command, reference).await,
    }
}
