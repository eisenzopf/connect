//! One owned callback endpoint; task identity survives endpoint retirement.
use super::manifest::Manifest;
use crate::{
    runtime::RunningServer,
    vapi_bootstrap::{self, BootstrapAuth},
};
use anyhow::Result;
use connect_connectors::{vapi_origin::OriginKind, Vapi};
use connect_mcp::RunningMcp;
use std::{sync::Arc, time::Duration};

pub(super) struct Prepared {
    listener: Option<RunningMcp>,
    expires_at: i64,
}
impl Prepared {
    pub async fn serve(
        self,
        server: &RunningServer,
        mut shutdown: crate::cli::serve::Shutdown,
    ) -> Result<()> {
        let Some(listener) = self.listener else {
            return shutdown.wait(server).await;
        };
        let waiting = shutdown.wait(server);
        tokio::pin!(waiting);
        let seconds = self
            .expires_at
            .saturating_sub(chrono::Utc::now().timestamp())
            .max(0) as u64;
        let stopped = tokio::select! {
            result = &mut waiting => Some(result),
            _ = tokio::time::sleep(Duration::from_secs(seconds)) => None,
        };
        // Expiry closes only this invocation endpoint, preserving the server and
        // conversation. Cancellation/revocation are checked on every request.
        listener.shutdown().await?;
        match stopped {
            Some(result) => result,
            None => waiting.await,
        }
    }
}

pub(super) async fn prepare(
    server: &RunningServer,
    manifest: &Manifest,
    provider: Arc<Vapi>,
) -> Result<Prepared> {
    let definition = &manifest.definition;
    if definition.origin == OriginKind::Call {
        let session = manifest
            .session_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Missing accepted voice session"))?;
        let address = server
            .prepare_vapi_task_session_with_callback(
                session,
                definition.clone(),
                &manifest.state_directory,
                provider,
                manifest.callback_url.as_deref(),
            )
            .await?;
        report(manifest, address, None);
        return Ok(Prepared {
            listener: None,
            expires_at: definition.expires_at,
        });
    }
    let bind = manifest
        .callback_bind
        .ok_or_else(|| anyhow::anyhow!("Missing chat callback binding"))?;
    let listener = server
        .mcp_capacity
        .start_auxiliary(
            server.service.clone(),
            Arc::new(BootstrapAuth::new(
                server.service.clone(),
                provider.clone(),
                manifest.state_directory.clone(),
            )),
            bind,
        )
        .await?;
    let address = listener.address;
    let local_url = format!("http://{address}/mcp");
    let url = manifest.callback_url.as_deref().unwrap_or(&local_url);
    let setup = async {
        if manifest.state_directory.try_exists()? {
            vapi_bootstrap::verify_prepared(
                server.service.clone(),
                definition,
                Some(url),
                &manifest.state_directory,
            )
            .await?;
        } else {
            vapi_bootstrap::prepare(
                server.service.clone(),
                definition.clone(),
                url,
                &manifest.state_directory,
            )
            .await?;
        }
        vapi_bootstrap::create_session(server.service.clone(), &provider, &manifest.state_directory)
            .await
    };
    let result = tokio::time::timeout(Duration::from_secs(30), setup)
        .await
        .map_err(|_| {
            anyhow::anyhow!("Vapi setup deadline exceeded; retain and reconcile original intent")
        })
        .and_then(|result| result);
    match result {
        Ok(id) => {
            report(manifest, address, Some(&id));
            Ok(Prepared {
                listener: Some(listener),
                expires_at: definition.expires_at,
            })
        }
        Err(error) => {
            listener.shutdown().await?;
            Err(error)
        }
    }
}
fn report(manifest: &Manifest, address: std::net::SocketAddr, external_id: Option<&str>) {
    println!(
        "{}",
        serde_json::json!({"status":"vapi-task-ready", "task_id":manifest.definition.task_id,
        "origin":manifest.definition.origin, "callback_local":format!("http://{address}/mcp"),
        "provider_resource":external_id})
    );
}
