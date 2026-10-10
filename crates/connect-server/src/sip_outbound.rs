//! Durable outbound SIP worker for accepted Connect Sessions.

use crate::media_lifecycle::MediaRegistrar;
use connect_service::StoreSessionBindingResolver;
use connect_store::{SipLegAction, Store};
use rvoip_core::{
    adapter::{EndReason, OriginateRequest},
    capability::CapabilityDescriptor,
    connection::{Direction, Transport},
    ids::{ConnectionId, ParticipantId},
    Orchestrator,
};
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

pub(crate) struct Worker {
    stop: CancellationToken,
    task: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
}

enum OriginationOutcome {
    Active,
    Cancelled,
    CleanupUncertain,
}

#[derive(Clone, Copy)]
enum AuthorityCheck {
    ObserveCancellation,
    AdmitDispatch,
}

impl Worker {
    pub(crate) fn start(
        store: Arc<Store>,
        readiness: Arc<connect_service::readiness::OperationalReadiness>,
        orchestrator: Arc<Orchestrator>,
        resolver: Arc<StoreSessionBindingResolver>,
        registrar: MediaRegistrar,
    ) -> anyhow::Result<Self> {
        store.recover_sip_legs()?;
        let stop = CancellationToken::new();
        let cancelled = stop.clone();
        let task = crate::worker::spawn(readiness, "sip-outbound", stop.clone(), async move {
            loop {
                if cancelled.is_cancelled() {
                    return Ok(());
                }
                let claim_store = store.clone();
                let job = tokio::task::spawn_blocking(move || claim_store.claim_sip_leg_job())
                    .await
                    .map_err(|_| anyhow::anyhow!("SIP leg claim task failed"))??;
                let Some(job) = job else {
                    tokio::select! {
                        _ = cancelled.cancelled() => return Ok(()),
                        _ = tokio::time::sleep(Duration::from_millis(100)) => {},
                    }
                    continue;
                };
                if job.action == SipLegAction::Start
                    && check_authority(
                        store.clone(),
                        job.clone(),
                        AuthorityCheck::ObserveCancellation,
                    )
                    .await?
                {
                    check_authority(store.clone(), job.clone(), AuthorityCheck::AdmitDispatch)
                        .await?;
                    continue;
                }
                match job.action {
                    SipLegAction::Start => {
                        let result: anyhow::Result<OriginationOutcome> = async {
                            let session = resolver
                                .project_credential(&job.credential_id, &job.leg.session_id)
                                ?;
                            let prepared = orchestrator
                                .prepare_outbound_connection(
                                    OriginateRequest::new(
                                        session,
                                        ParticipantId::from_string(job.actor_id.clone()),
                                        job.leg.destination.clone(),
                                        Direction::Outbound,
                                        CapabilityDescriptor::default(),
                                    )
                                    .with_transport(Transport::Sip),
                                )
                                .await?;
                            let connection_id = prepared.connection_id().clone();
                            store.prepare_sip_leg(&job, connection_id.as_str())?;
                            registrar
                                .register_outbound(
                                    connection_id.clone(),
                                    job.credential_id.clone(),
                                    job.leg.session_id.clone(),
                                    job.leg.id.clone(),
                                )
                                .await?;
                            if !check_authority(store.clone(), job.clone(), AuthorityCheck::AdmitDispatch).await? {
                                return Ok(OriginationOutcome::Cancelled);
                            }
                            let mut commit = std::pin::pin!(prepared.commit());
                            loop {
                                tokio::select! {
                                    result = &mut commit => {
                                        result?;
                                        break;
                                    }
                                    _ = tokio::time::sleep(Duration::from_millis(50)) => {
                                        let check_store = store.clone();
                                        let check_job = job.clone();
                                        let cancelled = tokio::task::spawn_blocking(move || {
                                            check_store.sip_origination_cancelled(&check_job)
                                        })
                                        .await
                                        .map_err(|_| connect_protocol::Error::storage())??;
                                        if cancelled {
                                            return Ok(if orchestrator.end_connection(connection_id.clone(), EndReason::Cancelled).await.is_ok() {
                                                OriginationOutcome::Cancelled
                                            } else {
                                                OriginationOutcome::CleanupUncertain
                                            });
                                        }
                                    }
                                }
                            }
                            registrar.activate_outbound(connection_id).await?;
                            anyhow::Ok(OriginationOutcome::Active)
                        }
                        .await;
                        match result {
                            Ok(OriginationOutcome::Cancelled) => store.finish_sip_leg_job(
                                &job,
                                "cancelled",
                                Some("local-cancel-before-answer"),
                            )?,
                            Ok(OriginationOutcome::Active) => {
                                store.finish_sip_leg_job(&job, "active", None)?
                            }
                            Ok(OriginationOutcome::CleanupUncertain) => store.finish_sip_leg_job(
                                &job,
                                "uncertain",
                                Some("sip-cancellation-uncertain"),
                            )?,
                            Err(error)
                                if error.downcast_ref::<connect_protocol::Error>().is_some_and(
                                    |e| {
                                        e.code == "storage-unavailable"
                                            || e.code == "capacity-exceeded"
                                    },
                                ) =>
                            {
                                return Err(error)
                            }
                            Err(_) => store.finish_sip_leg_job(
                                &job,
                                "failed",
                                Some("sip-origination-failed"),
                            )?,
                        }
                    }
                    SipLegAction::End => {
                        if !check_authority(
                            store.clone(),
                            job.clone(),
                            AuthorityCheck::AdmitDispatch,
                        )
                        .await?
                        {
                            continue;
                        }
                        let result = match job.leg.connection_id.as_deref() {
                            Some(id) => {
                                orchestrator
                                    .end_connection(
                                        ConnectionId::from_string(id),
                                        EndReason::Cancelled,
                                    )
                                    .await
                            }
                            None => Err(rvoip_core::RvoipError::InvalidState(
                                "active SIP leg has no connection",
                            )),
                        };
                        store.finish_sip_leg_job(
                            &job,
                            if result.is_ok() { "ended" } else { "uncertain" },
                            Some(if result.is_ok() {
                                "local-hangup"
                            } else {
                                "sip-hangup-uncertain"
                            }),
                        )?;
                    }
                }
            }
        });
        Ok(Self {
            stop,
            task: Some(task),
        })
    }

    pub(crate) async fn shutdown(mut self) -> anyhow::Result<()> {
        self.stop.cancel();
        crate::worker::drain(
            self.task.take().into_iter().collect(),
            Duration::from_secs(5),
        )
        .await
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

// Keep SQLite off the async executor, including after asynchronous preparation.
async fn check_authority(
    store: Arc<Store>,
    job: connect_store::SipLegJob,
    check: AuthorityCheck,
) -> anyhow::Result<bool> {
    tokio::task::spawn_blocking(move || match check {
        AuthorityCheck::AdmitDispatch => store.sip_dispatch_allowed(&job),
        AuthorityCheck::ObserveCancellation => store.sip_origination_cancelled(&job),
    })
    .await
    .map_err(|_| connect_protocol::Error::storage())?
    .map_err(Into::into)
}
