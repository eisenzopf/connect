//! Receiver-only original audio selection; separate from video installation.
use super::video::{prepare, Attempt};
use super::*;
use rvoip_core::staged_streams::audio_subscriptions::{
    AudioReplacementPreparation, AudioReplacementPreparationAttempt,
};
impl Runtime {
    pub(super) fn prepare_private_audio<'a>(
        &'a self,
        root: &'a Arc<PreparedReplacement>,
        started: &'a Arc<StartedReplacement>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<AudioReplacementPreparation>> + Send + 'a>,
    > {
        Box::pin(async move {
            let until = root
                .deadline
                .min(tokio::time::Instant::now() + Duration::from_secs(5));
            let expected = started.controller.revision();
            let cancellation = root.handle.cancellation();
            prepare(
                &self.role_projection,
                &self.stop,
                &cancellation,
                until,
                || async {
                    let capture = self
                        .continuation_read_once(root, started, Phase::AudioPreroll(expected))
                        .await?;
                    let deadline = capture.deadline;
                    Ok((capture, deadline))
                },
                |capture| match self
                    .core
                    .prepare_staged_device_replacement_audio_with_held(&root.handle, capture)
                {
                    Ok(AudioReplacementPreparationAttempt::MetadataBusy { deadline }) => {
                        Ok(Attempt::MetadataBusy(deadline))
                    }
                    Ok(AudioReplacementPreparationAttempt::Prepared(prepared)) => {
                        Ok(Attempt::Prepared(prepared))
                    }
                    Err(error) => {
                        tracing::debug!(?error, "Replacement private audio preparation refused");
                        Err(Error::forbidden())
                    }
                },
            )
            .await
        })
    }
}
