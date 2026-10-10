//! Example binaries are independent clients, not privileged server commands.
use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use connect_review_loop::task_runner::{self, RunOptions, TaskStore};
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
    /// Explicit task-scoped runner with durable app state and reconnects.
    Task(TaskRun),
    /// Safe inspection of the runner's app-owned database.
    TaskInspect {
        #[arg(long)]
        state: PathBuf,
    },
    Inspect {
        #[arg(long)]
        state: PathBuf,
    },
}
#[derive(Args)]
struct TaskRun {
    #[arg(long)]
    controller_config: PathBuf,
    #[arg(long)]
    invocation_config: PathBuf,
    #[arg(long)]
    task_id: String,
    #[arg(long)]
    state: PathBuf,
    #[arg(long,value_parser=clap::value_parser!(u64).range(1..=60000))]
    run_for_ms: Option<u64>,
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
        Command::Task(run) => {
            let stop = tokio_util::sync::CancellationToken::new();
            let execution = task_runner::run(
                RunOptions {
                    controller_config: run.controller_config,
                    invocation_config: run.invocation_config,
                    task_id: run.task_id,
                    state: run.state,
                },
                stop.clone(),
            );
            tokio::pin!(execution);
            let signal = async {
                if let Some(ms) = run.run_for_ms {
                    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                    Ok(())
                } else {
                    tokio::signal::ctrl_c().await
                }
            };
            tokio::select! {
                result = &mut execution => result?,
                result = signal => { result?; stop.cancel(); execution.await?; }
            }
        }
        Command::TaskInspect { state } => {
            println!("{}", serde_json::to_string(&TaskStore::inspect(&state)?)?)
        }
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
