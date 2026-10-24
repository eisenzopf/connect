//! Current per-packet policy; only exact active source file custody is reused.
use super::*;
impl Policy {
    pub(super) fn packet_current(
        &self,
        scope: &MediaScope<'_>,
        held: Option<Arc<NativeMediaRead>>,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        if let Some(target) = self.initial_target(scope.connection)? {
            return target.current(scope);
        }
        if let Some(target) = self.telephone_target(scope.connection)? {
            return target.current(scope);
        }
        if let Some(target) = self.replacement_target(scope.connection)? {
            return target.current(scope);
        }
        let entry = self.entry(scope.connection)?;
        self.scope(scope, &entry).map_err(|error|{tracing::trace!(target:"connect_service::canonical_packet",connection=entry.origin.connection_id(),?error,"Native packet scope refused");error})?;
        if let MediaPurpose::Receive { source } = scope.purpose {
            if let Some(initial) = self.initial_target(&source.connection)? {
                return initial.native_receive(scope, source, &entry);
            }
            if let Some(telephone) = self.telephone_target(&source.connection)? {
                return telephone.native_receive(scope, source, &entry);
            }
        }
        let source = match scope.purpose {
            MediaPurpose::Receive { source } => Some((source, self.entry(&source.connection)?)),
            _ => None,
        };
        let started = std::time::Instant::now();
        let result = match (held.as_deref(), source.as_ref()) {
            (Some(held), Some((descriptor, source))) => {
                // The opaque context must be this actual source binding, not
                // another grant that happens to cover the same Conversation.
                source_matches(descriptor, source, held.target())?;
                self.store
                    .try_native_media_read_under(&entry.origin, &source.origin, held)
            }
            (Some(held), None) if matches!(scope.purpose, MediaPurpose::Publish) => self
                .store
                .try_native_media_source_read_under(&entry.origin, held),
            (Some(_), None) => return Err(StreamControlError::Authority),
            (None, _) => self.store.try_native_media_read(
                &entry.origin,
                source.as_ref().map(|(_, entry)| entry.origin.as_ref()),
            ),
        };
        tracing::trace!(target: "connect_service::canonical_packet",
            connection = entry.origin.connection_id(),
            elapsed_us = started.elapsed().as_micros() as u64,
            publishing = matches!(scope.purpose, MediaPurpose::Publish),
            owned_file_custody = held.is_some(),
            role = result.as_ref().ok().map(|r|r.target().presence_role.as_str()),
            detail = result.as_ref().err().map(|e|e.message.as_str()),
            receiving = source.is_some(),
            private = entry.origin.connection_id().starts_with("pvc_"),
            problem = result.as_ref().err().map(|e| e.code.as_str()),
            "Current native packet authority");
        let read = result.map_err(packet_authority_error)?;
        if !read.current() {
            return Err(StreamControlError::Authority);
        }
        // Local engine preparation can precede the durable readiness commit.
        // Fresh publication/delivery cannot use that intermediate state.
        if matches!(scope.purpose, MediaPurpose::Publish) && read.target().publication_suspended {
            return Err(StreamControlError::NotReady);
        }
        if !matches!(scope.purpose, MediaPurpose::Control)
            && !matches!(read.target().connection.state.as_str(), "ready" | "active")
        {
            return Err(StreamControlError::NotReady);
        }
        if source.is_some() && !read.permits_subscriptions() {
            return Err(StreamControlError::Authority);
        }
        if let Some((source, source_entry)) = &source {
            source_matches(
                source,
                source_entry,
                read.source().ok_or(StreamControlError::Authority)?,
            )?;
        }
        let owner = entry.owner(read.target(), scope.principal, source.is_some()).map_err(|error|{tracing::trace!(target:"connect_service::canonical_packet",connection=entry.origin.connection_id(),?error,"Native packet owner refused");error})?;
        let owner = entry
            .activation()?
            .ok_or(StreamControlError::NotReady)?
            .current(owner).map_err(|error|{tracing::trace!(target:"connect_service::canonical_packet",connection=entry.origin.connection_id(),?error,"Native packet activation refused");error})?;
        Ok(Box::new(Fresh::new(owner, read, held)))
    }
}
