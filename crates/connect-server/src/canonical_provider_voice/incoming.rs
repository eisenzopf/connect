//! Owned administrative discovery/commit jobs, drained with the provider host.
use super::*;
use connect_protocol::incoming_contacts::{IncomingContact, IncomingContactSelection};
use connect_store::ProviderIncomingVoiceResolutionPreparation as Preparation;
struct Resolving {
    host: Weak<Host>,
    key: Key,
}
impl Drop for Resolving {
    fn drop(&mut self) {
        if let Some(host) = self.host.upgrade() {
            host.state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .resolving
                .remove(&self.key);
        }
    }
}
impl Host {
    pub(super) async fn resolve_incoming(
        self: &Arc<Self>,
        credential: String,
        selection: IncomingContactSelection,
    ) -> Result<IncomingContact> {
        selection.validate()?;
        let key = (
            credential.clone(),
            selection.operation_id.clone(),
            selection.candidate_id.clone(),
        );
        let (send, receive) = tokio::sync::oneshot::channel();
        {
            let mut state = self.state.lock().map_err(|_| Error::storage())?;
            Self::reap(&mut state);
            if state.closed || self.stop.is_cancelled() {
                return Err(Error::forbidden());
            }
            if state.resolving.contains(&key) {
                return Err(Error::new(
                    "operation-pending",
                    "Incoming decision is still owned; retry its original operation ID",
                ));
            }
            if state.resolving.len() >= self.capacity {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Incoming decision capacity exhausted",
                ));
            }
            let role = state.role.as_ref().cloned().ok_or_else(Error::forbidden)?;
            state.resolving.insert(key.clone());
            let active = Resolving {
                host: Arc::downgrade(self),
                key,
            };
            let host = self.clone();
            state.resolutions.spawn(async move {
                let _active = active;
                let result = resolve(host, role, credential, selection).await;
                let _ = send.send(result);
                // Expected administrative refusals do not poison the media host.
                Ok(())
            });
        }
        receive.await.map_err(|_| {
            Error::new(
                "transport-uncertain",
                "Incoming decision reply was interrupted; retain its operation ID",
            )
        })?
    }
}
async fn resolve(
    host: Arc<Host>,
    role: Arc<Role>,
    credential: String,
    selection: IncomingContactSelection,
) -> Result<IncomingContact> {
    // Metadata authorizes the original owner before any protected driver lookup.
    let store = host.store.clone();
    let key = credential.clone();
    let id = selection.contact_id.clone();
    let metadata = database(move || store.incoming_contact(&key, &id)).await?;
    let candidate = metadata
        .candidates
        .iter()
        .find(|c| c.id == selection.candidate_id)
        .ok_or_else(Error::forbidden)?;
    let mut prepared = None;
    for profile in &host.profiles {
        if profile.policy.endpoint_id != candidate.endpoint_id {
            continue;
        }
        // Store alone validates the complete saved policy, not this coarse
        // hosting lookup. A mismatched account cannot authorize a resolution.
        let store = host.store.clone();
        let worker = role.clone();
        let key = credential.clone();
        let command = selection.clone();
        let manifest = profile.manifest.clone();
        let result = match database(move || {
            store.prepare_incoming_voice_resolution(&worker, &key, command, &manifest)
        })
        .await
        {
            Err(error) if error.code == "unsupported-connector-operation" => continue,
            result => result?,
        };
        match result {
            Preparation::Completed(contact) => return Ok(contact),
            Preparation::Query(ticket) if ticket.policy() == &profile.policy => {
                prepared = Some((profile.clone(), ticket));
                break;
            }
            Preparation::Query(_) => continue,
        }
    }
    let (profile, ticket) = prepared.ok_or_else(|| {
        Error::new(
            "provider-unavailable",
            "Original incoming provider profile is not mounted",
        )
    })?;
    let query = ticket.query()?;
    let remaining = ticket
        .expires_at()
        .saturating_sub(chrono::Utc::now().timestamp());
    if remaining <= 0 {
        return Err(Error::forbidden());
    }
    let receipts = tokio::select! {biased;
        _=host.stop.cancelled()=>return Err(Error::forbidden()),
        result=tokio::time::timeout(Duration::from_secs(remaining as u64),profile.driver.inbound_receipts(&query))=>result.map_err(|_|Error::new("provider-unavailable","Incoming resource reconciliation timed out"))?.map_err(|_|Error::new("provider-unavailable","Incoming resource reconciliation unavailable"))?
    };
    let captured = ticket.capture(receipts)?;
    if host.stop.is_cancelled() {
        return Err(Error::forbidden());
    }
    let store = host.store.clone();
    let result = database(move || store.record_incoming_voice_resolution(&captured)).await?;
    if let Some(origin) = result.origin {
        // Admission to the actual owned lifecycle is part of successful reply.
        // A committed result is durable even if reply/queue admission fails;
        // restart enumerates it and current replay retains the decision ID.
        if let Err(error) = host.queue_origin(origin.clone(), profile.clone(), true) {
            if let Err(cleanup_error) = cleanup(host.store.clone(), role, origin, profile).await {
                host.state.lock().map_err(|_| Error::storage())?.failed = true;
                return Err(cleanup_error);
            }
            return Err(error);
        }
    }
    Ok(result.contact)
}
