//! Grant-free waiting only; callers must have dropped every final-turn alias.
use super::*;
impl crate::canonical_runtime::Runtime {
    pub(crate) async fn managed_cut_metadata_pause(
        &self,
        until: tokio::time::Instant,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> connect_protocol::Result<()> {
        if self.stop.is_cancelled() || cancel.is_cancelled() || tokio::time::Instant::now() >= until
        {
            return Err(connect_protocol::Error::forbidden());
        }
        tokio::select! {biased;
            _=self.stop.cancelled()=>Err(connect_protocol::Error::forbidden()),
            _=cancel.cancelled()=>Err(connect_protocol::Error::forbidden()),
            _=tokio::time::sleep_until(until)=>Err(connect_protocol::Error::forbidden()),
            _=tokio::time::sleep_until(until.min(tokio::time::Instant::now()+Duration::from_millis(2)))=>Ok(()),
        }
    }
}
