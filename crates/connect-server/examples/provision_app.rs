#![recursion_limit = "256"]
//! Trusted setup and canonical server embedding; no private Store/Core access.
#[path = "../../../examples/provision-app/setup.rs"]
mod setup;
use anyhow::{bail, Context, Result};
use std::path::Path;

#[tokio::main]
async fn main() -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    match args.get(1).map(String::as_str) {
        Some("setup") if args.len()==5 => {
            let config=connect_client::application::ApplicationClientConfig::load(Path::new(&args[2]))?;
            let plan:setup::SetupPlan=serde_json::from_slice(&std::fs::read(&args[3])?)?;
            let mut client=connect_client::application::ApplicationClient::open(config).await?;
            setup::setup(&mut client,plan,Path::new(&args[4])).await
                .context("Setup did not complete; retain the private journal and explicitly rerun the same plan")?;
            println!("Setup complete. Private enrollment output and public contacts are in the selected state directory.");
        }
        Some("serve") if args.len()==3 => {
            let config=connect_server::canonical::CanonicalServerConfig::load(Path::new(&args[2]))?;
            let server=connect_server::canonical::RunningCanonicalServer::start(config).await?;
            let mut terminate=tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            tokio::select! { result=tokio::signal::ctrl_c()=>result?, _=terminate.recv()=>{} }
            server.shutdown().await?;
        }
        _ => bail!("Usage: provision_app setup ADMIN_CONFIG PLAN PRIVATE_STATE | provision_app serve CANONICAL_SERVER_CONFIG"),
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../examples/provision-app/tests.rs"]
mod tests;
