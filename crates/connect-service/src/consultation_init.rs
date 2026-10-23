//! Protected server initialization hook. Ordinary clients submit the operation.
use async_trait::async_trait;
use connect_protocol::{transitions::TransitionCommand, Error, Result};
use connect_store::NativeConsultationOrigin;
use std::sync::Arc;

/// Owned metadata from the admitted intent; no retained command reader or bearer.
pub struct ConsultationInitialization {
    pub credential_id: String,
    pub command: TransitionCommand,
    pub origin: NativeConsultationOrigin,
}
#[async_trait]
pub trait ConsultationInitializer: Send + Sync {
    async fn initialize(&self, request: ConsultationInitialization) -> Result<()>;
    /// Stop new effects immediately; owned jobs still retain their receipts.
    fn cancel(&self);
    async fn drain(&self) -> Result<()>;
}
impl crate::Service {
    /// Protected host composition, installed once. It adds no wire permission,
    /// endpoint or model tool and cannot bypass current Store/native authority.
    pub fn install_consultation_initializer(
        &self,
        initializer: Arc<dyn ConsultationInitializer>,
    ) -> Result<()> {
        let runtime = self
            .canonical_runtime
            .get()
            .ok_or_else(|| Error::new("not-ready", "Canonical native runtime is not installed"))?;
        if runtime.stop.is_cancelled() {
            return Err(Error::forbidden());
        }
        runtime
            .consultation_initializer
            .set(initializer)
            .map_err(|_| Error::new("conflict", "Consultation initializer is already installed"))
    }
    pub fn cancel_consultation_initialization(&self) {
        if let Some(initializer) = self
            .canonical_runtime
            .get()
            .and_then(|r| r.consultation_initializer.get())
        {
            initializer.cancel();
        }
    }
    pub async fn drain_consultation_initialization(&self) -> Result<()> {
        if let Some(initializer) = self
            .canonical_runtime
            .get()
            .and_then(|r| r.consultation_initializer.get())
        {
            initializer.cancel();
            initializer.drain().await?;
        }
        Ok(())
    }
}
