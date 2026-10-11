//! Protected installation into the existing rvoip-owned media lifecycle.
use super::{BootstrapAuth, Definition};
use anyhow::Result;
use connect_connectors::{vapi_origin::OriginKind, Vapi};
use connect_mcp::RunningMcp;
use connect_service::Service;
use connect_store::VapiSessionRoute;
use rvoip_vapi::VapiCallOptions;
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

pub(crate) struct CallSetup {
    service: Arc<Service>,
    directory: PathBuf,
    listener: Option<RunningMcp>,
    expires_at: i64,
}
impl CallSetup {
    pub(crate) fn expired(&self, now: i64) -> bool {
        self.listener.is_some() && self.expires_at <= now
    }
    pub(crate) async fn options(
        &self,
        caller: &str,
        expected: &VapiSessionRoute,
    ) -> Result<VapiCallOptions> {
        anyhow::ensure!(
            self.listener.is_some(),
            "Task setup is closed; retain original recovery intent"
        );
        super::stage_call(self.service.clone(), &self.directory, caller, expected).await
    }
    pub(crate) async fn shutdown(mut self) -> Result<()> {
        self.close().await
    }
    pub(crate) async fn close(&mut self) -> Result<()> {
        let Some(listener) = self.listener.take() else {
            return Ok(());
        };
        let result = listener.shutdown().await;
        if result.is_err() {
            self.service.readiness.fail("mcp-task");
        }
        result.map_err(Into::into)
    }
}
impl crate::runtime::RunningServer {
    /// Install one owner-selected task's accepted session before media starts.
    /// It creates a private intent and lifecycle-owned local MCP listener only;
    /// rvoip later owns call creation, audio and paired termination. No model
    /// tool can invoke this method or supply its protected credential reference.
    /// Operations are explicitly configured, never inferred from a native token.
    pub async fn prepare_vapi_task_session(
        &self,
        session_id: &str,
        definition: Definition,
        directory: &Path,
        provider: Arc<Vapi>,
    ) -> Result<SocketAddr> {
        anyhow::ensure!(
            definition.origin == OriginKind::Call,
            "Voice setup requires Call origin"
        );
        let route = self
            .service
            .authorize_vapi_task_session_setup(
                definition.credential_id.clone(),
                definition.task_id.clone(),
                session_id.into(),
            )
            .await?;
        anyhow::ensure!(
            route.assistant_id == definition.assistant_id,
            "Saved assistant differs from voice route"
        );
        let registrar = self.media_registrar()?;
        let listener = self
            .mcp_capacity
            .start_auxiliary(
                self.service.clone(),
                Arc::new(BootstrapAuth::new(
                    self.service.clone(),
                    provider,
                    directory.into(),
                )),
                "127.0.0.1:0".parse()?,
            )
            .await?;
        let address = listener.address;
        let expires_at = definition.expires_at;
        if let Err(error) = super::prepare(
            self.service.clone(),
            definition,
            &format!("http://{address}/mcp"),
            directory,
        )
        .await
        {
            listener.shutdown().await?;
            return Err(error);
        }
        // The private file owns secrets; the Store owns the durable binding.
        // Persist before registering media so a crash cannot lose this boundary.
        let registration = async {
            let held = super::state::Locked::load(directory)?;
            self.service
                .register_vapi_task_session_setup(
                    held.state.definition.credential_id.clone(),
                    held.state.definition.grant(&held.state.nonce),
                    session_id.into(),
                    route,
                )
                .await?;
            Ok::<(), anyhow::Error>(())
        }
        .await;
        if let Err(error) = registration {
            listener.shutdown().await?;
            return Err(error);
        }
        let setup = Box::new(CallSetup {
            service: self.service.clone(),
            directory: directory.into(),
            listener: Some(listener),
            expires_at,
        });
        if let Err(setup) = registrar
            .register_vapi_setup(session_id.into(), setup)
            .await?
        {
            setup.shutdown().await?;
            anyhow::bail!("Session already has a protected Vapi setup or active call");
        }
        Ok(address)
    }
}
