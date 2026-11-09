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
        // This code denotes a refused metadata poll, never a revoked grant.
        // Queued Data already retries it with the same original identity.
        "authority-busy" => StreamControlError::MetadataBusy,
        "not-recipient" | "conflict" | "unsupported-media-profile" => {
            StreamControlError::NotRecipient
        }
        _ => StreamControlError::Authority,
    }
}
impl Policy {
    pub(super) fn data_entry(
        &self,
        id: &ConnectionId,
    ) -> connect_protocol::Result<Arc<Connection>> {
        self.entry(id).map_err(|error| {
            // entry() returns NotReady only when its metadata mutex is busy.
            // Missing or poisoned entries remain definitive authority failures.
            let error = match error {
                StreamControlError::NotReady => {
                    connect_protocol::Error::new("authority-busy", "Native data registry is busy")
                }
                _ => connect_protocol::Error::forbidden(),
            };
            crate::canonical_native_data::failure("data-invocation-registry", &error);
            error
        })
    }
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
        let connection =
            self.data_entry(&ConnectionId::from_string(lease.runtime_connection_id()))?;
        let read = Arc::new(
            self.store
                .native_data_read_with_peer(&connection.origin, proof, lease)
                .inspect_err(|error| {
                    crate::canonical_native_data::failure("data-invocation-store", error);
                })?,
        );
        let wire = crate::canonical_native_streams::intent(a.envelope())?;
        let i = Arc::new(DataInvocation {
            policy: self.clone(),
            connection,
            authorized: a,
            read,
            wire,
            deadline,
        });
        i.current().map_err(|_| {
            let error = connect_protocol::Error::forbidden();
            crate::canonical_native_data::failure("data-invocation-current", &error);
            error
        })?;
        crate::peer_authority::bind_native_data(&i.authorized, &i).inspect_err(|error| {
            crate::canonical_native_data::failure("data-invocation-binding", error);
        })?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_read_metadata_contention_remains_distinct_from_storage_or_authority_failure() {
        let busy = data_denied(connect_protocol::Error::new(
            "authority-busy",
            "Current original metadata interval is busy",
        ));
        assert!(matches!(busy, StreamControlError::MetadataBusy));
        // Durable storage failure is not evidence of temporary lock contention.
        // Unknown future errors must also remain a closed authorization result.
        for error in [
            connect_protocol::Error::storage(),
            connect_protocol::Error::forbidden(),
            connect_protocol::Error::new("not-ready", "Missing original projection"),
            connect_protocol::Error::new("unknown-error", "Opaque failure"),
        ] {
            assert!(matches!(data_denied(error), StreamControlError::Authority));
        }
    }

    #[test]
    fn data_read_original_audience_refusals_remain_terminal_for_this_message() {
        for code in ["not-recipient", "conflict", "unsupported-media-profile"] {
            let error =
                connect_protocol::Error::new(code, "Original Data cannot address this target");
            assert!(matches!(
                data_denied(error),
                StreamControlError::NotRecipient
            ));
        }
    }
}
