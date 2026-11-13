//! One encoded original across definite local queue exclusion, never a new send.
use super::*;
pub(super) struct Encoded {
    bytes: Vec<u8>,
    sequence: u32,
    binding: media::Binding,
    generation: (u64, Option<u64>),
}
impl Encoded {
    pub(super) fn capture(
        cache: &std::sync::Mutex<NativeCache>,
        key: &streams::StreamKey,
        packet: &MediaPacket,
    ) -> Result<Self> {
        let cache = cache
            .lock()
            .map_err(|_| Error::media(ErrorKind::Protocol, false))?;
        let (binding, generation) = native_binding_in(&cache, key)?;
        let generation = generation.ok_or_else(|| Error::media(ErrorKind::Protocol, false))?;
        let (bytes, sequence) = media::encode(&binding, packet)?;
        Ok(Self {
            bytes,
            sequence,
            binding,
            generation,
        })
    }
}
pub(super) struct Pending {
    command: MediaCommand,
    next_retry_at: tokio::time::Instant,
}
impl Pending {
    pub(super) fn new(command: MediaCommand) -> Self {
        let next_retry_at = command.expires;
        Self {
            command,
            next_retry_at,
        }
    }
    pub(super) fn wakeup(&self) -> tokio::time::Instant {
        self.next_retry_at
    }
    fn require_current(&self, stop: &CancellationToken) -> Result<()> {
        if self.command.reply.is_closed()
            || self.command.signal.is_cancelled()
            || stop.is_cancelled()
        {
            return Err(Error::media(ErrorKind::Cancelled, false));
        }
        if tokio::time::Instant::now() >= self.command.expires {
            return Err(Error::media(ErrorKind::Deadline, false));
        }
        Ok(())
    }
    pub(super) fn try_send(
        &mut self,
        peer: &crate::canonical::OwnedNativeIo,
        cache: &std::sync::Mutex<NativeCache>,
        stop: &CancellationToken,
    ) -> Result<Option<LocalMediaSubmission>> {
        self.try_send_with(cache, stop, |bytes| peer.send(bytes))
    }
    fn try_send_with(
        &mut self,
        cache: &std::sync::Mutex<NativeCache>,
        stop: &CancellationToken,
        send: impl FnOnce(&[u8]) -> crate::Result<()>,
    ) -> Result<Option<LocalMediaSubmission>> {
        let result = self.attempt_with(cache, stop, send);
        if matches!(&result, Ok(None)) {
            // Only an ACTUAL refused attempt can schedule the next retry.
            // Unrelated receive/control branches must not renew this wakeup.
            self.next_retry_at = self
                .command
                .expires
                .min(tokio::time::Instant::now() + Duration::from_millis(1));
        }
        result
    }
    fn attempt_with(
        &self,
        cache: &std::sync::Mutex<NativeCache>,
        stop: &CancellationToken,
        send: impl FnOnce(&[u8]) -> crate::Result<()>,
    ) -> Result<Option<LocalMediaSubmission>> {
        self.require_current(stop)?;
        let cache = match cache.try_lock() {
            Ok(cache) => cache,
            Err(std::sync::TryLockError::WouldBlock) => return Ok(None),
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err(Error::media(ErrorKind::Protocol, false))
            }
        };
        let MediaSend::Packet(encoded) = &self.command.packet else {
            return Err(Error::media(ErrorKind::Protocol, false));
        };
        let (current, generation) = native_binding_in(&cache, &self.command.key)?;
        if generation != Some(encoded.generation) || !same_binding(&current, &encoded.binding) {
            return Err(Error::media(ErrorKind::Conflict, false));
        }
        // Budget may have elapsed during lock acquisition/preemption. Check
        // AGAIN immediately before the single atomic enqueue, with exact
        // metadata still pinned. No reader/lock survives a pending return.
        self.require_current(stop)?;
        match send(&encoded.bytes) {
            Ok(()) => Ok(Some(LocalMediaSubmission {
                datagram_sequence: encoded.sequence,
            })),
            Err(crate::ClientError::MediaBackpressure) => Ok(None),
            Err(error) => Err(Error::media(Error::transport(error, None).kind, false)),
        }
    }
    pub(super) fn finish(self, result: Result<LocalMediaSubmission>) {
        let _ = self.command.reply.send(result);
    }
}
fn same_binding(a: &media::Binding, b: &media::Binding) -> bool {
    a.clock == b.clock
        && a.stream.id == b.stream.id
        && a.stream.medium == b.stream.medium
        && a.stream.direction == b.stream.direction
        && a.stream.codec == b.stream.codec
        && a.stream.stream_local_id == b.stream.stream_local_id
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicUsize, Mutex};
    fn fixture() -> (streams::StreamKey, Mutex<NativeCache>) {
        let key = streams::StreamKey {
            conversation_id: "room".parse().unwrap(),
            session_id: "session".parse().unwrap(),
            connection_id: "connection".parse().unwrap(),
            stream_id: "audio".parse().unwrap(),
        };
        let revision = StreamRevision::try_from(1).unwrap();
        let entry = NativeConnection {
            session: key.session_id.clone(),
            conversation: key.conversation_id.clone(),
            selected: NativeNegotiation {
                video_feedback: Default::default(),
                source_clock: true,
                stream_revision: revision,
                streams: vec![NegotiatedStream {
                    id: key.stream_id.clone(),
                    medium: Medium::Audio,
                    direction: Direction::Sendrecv,
                    codec: serde_json::json!({"name":"opus","params":{"payload_type":111,"sample_rate":48000,"channels":1}}),
                    stream_local_id: Some(7),
                }],
            },
            ended: false,
            known_revision: revision,
            participant: "local".parse().unwrap(),
            wire_handles: [(key.stream_id.clone(), 7)].into(),
            local_capabilities: serde_json::json!({}),
            server_capabilities: serde_json::json!({}),
            reserved_handles: [7].into(),
            generations: [(key.stream_id.clone(), 1)].into(),
            prepared: None,
            gates: Default::default(),
            selectors: Default::default(),
            reconciliation: None,
        };
        (
            key.clone(),
            Mutex::new([(key.connection_id.clone(), entry)].into()),
        )
    }
    fn pending(
        key: &streams::StreamKey,
        cache: &Mutex<NativeCache>,
        until: tokio::time::Instant,
    ) -> (Pending, oneshot::Receiver<Result<LocalMediaSubmission>>) {
        let mut raw = vec![0x80, 111, 0, 42];
        raw.extend_from_slice(&960u32.to_be_bytes());
        raw.extend_from_slice(&41u32.to_be_bytes());
        raw.extend_from_slice(&[0xf8, 0xff, 0xfe]);
        let packet = MediaPacket::Rtp(RtpPacket::new(99, raw).unwrap());
        let encoded = Encoded::capture(cache, key, &packet).unwrap();
        let (reply, response) = oneshot::channel();
        (
            Pending::new(MediaCommand {
                key: key.clone(),
                packet: MediaSend::Packet(encoded),
                expires: until,
                signal: CancellationToken::new(),
                reply,
            }),
            response,
        )
    }
    #[tokio::test]
    async fn deferred_sdk_original_retains_one_encoding_sequence_and_reply_owner() {
        let (key, cache) = fixture();
        let stop = CancellationToken::new();
        let (mut pending, response) = pending(
            &key,
            &cache,
            tokio::time::Instant::now() + Duration::from_secs(1),
        );
        let MediaSend::Packet(encoded) = &pending.command.packet else {
            unreachable!()
        };
        let original = encoded.bytes.clone();
        let pointer = encoded.bytes.as_ptr();
        let sequence = encoded.sequence;
        let calls = AtomicUsize::new(0);
        for _ in 0..2 {
            let result = pending
                .try_send_with(&cache, &stop, |bytes| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(bytes, original);
                    assert_eq!(bytes.as_ptr(), pointer);
                    Err(crate::ClientError::MediaBackpressure)
                })
                .unwrap();
            assert!(result.is_none());
            assert!(
                cache.try_lock().is_ok(),
                "no metadata lock may survive local capacity wait"
            );
            let fixed_wakeup = pending.wakeup();
            // Model independent receive/control turns while queue capacity may
            // recover. They must not push the pending original's wakeup out.
            for _ in 0..3 {
                tokio::time::sleep(Duration::from_millis(1)).await;
                assert_eq!(
                    pending.wakeup(),
                    fixed_wakeup,
                    "unrelated activity starved original retry"
                );
            }
            assert!(tokio::time::Instant::now() >= fixed_wakeup);
        }
        let done = pending
            .try_send_with(&cache, &stop, |bytes| {
                calls.fetch_add(1, Ordering::SeqCst);
                assert_eq!(bytes, original);
                assert_eq!(bytes.as_ptr(), pointer);
                Ok(())
            })
            .unwrap()
            .unwrap();
        assert_eq!(done.datagram_sequence, sequence);
        pending.finish(Ok(done)); // Consumes the original; no submitted retry.
        assert_eq!(response.await.unwrap().unwrap().datagram_sequence, 99);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }
    #[test]
    fn pending_cache_exclusion_never_blocks_control_and_expiry_or_cancel_prevents_enqueue() {
        for cancel in [false, true] {
            let (key, cache) = fixture();
            let stop = CancellationToken::new();
            let until = tokio::time::Instant::now() + Duration::from_millis(200);
            let (mut pending, _response) = pending(&key, &cache, until);
            let held = cache.lock().unwrap();
            let result = pending.try_send_with(&cache, &stop, |_| {
                panic!("held-cache input reached enqueue")
            });
            assert!(
                matches!(result, Ok(None)),
                "cache contention must not park the shared worker"
            );
            if cancel {
                pending.command.signal.cancel();
            } else {
                std::thread::sleep(Duration::from_millis(210));
            }
            drop(held);
            let result = pending.try_send_with(&cache, &stop, |_| {
                panic!("expired/cancelled original reached enqueue")
            });
            let error = result.err().unwrap();
            assert_eq!(
                error.kind,
                if cancel {
                    ErrorKind::Cancelled
                } else {
                    ErrorKind::Deadline
                }
            );
            assert_eq!(error.certainty, Certainty::NotSubmitted);
        }
    }
    #[test]
    fn pending_binding_changes_and_noncapacity_errors_are_terminal_without_reexecution() {
        for changed in [0, 1, 2, 3] {
            let (key, cache) = fixture();
            let stop = CancellationToken::new();
            let (mut pending, _response) = pending(
                &key,
                &cache,
                tokio::time::Instant::now() + Duration::from_secs(1),
            );
            {
                let mut entries = cache.lock().unwrap();
                let entry = entries.get_mut(&key.connection_id).unwrap();
                match changed {
                    0 => entry.ended = true,
                    1 => {
                        entry.generations.insert(key.stream_id.clone(), 2);
                    }
                    2 => entry.selected.streams[0].stream_local_id = Some(8),
                    _ => entry.selected.streams[0].direction = Direction::Recvonly,
                }
            }
            let result = pending.try_send_with(&cache, &stop, |_| {
                panic!("changed binding reached local enqueue")
            });
            let error = result.err().unwrap();
            assert!(matches!(
                error.kind,
                ErrorKind::Unsupported | ErrorKind::Conflict
            ));
            assert_eq!(error.certainty, Certainty::NotSubmitted);
        }
        // An actual newer hold/resume observation restores all effective
        // binding fields. Only its relevant gate stamp can reject this old
        // encoded original; do not let restored direction revive it.
        {
            let (key, cache) = fixture();
            let stop = CancellationToken::new();
            let (mut pending, _response) = pending(
                &key,
                &cache,
                tokio::time::Instant::now() + Duration::from_secs(1),
            );
            let inbox = media::Inbox::default();
            {
                let entries = cache.lock().unwrap();
                let entry = entries.get(&key.connection_id).unwrap();
                inbox
                    .install(
                        &key.conversation_id,
                        &key.session_id,
                        &key.connection_id,
                        &entry.selected,
                    )
                    .unwrap();
            }
            for (action, revision) in [("hold", 2u64), ("resume", 3u64)] {
                let operation = OperationKey {
                    operation_id: format!("pending-{action}").parse().unwrap(),
                    command: "connection.update".into(),
                    scope: OperationScope {
                        conversation_id: Some(key.conversation_id.clone()),
                        session_id: Some(key.session_id.clone()),
                        connection_id: Some(key.connection_id.clone()),
                    },
                };
                let intent = if action == "hold" {
                    serde_json::json!({"action":"hold","details":{"direction":"inactive"},"streams":["audio"]})
                } else {
                    serde_json::json!({"action":"resume","streams":["audio"]})
                };
                let result = serde_json::json!({"connection_id":"connection","connection_generation":1,"stream_revision":revision,"resources_changed":false,"action":action,"gate_streams":[{"binding":{"stream_id":"audio","medium":"audio","direction":"sendrecv","codec":{"name":"opus","params":{"payload_type":111,"sample_rate":48000,"channels":1}},"stream_local_id":7,"stream_generation":1,"connection_generation":1},"state":if action=="hold"{"paused"}else{"active"},"source_muted":false,"held_direction":null}]});
                let frame=crate::canonical::Envelope::from_value(serde_json::json!({"v":2,"type":"operation.updated","id":format!("fact-{action}"),"ts":chrono::Utc::now(),"cid":"room","sid":"session","connid":"connection","operation_id":operation.operation_id.as_str(),"in_reply_to":"original-control","payload":{"operation_id":operation.operation_id.as_str(),"state":"completed","revision":1,"result":result}})).unwrap();
                controls::observe(&cache, &inbox, &operation, &intent, revision - 1, &frame)
                    .unwrap();
            }
            let MediaSend::Packet(encoded) = &pending.command.packet else {
                unreachable!()
            };
            {
                let entries = cache.lock().unwrap();
                let (current, stamp) = native_binding_in(&entries, &key).unwrap();
                assert!(
                    same_binding(&current, &encoded.binding),
                    "hold/resume must restore exact effective direction/hop/codec"
                );
                let stamp = stamp.unwrap();
                assert_eq!(stamp.0, encoded.generation.0);
                assert_ne!(
                    stamp.1, encoded.generation.1,
                    "newer actual gate stamp never reached fixture"
                );
            }
            let error = pending
                .try_send_with(&cache, &stop, |_| {
                    panic!("hold/resume revived old pending media")
                })
                .err()
                .unwrap();
            assert_eq!(error.kind, ErrorKind::Conflict);
            assert_eq!(error.certainty, Certainty::NotSubmitted);
        }
        let (key, cache) = fixture();
        let stop = CancellationToken::new();
        let (mut pending, _response) = pending(
            &key,
            &cache,
            tokio::time::Instant::now() + Duration::from_secs(1),
        );
        let calls = AtomicUsize::new(0);
        let result = pending.try_send_with(&cache, &stop, |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(crate::ClientError::Transport)
        });
        let error = result.err().unwrap();
        assert_eq!(error.kind, ErrorKind::TransportUncertain);
        assert_eq!(error.certainty, Certainty::NotSubmitted);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
