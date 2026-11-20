//! Read-only classification of the SAME signed private receiver and public peers.
//! Neither this observer nor copied readiness metadata authorizes any media.
use super::*;
use connect_client::canonical::NativeReplacementReadiness;
use rvoip_uctp::substrate::{pack_rtp_datagram, unpack_rtp_datagram, RtpDatagram, RtpMediaPayload};

pub(super) struct Evidence {
    readiness: NativeReplacementReadiness,
    observation: connect_service::CanonicalReplacementAudioObserverForTest,
    candidate_runtime: String,
    retained_runtime: String,
    retained_runtime_epoch: String,
    old_runtime: String,
    retained_generation: u64,
    private_sequences: Vec<u32>,
}
impl Evidence {
    pub(super) async fn begin(
        f: &Fixture,
        record: &str,
        operation: &str,
        candidate_runtime: &str,
        peers: &mut [Peer],
        until: tokio::time::Instant,
    ) -> Self {
        assert_eq!(peers.len(), 3);
        let readiness = peers[2].client.native_replacement_readiness().unwrap();
        assert_eq!(readiness.record_id(), record);
        assert_eq!(readiness.operation_id(), Some(operation));
        assert_eq!(readiness.source_generation(), 1);
        assert_eq!(readiness.target_generation(), 2);
        assert_eq!(readiness.stream_revision(), 2);
        assert!(
            readiness.video().is_none(),
            "these five controls are audio-only"
        );
        assert_eq!(readiness.audio().stream_local_id, peers[2].handle);
        assert_eq!(readiness.audio().payload_type, 0);
        readiness.audio().native_egress.validate().unwrap();
        let originals = activation::pre_core::original_bindings(f);
        let retained = originals
            .iter()
            .find(|row| row.0 == "codec-source")
            .unwrap();
        let old = originals
            .iter()
            .find(|row| row.0 == "codec-target")
            .unwrap();
        // Tuple .3 is the installation/process epoch, NOT the Core connection.
        // The immutable offer mapping is the actual Core identity used by
        // NativeMediaOrigin.runtime_connection_id() and MediaSource.connection.
        let retained_runtime_epoch = retained.3.clone().unwrap();
        assert_eq!(retained_runtime_epoch, f.server.store.runtime_epoch());
        assert_eq!(old.3.as_deref(), Some(f.server.store.runtime_epoch()));
        let (retained_runtime, old_runtime) = {
            let database = rusqlite::Connection::open(&f.config.database).unwrap();
            let core_mapping = |wire: &str, generation: u64, runtime_epoch: &str| -> String {
                database.query_row(
                    "SELECT mapping.runtime_connection_id FROM connection_offers mapping JOIN connections c ON c.id=mapping.connection_id JOIN native_connection_lifetimes life ON life.connection_id=c.id AND life.generation=c.generation AND life.runtime_epoch=c.observed_runtime_id AND life.retired=0 JOIN sessions session ON session.id=c.session_id JOIN operations root ON root.record_id=?4 AND root.conversation_id=session.conversation_id AND root.session_id=c.session_id WHERE c.id=?1 AND c.generation=?2 AND c.observed_runtime_id=?3 AND c.state IN ('ready','active') AND c.readiness_json IS NOT NULL",
                    rusqlite::params![wire, generation, runtime_epoch, record],
                    |row| row.get(0),
                ).unwrap()
            };
            (
                core_mapping(&retained.0, retained.2, &retained_runtime_epoch),
                core_mapping(&old.0, old.2, old.3.as_deref().unwrap()),
            )
        }; // All SQLite connection/statement custody ends BEFORE any await.
        assert_ne!(retained_runtime, old_runtime);
        assert_ne!(retained_runtime, retained_runtime_epoch);
        let observation = loop {
            match f
                .server
                .service
                .canonical_replacement_audio_observer_for_test(record)
            {
                Ok(observation) => break observation,
                Err(error) if error.code == "authority-busy" => {
                    assert!(tokio::time::Instant::now() < until);
                    tokio::time::sleep_until(
                        until.min(tokio::time::Instant::now() + Duration::from_millis(1)),
                    )
                    .await;
                }
                Err(error) => panic!("exact original preparation observation refused: {error:?}"),
            }
        };
        let mut result = Self {
            readiness,
            observation,
            candidate_runtime: candidate_runtime.into(),
            retained_generation: retained.2,
            retained_runtime,
            retained_runtime_epoch,
            old_runtime,
            private_sequences: Vec::new(),
        };
        result.state(f, until, false).await;
        // The readiness publisher has stopped. Drain its actual finite old
        // source tail from ALL readers, without giving it fresh probe credit.
        result
            .window(
                f,
                peers,
                None,
                &[1],
                false,
                false,
                until,
                "signed-readiness-backlog",
            )
            .await;
        result
    }
    async fn state(&self, _f: &Fixture, until: tokio::time::Instant, closed: bool) {
        let observed = loop {
            match self.observation.snapshot() {
                Ok(value) => break value,
                Err(error) if error.code == "authority-busy" => {
                    assert!(
                        tokio::time::Instant::now() < until,
                        "actual private metadata stayed Busy"
                    );
                    tokio::time::sleep_until(
                        until.min(tokio::time::Instant::now() + Duration::from_millis(1)),
                    )
                    .await;
                }
                Err(error) => panic!("actual private metadata refused: {error:?}"),
            }
        };
        if closed {
            assert!(observed.cancelled && observed.committed!=Some(true),
                "negative observation requires actual original cancellation and no known model commit");
            assert!(observed.audio.as_ref().is_none_or(|audio|
                !audio.private_current && audio.source_muted && audio.ordinary_programme_empty),
                "cancelled route is neither retired nor muted/private-only with empty ordinary programme");
            return;
        }
        assert!(observed.committed == Some(false) && !observed.cancelled);
        let audio = observed
            .audio
            .as_ref()
            .expect("exact original private receiver resources retired");
        assert!(
            audio.private_current && audio.source_muted && audio.ordinary_programme_empty,
            "pre-cut receiver must be current PRIVATE only, muted and no ordinary programme"
        );
        assert_eq!(audio.target_connection.as_str(), self.candidate_runtime);
        let binding = &audio.target;
        // Native binding_wire uses requested.logical_id for signed strm_id;
        // the private transport UUID is distinct and checked against the
        // actual native writer below, never guessed from the logical ID.
        assert_eq!(
            binding.negotiated.requested.logical_id.as_str(),
            self.readiness.audio().stream_id
        );
        assert_eq!(
            binding.stream_local_id,
            self.readiness.audio().stream_local_id
        );
        assert_eq!(
            binding.connection_generation,
            self.readiness.target_generation()
        );
        assert_eq!(
            binding.generation,
            self.readiness.audio().native_egress.stream_generation
        );
        assert_eq!(
            binding.negotiated.requested.logical_id.as_str(),
            "replacement-voice"
        );
        assert!(binding.source_ready && binding.sink_ready);
        let identity = audio
            .native_egress
            .as_ref()
            .expect("actual signed native writer retired");
        assert_eq!(identity.connection().as_str(), self.candidate_runtime);
        assert_eq!(
            identity.logical().as_str(),
            self.readiness.audio().stream_id
        );
        assert_eq!(identity.transport(), &binding.transport_id);
        assert_eq!(identity.handle(), binding.stream_local_id);
        assert_eq!(identity.generation(), binding.generation);
        assert_eq!(
            identity.connection_generation(),
            binding.connection_generation
        );
        assert_eq!(
            format!("{:032x}", identity.incarnation()),
            self.readiness.audio().native_egress.physical_incarnation
        );
        assert_eq!(audio.sources.len(), 1, "no source permission widening");
        let source = &audio.sources[0];
        assert_eq!(source.connection.as_str(), self.retained_runtime);
        assert_eq!(source.connection_generation, self.retained_generation);
        assert_eq!(source.logical_id.as_str(), "codec-voice");
        assert_ne!(source.connection.as_str(), self.candidate_runtime);
        // Counterfactuals use this ACTUAL private source descriptor: wrong
        // process-epoch domain, actual foreign Core owner and stale generation
        // must not match merely because the logical audio ID is unchanged.
        let matches_retained = |core: &str, generation: u64| {
            source.connection.as_str() == core
                && source.connection_generation == generation
                && source.logical_id.as_str() == "codec-voice"
        };
        assert!(matches_retained(
            &self.retained_runtime,
            self.retained_generation
        ));
        assert!(!matches_retained(
            &self.retained_runtime_epoch,
            self.retained_generation
        ));
        assert!(!matches_retained(
            &self.old_runtime,
            self.retained_generation
        ));
        assert!(!matches_retained(
            &self.candidate_runtime,
            self.retained_generation
        ));
        assert!(!matches_retained(
            &self.retained_runtime,
            self.retained_generation.checked_add(1).unwrap()
        ));
    }
    pub(super) async fn live_duplex(
        &mut self,
        f: &Fixture,
        peers: &mut [Peer],
        until: tokio::time::Instant,
    ) {
        self.window(
            f,
            peers,
            Some(0),
            &[1],
            true,
            false,
            until,
            "held-remote-public-and-private",
        )
        .await;
        self.window(
            f,
            peers,
            Some(1),
            &[0],
            false,
            false,
            until,
            "held-old-public-only",
        )
        .await;
        self.window(
            f,
            peers,
            Some(2),
            &[],
            false,
            false,
            until,
            "held-candidate-outgoing-muted",
        )
        .await;
    }
    pub(super) async fn cancelled_original(
        &mut self,
        f: &Fixture,
        peers: &mut [Peer],
        until: tokio::time::Instant,
    ) {
        loop {
            match self.observation.snapshot() {
                Ok(value)
                    if value.cancelled
                        && value.committed != Some(true)
                        && value.audio.as_ref().is_none_or(|audio| {
                            !audio.private_current
                                && audio.source_muted
                                && audio.ordinary_programme_empty
                        }) =>
                {
                    break
                }
                Ok(_) => {}
                Err(error) if error.code == "authority-busy" => {}
                Err(error) => panic!("cancelled private observation refused: {error:?}"),
            }
            assert!(
                tokio::time::Instant::now() < until,
                "original private cancellation did not close"
            );
            tokio::time::sleep_until(
                until.min(tokio::time::Instant::now() + Duration::from_millis(1)),
            )
            .await;
        }
        // Any bounded already-admitted tail is history, not current private IO.
        self.window(
            f,
            peers,
            None,
            &[1],
            false,
            true,
            until,
            "cancelled-private-history",
        )
        .await;
        self.window(
            f,
            peers,
            Some(0),
            &[1],
            false,
            true,
            until,
            "cancelled-original-public-only",
        )
        .await;
    }
    async fn window(
        &mut self,
        f: &Fixture,
        peers: &mut [Peer],
        source: Option<usize>,
        public: &[usize],
        private_positive: bool,
        closed: bool,
        until: tokio::time::Instant,
        name: &str,
    ) {
        enum Event {
            Tick,
            Timer,
            Quiet,
            Packet(
                usize,
                std::result::Result<Vec<u8>, connect_client::ClientError>,
            ),
        }
        self.state(f, until, closed).await;
        let start = tokio::time::Instant::now();
        assert!(start < until);
        let send_until = start + Duration::from_millis(400);
        let minimum_tail = if source.is_some() {
            send_until + Duration::from_millis(300)
        } else {
            start + Duration::from_millis(300)
        };
        assert!(
            minimum_tail < until,
            "media observation cannot consume the original decision margin"
        );
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_marked = start;
        let mut counts = [0usize; 3];
        let mut raw_public = [0usize; 3];
        let mut raw_private = 0usize;
        let mut private = 0usize;
        let mut historical = [0usize; 3];
        let mut attempts = 0usize;
        let mut enqueued = 0usize;
        let mut packets = 0usize;
        let mut quiet = false;
        while tokio::time::Instant::now() < until {
            let now = tokio::time::Instant::now();
            let event = {
                let receiving = peers
                    .iter_mut()
                    .enumerate()
                    .filter(|(_, p)| p.live && p.handle != 0)
                    .map(|(i, p)| {
                        Box::pin(async move { (i, p.client.receive_native_media_datagram().await) })
                    })
                    .collect::<Vec<_>>();
                if now >= minimum_tail
                    && now.duration_since(last_marked) >= Duration::from_millis(200)
                {
                    // Preserve the original probe's reader-FIRST quiet decision:
                    // already-queued marked frames belong to THIS source, never
                    // to the next phase or a fabricated quiet boundary.
                    tokio::select! { biased;
                        ((i,value),_,_)=select_all(receiving) => Event::Packet(i,value),
                        _=tokio::time::sleep_until(until.min(now+Duration::from_millis(1))) => Event::Quiet,
                    }
                } else {
                    tokio::select! {
                        _=tick.tick(), if source.is_some() && now<send_until => Event::Tick,
                        _=tokio::time::sleep_until(until.min(now+Duration::from_millis(1))) => Event::Timer,
                        ((i,value),_,_)=select_all(receiving) => Event::Packet(i,value),
                    }
                }
            };
            match event {
                Event::Quiet => {
                    quiet = true;
                    break;
                }
                Event::Timer => {}
                Event::Tick => {
                    if tokio::time::Instant::now() >= send_until {
                        continue;
                    }
                    let source = source.unwrap();
                    let peer = &mut peers[source];
                    peer.sequence = peer.sequence.wrapping_add(1);
                    let packet = pack_rtp_datagram(&RtpDatagram {
                        flags: 0,
                        stream_local_id: peer.handle,
                        seq: peer.sequence,
                        rtp: RtpMediaPayload {
                            payload: (0..160)
                                .map(|i| if i % 16 < 8 { 0x80 } else { 0x00 })
                                .collect::<Vec<_>>()
                                .into(),
                            payload_type: 0,
                            sequence_number: peer.sequence as u16,
                            timestamp: (peer.clock.elapsed().as_micros() * 8 / 1000) as u32,
                            ssrc: 0x4736_8000 + source as u32,
                        },
                    })
                    .unwrap();
                    // This RAW API checks syntax/physical authentication and
                    // local queue admission only. Preparing media authorization
                    // is enforced by the server, so there is no local typed
                    // Preparing refusal to waive as a successful mute test.
                    attempts += 1;
                    peer.client
                        .send_native_media_datagram(&packet)
                        .expect("raw native enqueue failed; not a media authorization refusal");
                    enqueued += 1;
                }
                Event::Packet(i, Err(error)) => {
                    assert!(
                        closed
                            && i == 2
                            && matches!(&error, connect_client::ClientError::Transport),
                        "unexpected live reader/protocol failure: {error}"
                    );
                    self.state(f, until, true).await;
                    peers[i].live = false; // Actual closed physical reader, not an omitted observer.
                    eprintln!("SERVICE_PRIVATE_OBSERVER reader=2 actual_transport_closed=true phase={name}");
                }
                Event::Packet(i, Ok(bytes)) => {
                    packets += 1;
                    assert!(packets <= 2048, "bounded actual media observation");
                    let packet = unpack_rtp_datagram(&bytes).expect("unknown media datagram");
                    assert_eq!(
                        packet.stream_local_id, peers[i].handle,
                        "unknown receiving handle"
                    );
                    assert_eq!(packet.rtp.payload_type, 0);
                    assert_eq!(packet.rtp.payload.len(), 160);
                    let pcm = packet
                        .rtp
                        .payload
                        .iter()
                        .map(|byte| {
                            let v = !*byte;
                            let value = (((i32::from(v & 15) << 3) + 132)
                                << (i32::from(v & 112) >> 4))
                                - 132;
                            if v & 128 != 0 {
                                -value
                            } else {
                                value
                            }
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(pcm.len(), 160);
                    let marked = packet
                        .rtp
                        .payload
                        .iter()
                        .any(|v| !matches!(*v, 0xff | 0x7f));
                    if i == 2 {
                        assert_eq!(
                            packet.stream_local_id,
                            self.readiness.audio().stream_local_id
                        );
                        assert!(self.private_sequences.len() < 2048);
                        self.private_sequences.push(packet.seq);
                    }
                    if !marked {
                        continue;
                    }
                    let audible = pcm.iter().any(|value| value.abs() > 1000);
                    last_marked = tokio::time::Instant::now();
                    if source.is_none() {
                        // Only the previously marked retained speaker ran during
                        // readiness; these old packets never earn fresh credit.
                        assert!(
                            i == 1 || i == 2,
                            "unknown readiness/cancellation backlog receiver"
                        );
                        historical[i] += 1;
                        continue;
                    }
                    assert!(
                        enqueued > 0,
                        "fresh phase received a marked pre-publication backlog"
                    );
                    if i == 2 {
                        assert!(!closed && source==Some(0) && private_positive,
                            "same-participant/outgoing-muted/cancelled source reached private receiver");
                        raw_private += 1;
                        if audible {
                            private += 1;
                        }
                    } else {
                        assert!(
                            public.contains(&i),
                            "fresh source {source:?} reached forbidden PUBLIC receiver {i}"
                        );
                        raw_public[i] += 1;
                        if audible {
                            counts[i] += 1;
                        }
                    }
                }
            }
        }
        assert!(
            quiet,
            "same original source did not reach measured quiet before cutoff"
        );
        self.state(f, until, closed).await;
        if source.is_some() {
            assert!(
                attempts > 10 && enqueued > 10,
                "too few real local enqueue attempts; wire/admission is not inferred"
            );
            for i in public {
                assert!(
                    counts[*i] > 10,
                    "too few FRESH public decoded frames: {counts:?}"
                );
            }
            if private_positive {
                assert!(
                    private > 10,
                    "too few FRESH authorized private decoded frames"
                );
            }
        }
        eprintln!("SERVICE_PRIVATE_OBSERVER phase={name} source={source:?} public={public:?} fresh_public={counts:?} fresh_private={private} raw_public={raw_public:?} raw_private={raw_private} historical={historical:?} attempts={attempts} local_enqueues={enqueued} packets={packets} elapsed_ms={} actual_private_only={} actual_closed={closed} publication_cutoff_extended=false",start.elapsed().as_millis(),!closed);
    }
    pub(super) fn validate_completed(&self, done: &OperationRecord) {
        let boundary = self
            .readiness
            .activation_from_completed(&serde_json::to_value(done).unwrap())
            .unwrap();
        let connect_protocol::replacement_activation::NativeActivationAudience::Public { streams } =
            boundary.audience
        else {
            panic!("same actual move has no public sequence boundary")
        };
        let cut = streams
            .iter()
            .find(|value| value.stream_id == self.readiness.audio().stream_id)
            .unwrap();
        assert!(!self.private_sequences.is_empty());
        for sequence in &self.private_sequences {
            assert_eq!(
                cut.membership(*sequence).unwrap(),
                connect_protocol::replacement_activation::SequenceMembership::Private,
                "pre-cut/history packet was ordinary public membership"
            );
        }
        eprintln!(
            "SERVICE_PRIVATE_OBSERVER original_private_packets={} actual_completed_boundary=true",
            self.private_sequences.len()
        );
    }
    pub(super) fn validate_public(&self, done: &OperationRecord, packets: &[NativeProbeSequence]) {
        let boundary = self
            .readiness
            .activation_from_completed(&serde_json::to_value(done).unwrap())
            .unwrap();
        let connect_protocol::replacement_activation::NativeActivationAudience::Public { streams } =
            boundary.audience
        else {
            panic!()
        };
        let cut = streams
            .iter()
            .find(|value| value.stream_id == self.readiness.audio().stream_id)
            .unwrap();
        let mut public = 0;
        for packet in packets
            .iter()
            .filter(|packet| packet.receiver == 2 && packet.non_silent)
        {
            assert_eq!(packet.handle, cut.stream_local_id);
            assert_eq!(packet.decoded_samples, 160);
            assert_eq!(
                cut.membership(packet.sequence).unwrap(),
                connect_protocol::replacement_activation::SequenceMembership::Public
            );
            public += 1;
        }
        assert!(
            public > 10,
            "ordinary playback requires fresh actual public membership"
        );
    }
}
