//! The rvoip hook owns the last authorization/one-dispatch boundary.
use super::state::Locked;
use connect_store::{Store, VapiSessionRoute};
use rvoip_vapi::{VapiCallAdmission, VapiError};
use std::{path::PathBuf, sync::Arc};

pub(super) struct Dispatch {
    pub store: Arc<Store>,
    pub directory: PathBuf,
    pub caller: String,
    pub expected: VapiSessionRoute,
}
#[async_trait::async_trait]
impl VapiCallAdmission for Dispatch {
    async fn admit(&self) -> rvoip_vapi::Result<()> {
        let directory = self.directory.clone();
        let store = self.store.clone();
        let caller = self.caller.clone();
        let expected = self.expected.clone();
        tokio::task::spawn_blocking(move || {
            let held = Locked::load(&directory)
                .map_err(|_| VapiError::InvalidCallOptions("Protected dispatch unavailable"))?;
            let state = held.state;
            if !state.posted {
                return Err(VapiError::InvalidCallOptions("Call intent was not staged"));
            }
            let admitted = store
                .admit_vapi_task_session_call(
                    &state.definition.credential_id,
                    &state.definition.grant(&state.nonce),
                    &caller,
                    &expected.session_id,
                    &expected,
                )
                .map_err(|_| VapiError::InvalidCallOptions("Current task call admission denied"))?;
            if !admitted {
                return Err(VapiError::InvalidCallOptions(
                    "Call may already have been dispatched",
                ));
            }
            Ok(())
        })
        .await
        .map_err(|_| VapiError::InvalidCallOptions("Call admission worker failed"))?
    }
}
