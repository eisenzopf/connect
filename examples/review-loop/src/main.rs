//! Example binaries are independent clients, not privileged server commands.
use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use connect_review_loop::{store::WorkStore, worker};
use std::path::PathBuf;
#[derive(Parser)]
#[command(
    name = "connect-example",
    version,
    about = "Deterministic native app and agent review loop"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Reviews(Run),
    Agent(Run),
    Inspect {
        #[arg(long)]
        state: PathBuf,
    },
}
#[derive(Args)]
struct Run {
    #[arg(long)]
    config: PathBuf,
    #[arg(long)]
    state: Option<PathBuf>,
    #[arg(long,value_parser=clap::value_parser!(u64).range(1..=60000))]
    run_for_ms: Option<u64>,
}
#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Inspect { state } => {
            println!("{}", serde_json::to_string(&WorkStore::inspect(&state)?)?)
        }
        Command::Reviews(run) => execute(run, false).await?,
        Command::Agent(run) => execute(run, true).await?,
    }
    Ok(())
}
async fn execute(run: Run, agent: bool) -> Result<()> {
    let state = run
        .state
        .unwrap_or_else(|| run.config.with_extension("work.sqlite3"));
    worker::run(&run.config, &state, agent, run.run_for_ms).await
}
