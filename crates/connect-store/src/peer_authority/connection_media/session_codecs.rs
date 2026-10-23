//! A current Session policy and the independently current actual native source
//! are both required. A codec preference, copied job or origin is never a grant.
use super::*;
use crate::SessionCodecTarget;

pub struct NativeSessionCodecRead {
    pub(crate) media: NativeMediaRead,
    pub(crate) batch: String,
    pub(crate) parent: String,
    pub(crate) credential: String,
    pub(crate) key: String,
    pub(crate) device: String,
    pub(crate) revision: u64,
    pub(crate) credential_epoch: u64,
    pub(crate) expires_at: i64,
    metadata: CredentialMetadata,
    signing_key: PeerSigningKey,
    peer_device: crate::PeerDeviceClaim,
}
impl NativeSessionCodecRead {
    pub fn current(&self) -> bool {
        self.media.current() && crate::now() < self.expires_at
    }
    pub fn media(&self) -> &NativeMediaRead {
        &self.media
    }
    pub fn batch_record_id(&self) -> &str {
        &self.batch
    }
    pub fn stream_revision(&self) -> u64 {
        self.revision
    }
    pub fn credential_metadata(&self) -> &CredentialMetadata {
        &self.metadata
    }
    pub fn signing_key(&self) -> &PeerSigningKey {
        &self.signing_key
    }
    pub fn signing_key_id(&self) -> &str {
        &self.key
    }
    pub fn peer_device(&self) -> &crate::PeerDeviceClaim {
        &self.peer_device
    }
    pub(crate) fn require(&self, store: &Store) -> Result<()> {
        let owner = store.require_runtime_owner()?;
        if !Arc::ptr_eq(&owner, &self.media.owner) || !self.current() {
            return Err(Error::forbidden());
        }
        Ok(())
    }
}

/// Resolve the original manager credential again. The accepted policy receipt
/// fixes intent; a background effect still needs current named management and
/// recipient authority, original membership epoch and the latest policy.
fn policy_facts(
    store: &Store,
    c: &Connection,
    target: &SessionCodecTarget,
) -> Result<(Vec<AuthorityScope>, i64)> {
    let (credential,owner,policy,runtime,expires,session,conversation,preferences,generation,state,latest,cancelled):(String,String,String,String,i64,String,String,String,u64,String,Option<String>,bool)=c.query_row(
        "SELECT b.manager_credential_id,o.owner_id,b.policy_operation_id,b.runtime_epoch,b.expires_at,o.session_id,o.conversation_id,b.codec_preferences_json,t.generation,t.state,s.codec_policy_operation_id,o.cancel_requested FROM session_codec_batches b JOIN operations o ON o.record_id=b.operation_record_id JOIN session_codec_targets t ON t.batch_record_id=b.operation_record_id JOIN session_control s ON s.session_id=o.session_id WHERE b.operation_record_id=?1 AND t.connection_id=?2 AND o.state IN ('accepted','preparing')",
        params![target.batch_record_id,target.connection_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?,r.get(11)?))
    ).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
    if runtime != store.runtime_epoch()
        || expires <= crate::now()
        || target.expires_at != expires
        || generation != target.generation
        || state != "queued"
        || cancelled
        || latest.as_deref() != Some(&policy)
        || serde_json::from_str::<Vec<String>>(&preferences).map_err(|_| Error::storage())?
            != target.codec_preferences
    {
        return Err(Error::forbidden());
    }
    let actor = crate::identity::actor_from_connection(c, &credential)?;
    if actor.identity_id != owner {
        return Err(Error::forbidden());
    }
    let saved = crate::operations::receipts::receipt(c, &actor.identity_id, &policy)?
        .ok_or_else(Error::forbidden)?;
    crate::operations::receipts::check_receipt_authority(c, &actor, &saved, None)?;
    let connect_protocol::Operation::SessionControl { request } = &saved.operation else {
        return Err(Error::forbidden());
    };
    let connect_protocol::session_control::Action::Renegotiate { codec_preferences } =
        &request.action
    else {
        return Err(Error::forbidden());
    };
    if request.session_id != session
        || request.conversation_id != conversation
        || *codec_preferences != target.codec_preferences
        || saved.result.is_err()
    {
        return Err(Error::forbidden());
    }
    crate::session_control::manager(c, &actor, request)?;
    if matches!(
        crate::session_control::view(c, &conversation, &session)?
            .state
            .as_str(),
        "ended" | "cancelled" | "rejected" | "failed"
    ) {
        return Err(Error::forbidden());
    }
    let scopes = crate::authority_admission::operation_scopes(c, &actor, &saved.operation, store)?
        .into_iter()
        .map(|scope| scope.read_scope())
        .collect();
    Ok((scopes, expires.min(actor.expires_at)))
}

impl Store {
    /// Acquire one complete current set, then resolve again under it. Unlike
    /// selector reconciliation this does not infer stream.subscribe permission.
    /// Each selected publisher still needs its own independent receive read.
    pub fn try_native_session_codec_read(
        &self,
        origin: &NativeMediaOrigin,
        target: &SessionCodecTarget,
    ) -> Result<NativeSessionCodecRead> {
        if origin.connection_id() != target.connection_id
            || origin.generation() != target.generation
        {
            return Err(Error::forbidden());
        }
        let scopes = {
            let c = self.try_connection()?;
            let mut scopes = facts(self, &c, origin)?.1;
            scopes.extend(policy_facts(self, &c, target)?.0);
            scopes.extend(checked(self, &c, &origin.binding)?.2);
            AuthorityScope::merged(&scopes)?
        };
        let held = self.try_acquire_authority(&scopes)?;
        let c = self.try_connection()?;
        let (view, mut required, expires) = facts(self, &c, origin)?;
        let (policy_scopes, policy_expires) = policy_facts(self, &c, target)?;
        required.extend(policy_scopes);
        let (metadata, signing_key, key_scopes, _) = checked(self, &c, &origin.binding)?;
        required.extend(key_scopes);
        if !self.authority_covers(&held, &required)
            || view.connection.connection_id != target.connection_id
            || view.connection.generation != target.generation
        {
            return Err(Error::forbidden());
        }
        let revision:u64=c.query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1 AND generation=?2 AND observed_runtime_id=?3 AND state IN ('ready','active')",params![target.connection_id,target.generation,self.runtime_epoch()],|r|r.get(0)).map_err(|_|Error::storage())?;
        if revision == 0 || revision >= 9_007_199_254_740_991 {
            return Err(Error::forbidden());
        }
        let expires_at = expires.min(policy_expires);
        let result = NativeSessionCodecRead {
            media: NativeMediaRead {
                target: view,
                source: None,
                _scopes: held,
                owner: current_owner(self, origin)?,
                expires_at,
                may_subscribe: subscription_permission(&c, origin)?,
            },
            batch: target.batch_record_id.clone(),
            parent: origin.attempt.record_id().into(),
            credential: origin
                .binding
                .credentials
                .last()
                .ok_or_else(Error::forbidden)?
                .clone(),
            key: origin.binding.key_id.clone(),
            device: origin.binding.device.id.clone(),
            revision,
            credential_epoch: origin.credential_epoch(),
            expires_at,
            metadata,
            signing_key,
            peer_device: origin.binding.device.clone(),
        };
        result.require(self)?;
        Ok(result)
    }
}
