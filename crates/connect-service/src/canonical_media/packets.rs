//! Current per-packet policy; only exact active source file custody is reused.
use super::*;
#[cfg(feature = "test-support")]
use rvoip_core::staged_streams::audio_diagnostic::{self, Stage};

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
                // Busy packet readers otherwise repeatedly contend for the
                // same admission files while metadata writers try to finish.
                // Pause inside the original budget, then construct a fresh
                // read; no prior permission or failed partial custody survives.
                let remaining = until.saturating_duration_since(std::time::Instant::now());
                std::thread::sleep(remaining.min(std::time::Duration::from_micros(100)));
            }
            result => return result,
        }
    }
}
// Measurement only: actual authority/expiry clocks remain unconditional in
// their owners. Healthy packet delivery does not read a tracing clock unless
// one of the existing elapsed-time events can be recorded.
fn packet_trace_clock() -> Option<std::time::Instant> {
    (tracing::enabled!(target: "connect_service::canonical_packet", tracing::Level::TRACE)
        || tracing::enabled!(target: "connect_service::packet_refusal", tracing::Level::DEBUG))
    .then(std::time::Instant::now)
}
// Both publications belong to this one IO lease. Admit target before source;
// a source refusal drops the already-admitted target, and success retains the
// same target/source destruction order without allocating a Vec per packet.
fn publication_pair<T>(
    target: impl FnOnce() -> ControlResult<Option<T>>,
    source: impl FnOnce() -> ControlResult<Option<T>>,
) -> ControlResult<[Option<T>; 2]> {
    let target = target()?;
    let source = source()?;
    Ok([target, source])
}
// A continuation lookup can straddle ordinary publication after its registry
// Arc was captured. Retained media owners already retry NotReady using the same
// packet and earliest deadline. Keep original-ingress MetadataBusy distinct:
// semantic NotReady there is a permanent refusal, not permission to recapture.
fn packet_error(
    original: bool,
    purpose: &MediaPurpose<'_>,
    error: StreamControlError,
) -> StreamControlError {
    if !original
        && matches!(
            purpose,
            MediaPurpose::Publish | MediaPurpose::Receive { .. }
        )
        && matches!(error, StreamControlError::MetadataBusy)
    {
        StreamControlError::NotReady
    } else {
        error
    }
}

impl Policy {
    pub(super) fn packet_current(
        &self,
        scope: &MediaScope<'_>,
        held: Option<Arc<NativeMediaRead>>,
        queued: bool,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.packet_current_inner(scope, held, queued, false)
    }
    pub(super) fn packet_current_inner(
        &self,
        scope: &MediaScope<'_>,
        held: Option<Arc<NativeMediaRead>>,
        queued: bool,
        original: bool,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        let result = self
            .packet_current_observed(scope, held, queued, original)
            .map_err(|error| packet_error(original, &scope.purpose, error))
            .inspect_err(|error| {
                tracing::debug!(target: "connect_service::packet_refusal",
                    connection = %scope.connection,
                    publishing = matches!(scope.purpose, MediaPurpose::Publish),
                    queued, original, ?error, "Current packet policy refused");
            });
        #[cfg(feature = "test-support")]
        audio_diagnostic::result(scope.connection, Stage::PolicyResult, result.as_ref().err());
        result
    }
    fn packet_current_observed(
        &self,
        scope: &MediaScope<'_>,
        held: Option<Arc<NativeMediaRead>>,
        queued: bool,
        original: bool,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        // Registry leaves load immutable Published maps. They cannot report
        // mutex NotReady; preserve downstream semantic refusal/retry handling.
        if let Some(target) = self.initial_target(scope.connection)? {
            return target.current(scope);
        }
        if let Some(target) = self.telephone_target(scope.connection)? {
            return target.current(scope);
        }
        let target_continuation = if let Some(target) = self.replacement_target(scope.connection)? {
            match target.current_continuation()? {
                Some(current) if !matches!(scope.purpose, MediaPurpose::Control) => Some(current),
                _ => {
                    return match (held.as_deref(), &scope.purpose) {
                        (Some(held), MediaPurpose::Receive { .. }) => {
                            target.current_packet_read(scope, held)
                        }
                        _ => target.current(scope),
                    }
                }
            }
        } else {
            None
        };
        let entry = match &target_continuation {
            Some(current) => current.entry.clone(),
            None => self.entry(scope.connection)?,
        };
        self.scope(scope, &entry).map_err(|error|{tracing::trace!(target:"connect_service::canonical_packet",connection=entry.origin.connection_id(),?error,"Native packet scope refused");error})?;
        if let MediaPurpose::Receive { source } = scope.purpose {
            if let Some(initial) = self.initial_target(&source.connection)? {
                return initial.native_receive(scope, source, &entry);
            }
            if let Some(telephone) = self.telephone_target(&source.connection)? {
                return telephone.native_receive(scope, source, &entry);
            }
        }
        let source_continuation = match scope.purpose {
            MediaPurpose::Receive { source } => self
                .replacement_target(&source.connection)?
                .map(|target| target.current_continuation())
                .transpose()?
                .flatten(),
            _ => None,
        };
        let source = match scope.purpose {
            MediaPurpose::Receive { source } => Some((
                source,
                match &source_continuation {
                    Some(current) => current.entry.clone(),
                    None => self.entry(&source.connection)?,
                },
            )),
            _ => None,
        };
        let started = packet_trace_clock();
        if let (Some(held), Some((descriptor, source))) = (held.as_deref(), source.as_ref()) {
            source_matches(descriptor, source, held.target())?;
        }
        let result = (|| {
            if held.is_some() && source.is_none() && !matches!(scope.purpose, MediaPurpose::Publish)
            {
                return Err(connect_protocol::Error::forbidden());
            }
            entry.snapshot_read_for(
                &self.store,
                source.as_ref().map(|(_, source)| source),
                held.as_deref(),
                target_continuation.as_ref(),
                source_continuation.as_ref(),
            )
        })();
        tracing::trace!(target: "connect_service::canonical_packet",
            connection = entry.origin.connection_id(),
            elapsed_us = started.as_ref().map_or(0, |started| started.elapsed().as_micros() as u64),
            publishing = matches!(scope.purpose, MediaPurpose::Publish),
            owned_file_custody = held.is_some(),
            role = result.as_ref().ok().map(|r|r.target().presence_role.as_str()),
            detail = result.as_ref().err().map(|e|e.message.as_str()),
            receiving = source.is_some(),
            private = entry.origin.connection_id().starts_with("pvc_"),
            problem = result.as_ref().err().map(|e| e.code.as_str()),
            "Current native packet authority");
        let read = result.map_err(|error| {
            tracing::debug!(target: "connect_service::packet_refusal",
                connection = %scope.connection,
                publishing = matches!(scope.purpose, MediaPurpose::Publish),
                queued, original, owned_file_custody = held.is_some(),
                code = %error.code, detail = %error.message,
                elapsed_us = started.as_ref().map_or(0, |started| started.elapsed().as_micros() as u64),
                "Current packet Store read refused");
            if original && error.code == "authority-busy" {
                StreamControlError::MetadataBusy
            } else {
                packet_authority_error(error)
            }
        });
        #[cfg(feature = "test-support")]
        audio_diagnostic::result(scope.connection, Stage::PolicyStore, read.as_ref().err());
        let read = read?;
        let read_state = (|| {
            if !read.current() {
                return Err(StreamControlError::Authority);
            }
            // Local engine preparation can precede the durable readiness commit.
            // Fresh publication/delivery cannot use that intermediate state.
            if matches!(scope.purpose, MediaPurpose::Publish) && read.target().publication_suspended
            {
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
            Ok(())
        })();
        #[cfg(feature = "test-support")]
        audio_diagnostic::result(
            scope.connection,
            Stage::PolicyReadState,
            read_state.as_ref().err(),
        );
        read_state?;
        let owner = entry.owner(read.target(), scope.principal, source.is_some()).map_err(|error|{tracing::trace!(target:"connect_service::canonical_packet",connection=entry.origin.connection_id(),?error,"Native packet owner refused");error});
        #[cfg(feature = "test-support")]
        audio_diagnostic::result(scope.connection, Stage::PolicyOwner, owner.as_ref().err());
        let owner = owner?;
        let owner = (|| entry
            .activation()?
            .ok_or(StreamControlError::NotReady)?
            .current(owner))().map_err(|error|{tracing::trace!(target:"connect_service::canonical_packet",connection=entry.origin.connection_id(),?error,"Native packet activation refused");error});
        #[cfg(feature = "test-support")]
        audio_diagnostic::result(
            scope.connection,
            Stage::PolicyActivation,
            owner.as_ref().err(),
        );
        let owner = owner?;
        // Current Store readers exist before these short Core metadata checks.
        // Capture no Core mutex across authorization acquisition or packet IO.
        let continuations = publication_pair(
            || {
                let Some(current) = &target_continuation else {
                    return Ok(None);
                };
                let publication = current.current(&owner);
                #[cfg(feature = "test-support")]
                audio_diagnostic::result(
                    scope.connection,
                    Stage::PolicyPublication,
                    publication.as_ref().err(),
                );
                publication.map(Some)
            },
            || {
                let Some(current) = &source_continuation else {
                    return Ok(None);
                };
                let source_owner = current.entry.owner(
                    read.source().ok_or(StreamControlError::Authority)?,
                    &current.entry.principal,
                    false,
                )?;
                let source_owner = current
                    .entry
                    .activation()?
                    .ok_or(StreamControlError::NotReady)?
                    .current(source_owner)?;
                let publication = current.current(&source_owner);
                #[cfg(feature = "test-support")]
                audio_diagnostic::result(
                    source_owner.connection.connection(),
                    Stage::PolicyPublication,
                    publication.as_ref().err(),
                );
                publication.map(Some)
            },
        )?;
        Ok(Box::new(
            Fresh::new(owner, read, held).with_continuations(continuations),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packet_trace_clock_only_observes_existing_elapsed_event_targets() {
        use tracing_subscriber::prelude::*;
        tracing::subscriber::with_default(tracing::subscriber::NoSubscriber::default(), || {
            assert!(packet_trace_clock().is_none());
        });
        for (directives, expected) in [
            ("off,connect_service::canonical_packet=trace", true),
            ("off,connect_service::packet_refusal=debug", true),
            ("off,connect_service::canonical_packet=debug", false),
            ("off,connect_service::packet_refusal=info", false),
            ("off,unrelated_packet=trace", false),
        ] {
            let subscriber = tracing_subscriber::registry()
                .with(tracing_subscriber::EnvFilter::try_new(directives).unwrap());
            tracing::subscriber::with_default(subscriber, || {
                assert_eq!(packet_trace_clock().is_some(), expected, "{directives}");
            });
        }
    }
    #[test]
    fn publication_pair_retains_both_until_io_drop_in_original_order() {
        use std::{cell::RefCell, rc::Rc};
        struct Publication(&'static str, Rc<RefCell<Vec<&'static str>>>);
        impl Drop for Publication {
            fn drop(&mut self) {
                self.1.borrow_mut().push(self.0);
            }
        }
        let dropped = Rc::new(RefCell::new(Vec::new()));
        let calls = RefCell::new(Vec::new());
        let pair = publication_pair(
            || {
                calls.borrow_mut().push("target");
                Ok(Some(Publication("target", dropped.clone())))
            },
            || {
                calls.borrow_mut().push("source");
                assert!(dropped.borrow().is_empty());
                Ok(Some(Publication("source", dropped.clone())))
            },
        )
        .unwrap();
        assert_eq!(*calls.borrow(), ["target", "source"]);
        assert!(dropped.borrow().is_empty());
        drop(pair);
        assert_eq!(*dropped.borrow(), ["target", "source"]);
        let empty = publication_pair::<Publication>(|| Ok(None), || Ok(None)).unwrap();
        assert!(empty.iter().all(Option::is_none));
    }
    #[test]
    fn publication_pair_refusal_releases_target_and_never_admits_later_source() {
        use std::cell::Cell;
        struct Publication<'a>(&'a Cell<usize>);
        impl Drop for Publication<'_> {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }
        let dropped = Cell::new(0);
        let source_called = Cell::new(false);
        let result = publication_pair::<Publication<'_>>(
            || Err(StreamControlError::Authority),
            || {
                source_called.set(true);
                Ok(None)
            },
        );
        assert!(matches!(result, Err(StreamControlError::Authority)));
        assert!(!source_called.get());
        let result = publication_pair(
            || Ok(Some(Publication(&dropped))),
            || {
                assert_eq!(dropped.get(), 0);
                Err(StreamControlError::NotRecipient)
            },
        );
        assert!(matches!(result, Err(StreamControlError::NotRecipient)));
        assert_eq!(dropped.get(), 1);
    }
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

    fn packet_source_descriptor() -> MediaSource {
        MediaSource {
            conversation: rvoip_core::ids::ConversationId::from_string("conversation"),
            session: SessionId::from_string("session"),
            connection: ConnectionId::from_string("source"),
            participant: rvoip_core::ids::ParticipantId::from_string("participant"),
            membership_epoch: 1,
            connection_generation: 1,
            logical_id: rvoip_core::ids::StreamId::from_string("logical"),
            transport_id: rvoip_core::ids::StreamId::from_string("transport"),
            stream_generation: 1,
        }
    }

    #[test]
    fn publication_metadata_busy_uses_only_nonoriginal_media_retry_contract() {
        let source = packet_source_descriptor();
        for purpose in [
            MediaPurpose::Control,
            MediaPurpose::Publish,
            MediaPurpose::Receive { source: &source },
        ] {
            for original in [false, true] {
                let mapped = packet_error(original, &purpose, StreamControlError::MetadataBusy);
                if !original && !matches!(purpose, MediaPurpose::Control) {
                    assert!(matches!(mapped, StreamControlError::NotReady));
                } else {
                    assert!(matches!(mapped, StreamControlError::MetadataBusy));
                }
            }
        }
    }

    #[test]
    fn publication_retry_mapping_preserves_every_other_refusal_and_payload() {
        let source = packet_source_descriptor();
        let errors: [fn() -> StreamControlError; 9] = [
            || {
                StreamControlError::Model(
                    rvoip_core::stream_lifecycle::MediaTransitionError::Conflict,
                )
            },
            || StreamControlError::Unsupported,
            || StreamControlError::Authority,
            || StreamControlError::NotRecipient,
            || StreamControlError::Cancelled,
            || StreamControlError::Deadline,
            || StreamControlError::NotReady,
            || StreamControlError::Backpressure,
            || StreamControlError::CleanupUnconfirmed,
        ];
        for purpose in [
            MediaPurpose::Control,
            MediaPurpose::Publish,
            MediaPurpose::Receive { source: &source },
        ] {
            for original in [false, true] {
                for make in errors {
                    let error = make();
                    let before = format!("{error:?}");
                    let mapped = packet_error(original, &purpose, error);
                    assert_eq!(format!("{mapped:?}"), before);
                }
            }
        }
    }
}
