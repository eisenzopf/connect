//! Current per-packet policy; only exact active source file custody is reused.
use super::*;

/// Retry only an unavailable current read, inside the engine's original packet
/// admission callback. No positive grant, SQL lock or partial reader survives a
/// failed attempt. The engine still checks its original gate/scene stamps after
/// this callback, so a mute/retirement during contention cannot become a replay.
/// The ten-millisecond retry window accommodates current metadata derivation at
/// a policy-clock edge, while remaining below one twenty-millisecond audio frame.
/// It bounds retry admission, not the duration of an individual Store read.
#[cfg(test)]
fn packet_read<T>(
    mut read: impl FnMut() -> connect_protocol::Result<T>,
) -> connect_protocol::Result<T> {
    let until = std::time::Instant::now() + std::time::Duration::from_millis(10);
    packet_read_until(until, &mut read)
}
fn packet_read_until<T>(
    until: std::time::Instant,
    mut read: impl FnMut() -> connect_protocol::Result<T>,
) -> connect_protocol::Result<T> {
    loop {
        match read() {
            Err(error) if error.code == "authority-busy" && std::time::Instant::now() < until => {
                std::thread::yield_now()
            }
            result => return result,
        }
    }
}
impl Policy {
    pub(super) fn packet_current(
        &self,
        scope: &MediaScope<'_>,
        held: Option<Arc<NativeMediaRead>>,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        // Share the existing packet-read retry budget across leaf registry and
        // Store acquisition. Retry no readiness, activation or authority result.
        let until = std::time::Instant::now() + std::time::Duration::from_millis(10);
        if let Some(target) =
            super::packet_metadata::read_until(until, || self.initial_target(scope.connection))?
        {
            return target.current(scope);
        }
        if let Some(target) =
            super::packet_metadata::read_until(until, || self.telephone_target(scope.connection))?
        {
            return target.current(scope);
        }
        if let Some(target) =
            super::packet_metadata::read_until(until, || self.replacement_target(scope.connection))?
        {
            return target.current(scope);
        }
        let entry = super::packet_metadata::read_until(until, || self.entry(scope.connection))?;
        self.scope(scope, &entry).map_err(|error|{tracing::trace!(target:"connect_service::canonical_packet",connection=entry.origin.connection_id(),?error,"Native packet scope refused");error})?;
        if let MediaPurpose::Receive { source } = scope.purpose {
            if let Some(initial) = super::packet_metadata::read_until(until, || {
                self.initial_target(&source.connection)
            })? {
                return initial.native_receive(scope, source, &entry);
            }
            if let Some(telephone) = super::packet_metadata::read_until(until, || {
                self.telephone_target(&source.connection)
            })? {
                return telephone.native_receive(scope, source, &entry);
            }
        }
        let source = match scope.purpose {
            MediaPurpose::Receive { source } => Some((
                source,
                super::packet_metadata::read_until(until, || self.entry(&source.connection))?,
            )),
            _ => None,
        };
        let started = std::time::Instant::now();
        if let (Some(held), Some((descriptor, source))) = (held.as_deref(), source.as_ref()) {
            source_matches(descriptor, source, held.target())?;
        }
        let result = packet_read_until(until, || match (held.as_deref(), source.as_ref()) {
            (Some(held), Some((_, source))) => {
                // The opaque context must be this actual source binding, not
                // another grant that happens to cover the same Conversation.
                self.store
                    .try_native_media_read_under(&entry.origin, &source.origin, held)
            }
            (Some(held), None) if matches!(scope.purpose, MediaPurpose::Publish) => self
                .store
                .try_native_media_source_read_under(&entry.origin, held),
            (Some(_), None) => return Err(connect_protocol::Error::forbidden()),
            (None, _) => self.store.try_native_media_read(
                &entry.origin,
                source.as_ref().map(|(_, entry)| entry.origin.as_ref()),
            ),
        });
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn temporary_packet_read_rechecks_and_never_reuses_a_prior_grant() {
        let mut calls = 0;
        let read = packet_read_until(
            std::time::Instant::now() + std::time::Duration::from_secs(1),
            || {
                calls += 1;
                if calls == 1 {
                    Err(connect_protocol::Error::new("authority-busy", "held"))
                } else {
                    Err::<(), _>(connect_protocol::Error::forbidden())
                }
            },
        );
        assert_eq!(calls, 2);
        assert_eq!(read.unwrap_err().code, "forbidden");
        let mut calls = 0;
        let read = packet_read_until(
            std::time::Instant::now() + std::time::Duration::from_secs(1),
            || {
                calls += 1;
                if calls == 1 {
                    Err(connect_protocol::Error::new("authority-busy", "held"))
                } else {
                    Ok("fresh-current-source")
                }
            },
        )
        .unwrap();
        assert_eq!(read, "fresh-current-source");
        assert_eq!(calls, 2);
    }
    #[test]
    fn permanent_packet_refusal_is_immediate_and_busy_work_is_bounded() {
        let mut calls = 0;
        assert_eq!(
            packet_read(|| {
                calls += 1;
                Err::<(), _>(connect_protocol::Error::new("storage", "failed"))
            })
            .unwrap_err()
            .code,
            "storage"
        );
        assert_eq!(calls, 1);
        let mut calls = 0;
        assert_eq!(
            packet_read_until(std::time::Instant::now(), || {
                calls += 1;
                Err::<(), _>(connect_protocol::Error::new("authority-busy", "held"))
            })
            .unwrap_err()
            .code,
            "authority-busy"
        );
        assert_eq!(calls, 1);
    }
}
