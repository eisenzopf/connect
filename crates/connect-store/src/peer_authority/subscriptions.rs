//! Sealed read continuation facts. Each page reacquires a real current proof.
use super::*;
use connect_protocol::events::EventPage;
#[derive(Clone)]
pub struct PeerSubscriptionSource {
    owner: Arc<InstallationOwnership>,
    request: PeerAuthorityRequest,
    expires_at: i64,
    source_expiry: i64,
}
pub struct PeerSubscriptionRead {
    pub page: EventPage,
    pub next: PeerSubscriptionSource,
    pub proof: PeerAuthorityGuard,
    pub terminal: bool,
}
impl PeerSubscriptionSource {
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
    pub fn source_expires_at(&self) -> i64 {
        self.source_expiry
    }
}
impl std::fmt::Debug for PeerSubscriptionSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PeerSubscriptionSource(redacted)")
    }
}
impl std::fmt::Debug for PeerSubscriptionRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PeerSubscriptionRead(redacted)")
    }
}
impl Store {
    /// The initial page and continuation are derived from one exact signed
    /// invocation under its existing guard. No public facts-only constructor.
    pub fn start_peer_subscription(
        &self,
        credential: &str,
        command: &PeerSubscriptionCommand,
        proof: &PeerAuthorityGuard,
    ) -> Result<(EventPage, Option<PeerSubscriptionSource>)> {
        let page = self.conversation_events_with_peer(credential, command, proof)?;
        if !command.live {
            return Ok((page, None));
        }
        let c = self.connection()?;
        self.require_peer_subscription(&c, proof, credential, command)?;
        let member = crate::audience::member(
            &c,
            &proof.metadata.actor.identity_id,
            &command.conversation_id,
            true,
        )?;
        let mut request = proof.request.clone();
        let PeerCommand::Subscription(next) = &mut request.command else {
            return Err(Error::forbidden());
        };
        next.actor_epoch = Some((member.participant_id, member.membership_epoch));
        next.ceiling = Some(crate::events::scope(&proof.metadata.actor)?);
        next.query.after = Some(page.cursor.clone());
        next.query.lease = page.lease.clone();
        let expiry = page
            .expires_at
            .or_else(|| page.snapshot.as_ref().map(|s| s.expires_at))
            .ok_or_else(Error::storage)?
            .min(proof.expires_at());
        if expiry <= crate::now() {
            return Err(Error::new(
                "resync-required",
                "Subscription lease elapsed during admission; request a fresh boundary",
            ));
        }
        Ok((
            page,
            Some(PeerSubscriptionSource {
                owner: proof._owner.clone(),
                request,
                expires_at: expiry,
                source_expiry: proof.expires_at(),
            }),
        ))
    }
    /// Read a new page with original epoch/ceiling/credential/key, never borrowed
    /// authority. The successor cursor is committed by the transport only after
    /// confirmed local submission; a disconnect resumes from the client's cursor.
    pub fn read_peer_subscription(
        &self,
        source: &PeerSubscriptionSource,
        wait: &AuthorityWait,
    ) -> Result<PeerSubscriptionRead> {
        loop {
            wait.check()?;
            match self.read_peer_subscription_attempt(source, wait) {
                Err(error) if error.code == "authority-busy" => continue,
                result => return result,
            }
        }
    }
    fn read_peer_subscription_attempt(
        &self,
        source: &PeerSubscriptionSource,
        wait: &AuthorityWait,
    ) -> Result<PeerSubscriptionRead> {
        let owner = self.require_runtime_owner()?;
        if !Arc::ptr_eq(&owner, &source.owner)
            || !source.owner.current()
            || crate::now() >= source.source_expiry
            || crate::now() > source.expires_at.saturating_add(1)
        {
            return Err(Error::forbidden());
        }
        let mut request = source.request.clone();
        let PeerCommand::Subscription(command) = &mut request.command else {
            return Err(Error::forbidden());
        };
        let expired = crate::now() >= source.expires_at;
        if expired {
            command.query.after = None;
            command.query.lease = None;
        }
        command.query.lease_seconds =
            Some(source.expires_at.saturating_sub(crate::now()).clamp(1, 30) as u32);
        let command = command.clone();
        let mut proof = self.admit_peer_authority(&request, wait)?;
        proof.limit_expiry(source.source_expiry.min(if expired {
            source.expires_at.saturating_add(1)
        } else {
            source.expires_at
        }))?;
        let credential = request.credentials.last().ok_or_else(Error::forbidden)?;
        let mut page = self.conversation_events_with_peer(credential, &command, &proof)?;
        let terminal = expired || page.resync_required;
        if expired {
            page.resync_required = true;
            page.lease = None;
            page.expires_at = None;
        }
        let mut next = source.clone();
        let PeerCommand::Subscription(command) = &mut next.request.command else {
            return Err(Error::forbidden());
        };
        command.query.after = Some(page.cursor.clone());
        command.query.lease = page.lease.clone();
        Ok(PeerSubscriptionRead {
            page,
            next,
            proof,
            terminal,
        })
    }
}
