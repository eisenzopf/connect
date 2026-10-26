//! A fresh native source and a committed selector are both required. Idle Core
//! selector metadata and a copied operation record never constitute this grant.
use super::*;

pub struct NativeReconciliationRead {
    pub(crate) media: NativeMediaRead,
    pub(crate) owner: Arc<InstallationOwnership>,
    pub(crate) parent: String,
    pub(crate) subscription: String,
    pub(crate) credential: String,
    pub(crate) key: String,
    pub(crate) device: String,
    pub(crate) expires_at: i64,
    pub(crate) revision: u64,
    metadata: CredentialMetadata,
    signing_key: PeerSigningKey,
    peer_device: crate::PeerDeviceClaim,
}
impl NativeReconciliationRead {
    pub fn current(&self) -> bool {
        self.media.current() && self.media.permits_subscriptions()
    }
    pub fn media(&self) -> &NativeMediaRead {
        &self.media
    }
    pub fn stream_revision(&self) -> u64 {
        self.revision
    }
    pub fn subscription_record_id(&self) -> &str {
        &self.subscription
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
        if !Arc::ptr_eq(&owner, &self.owner) || !self.current() {
            return Err(Error::forbidden());
        }
        Ok(())
    }
}
impl Store {
    /// Fresh original-owner permission for a small recorded-operation hint.
    /// The returned read exists only during actual reliable metadata output.
    pub fn try_native_reconciliation_hint_read(
        &self,
        origin: &NativeMediaOrigin,
        record: &str,
    ) -> Result<(
        NativeReconciliationRead,
        connect_protocol::native_streams::ReconciliationHint,
    )> {
        let read = self.try_native_reconciliation_read(origin)?;
        let c = self.try_connection()?;
        let (operation_id,expected_revision,revision,stream_revision): (String,u64,u64,u64)=c.query_row(
            "SELECT o.operation_id,o.expected_revision,o.revision,json_extract(o.result_json,'$.stream_revision') FROM operations o JOIN native_stream_operations n ON n.operation_record_id=o.record_id JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id WHERE o.record_id=?1 AND o.command_type='stream.reconcile' AND o.state='completed' AND o.conversation_id=?2 AND o.session_id=?3 AND o.connection_id=?4 AND n.parent_record_id=?5 AND n.credential_id=?6 AND n.key_id=?7 AND n.device_id=?8 AND n.closed=1 AND e.state='confirmed' AND json_extract(o.result_json,'$.server_reconciliation')=1 AND json_extract(o.result_json,'$.resources_changed')=0 AND json_extract(o.result_json,'$.subscription_record_id')=?9 AND json_extract(o.intent_json,'$.payload.subscription_record_id')=?9 AND n.generation=?10 AND json_extract(o.result_json,'$.connection_generation')=?10 AND json_extract(o.result_json,'$.command_type')='stream.reconcile' AND json_extract(o.result_json,'$.expected_revision')=o.expected_revision",
            params![record,origin.conversation_id(),origin.session_id(),origin.connection_id(),read.parent,read.credential,read.key,read.device,read.subscription,read.media.target().connection.generation],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        ).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
        let hint = connect_protocol::native_streams::ReconciliationHint {
            operation_id,
            record_id: record.into(),
            command_type: connect_protocol::native_streams::Command::Reconcile,
            expected_revision,
            state: connect_protocol::transitions::OperationState::Completed,
            revision,
            subscription_record_id: read.subscription.clone(),
            stream_revision,
            journal_hint: true,
            resources_changed: false,
        };
        hint.validate()?;
        read.require(self)?;
        Ok((read, hint))
    }
    pub fn try_native_reconciliation_read(
        &self,
        origin: &NativeMediaOrigin,
    ) -> Result<NativeReconciliationRead> {
        let media = self.try_native_media_read(origin, None)?;
        if !media.permits_subscriptions() {
            return Err(Error::forbidden());
        }
        let c = self.try_connection()?;
        let (metadata, signing_key, _, _) = checked(self, &c, &origin.binding)?;
        // The last committed explicit control anchors current Core intent.
        // Automatic reconciliation cannot invent or become another user intent.
        let subscription: String = c.query_row(
            "SELECT o.record_id FROM operations o JOIN native_stream_operations n ON n.operation_record_id=o.record_id JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id WHERE n.parent_record_id=?1 AND o.command_type IN ('stream.subscribe','stream.unsubscribe') AND o.state='completed' AND n.closed=1 AND e.state='confirmed' AND n.credential_id=?2 AND n.key_id=?3 AND n.device_id=?4 ORDER BY o.rowid DESC LIMIT 1",
            params![origin.attempt.record_id(),origin.binding.credentials.last().ok_or_else(Error::forbidden)?,origin.binding.key_id,origin.binding.device.id], |r|r.get(0),
        ).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
        let revision: u64 = c.query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1", [origin.connection_id()], |r|r.get(0)).map_err(|_|Error::storage())?;
        if revision == 0 || revision >= 9_007_199_254_740_991 {
            return Err(Error::forbidden());
        }
        let value = NativeReconciliationRead {
            owner: media.owner.clone(),
            expires_at: media.expires_at,
            media,
            parent: origin.attempt.record_id().into(),
            subscription,
            credential: origin
                .binding
                .credentials
                .last()
                .ok_or_else(Error::forbidden)?
                .clone(),
            key: origin.binding.key_id.clone(),
            device: origin.binding.device.id.clone(),
            revision,
            metadata,
            signing_key,
            peer_device: origin.binding.device.clone(),
        };
        value.require(self)?;
        Ok(value)
    }
}
