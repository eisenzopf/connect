//! Sealed original message facts plus bounded held or fresh delivery authority.
use super::*;
use connect_protocol::native_streams::Intent;
use connect_store::NativeDataRead;
use rvoip_core::staged_streams::reliable_data::StagedData;
use rvoip_uctp::v2::auth_dispatch::native_data_message;
pub(crate) struct DataInvocation {
    policy: Arc<Policy>,
    connection: Arc<Connection>,
    authorized: Arc<AuthorizedEnvelope>,
    read: Arc<NativeDataRead>,
    wire: Intent,
    deadline: tokio::time::Instant,
}
impl DataInvocation {
    pub fn matches(&self, a: &AuthorizedEnvelope) -> bool {
        std::ptr::eq(a, Arc::as_ptr(&self.authorized))
            && crate::canonical_native_streams::intent(a.envelope()).as_ref() == Ok(&self.wire)
    }
    fn current(&self) -> ControlResult<()> {
        if !self.read.current()
            || tokio::time::Instant::now() >= self.deadline
            || self.authorized.invocation_cancellation().is_cancelled()
            || !self
                .authorized
                .authority::<StoreInvocationAuthority>()
                .and_then(|h| h.peer_guard())
                .is_some_and(|g| g.current())
        {
            return Err(StreamControlError::Authority);
        }
        Ok(())
    }
}
struct HeldData {
    _invocation: Arc<DataInvocation>,
    owner: MediaOwner,
}
impl MediaAuthorityLease for HeldData {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
}
struct FreshData {
    _read: NativeDataRead,
    owner: MediaOwner,
}
impl MediaAuthorityLease for FreshData {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
}
fn data_denied(error: connect_protocol::Error) -> StreamControlError {
    match error.code.as_str() {
        "not-recipient" | "conflict" | "unsupported-media-profile" => {
            StreamControlError::NotRecipient
        }
        _ => StreamControlError::Authority,
    }
}
impl Policy {
    pub(crate) fn data_invocation(
        self: &Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        lease: &connect_store::NativeStreamLease,
        deadline: tokio::time::Instant,
    ) -> connect_protocol::Result<Arc<DataInvocation>> {
        let proof = a
            .authority::<StoreInvocationAuthority>()
            .and_then(|h| h.peer_guard())
            .ok_or_else(connect_protocol::Error::forbidden)?;
        let connection = self
            .entry(&ConnectionId::from_string(lease.runtime_connection_id()))
            .map_err(|_| connect_protocol::Error::forbidden())?;
        let read = Arc::new(self.store.native_data_read_with_peer(
            &connection.origin,
            proof,
            lease,
        )?);
        let wire = crate::canonical_native_streams::intent(a.envelope())?;
        let i = Arc::new(DataInvocation {
            policy: self.clone(),
            connection,
            authorized: a,
            read,
            wire,
            deadline,
        });
        i.current()
            .map_err(|_| connect_protocol::Error::forbidden())?;
        crate::peer_authority::bind_native_data(&i.authorized, &i)?;
        Ok(i)
    }
    pub(super) fn data_authority(
        &self,
        scope: &MediaScope<'_>,
        data: &StagedData,
        held: Option<&(dyn Any + Send + Sync)>,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        let context = native_data_message(data).ok_or(StreamControlError::Authority)?;
        if !context.matches_source(scope) {
            return Err(StreamControlError::Authority);
        }
        let wire = crate::canonical_native_streams::intent(context.original()).map_err(denied)?;
        let entry = self.entry(scope.connection)?;
        self.scope(scope, &entry)?;
        if let Some(held) = held {
            let a = held
                .downcast_ref::<AuthorizedEnvelope>()
                .ok_or(StreamControlError::Authority)?;
            let i = crate::peer_authority::native_data_invocation(a)
                .ok_or(StreamControlError::Authority)?;
            if !matches!(scope.purpose, MediaPurpose::Publish)
                || !std::ptr::eq(self, Arc::as_ptr(&i.policy))
                || !Arc::ptr_eq(&entry, &i.connection)
                || context.original().original() != a.envelope().original()
                || wire != i.wire
            {
                return Err(StreamControlError::Authority);
            }
            i.current()?;
            let owner = entry.owner(i.read.media().target(), scope.principal, false)?;
            let owner = entry
                .activation()?
                .ok_or(StreamControlError::NotReady)?
                .current(owner)?;
            return Ok(Box::new(HeldData {
                _invocation: i,
                owner,
            }));
        }
        let (read, receiving) = match scope.purpose {
            MediaPurpose::Publish => (
                self.store
                    .try_native_data_publish_read(&entry.origin, &wire)
                    .map_err(data_denied)?,
                false,
            ),
            MediaPurpose::Receive { source } => {
                let source_entry = self.entry(&source.connection)?;
                if !context.addresses(&rvoip_core::ids::ParticipantId::from_string(
                    entry.origin.participant_id(),
                )) {
                    return Err(StreamControlError::NotRecipient);
                }
                let read = self
                    .store
                    .try_native_data_read(&entry.origin, &source_entry.origin, &wire)
                    .map_err(data_denied)?;
                source_matches(
                    source,
                    &source_entry,
                    read.media().source().ok_or(StreamControlError::Authority)?,
                )?;
                (read, true)
            }
            _ => return Err(StreamControlError::Authority),
        };
        if !read.current() {
            return Err(StreamControlError::Authority);
        }
        let owner = entry.owner(read.media().target(), scope.principal, receiving)?;
        let owner = entry
            .activation()?
            .ok_or(StreamControlError::NotReady)?
            .current(owner)?;
        if receiving {
            let writer = self
                .store
                .native_data_writer(&entry.origin)
                .map_err(denied)?;
            let source = read.media().source().ok_or(StreamControlError::Authority)?;
            self.store
                .try_reserve_native_data_write(
                    &writer,
                    &wire.operation_id,
                    data.message().message_id.as_str(),
                    &source.runtime_connection_id,
                )
                .map_err(denied)?;
        }
        Ok(Box::new(FreshData { _read: read, owner }))
    }
}
