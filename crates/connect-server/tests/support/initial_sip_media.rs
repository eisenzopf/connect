//! Actual loopback SIP and guarded initial admission, without replacement state.
use super::*;
use rvoip_core::staged_streams::*;
use rvoip_core::stream_lifecycle::*;
use rvoip_sip::{Config as SipConfig, SipAdapter, SipOriginateContext, UnifiedCoordinator};
use rvoip_sip_core::{
    parser::parse_message,
    prelude::{Message as SipMessage, Method, StatusCode},
};
use std::collections::BTreeSet;

#[derive(Default)]
struct LiveState {
    prepared: Option<Arc<OutboundParticipantPreparation>>,
    observation: Option<Arc<PreparedMediaObservation>>,
}
struct LivePolicy {
    control: Arc<Policy>,
    state: Arc<Mutex<LiveState>>,
}
fn capability() -> EndpointMediaCapabilities {
    let audio = vec![MediaCapability {
        medium: Medium::Audio,
        formats: vec![MediaFormat::Encoded {
            name: "pcmu".into(),
            clock_rate_hz: 8000,
            channels: 1,
            fmtp: None,
        }],
    }];
    EndpointMediaCapabilities {
        send: audio.clone(),
        receive: audio,
        max_streams: 1,
        renegotiation: false,
    }
}
struct MediaLease {
    owner: MediaOwner,
    state: Arc<Mutex<LiveState>>,
    denied: Arc<AtomicBool>,
}
impl MediaAuthorityLease for MediaLease {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
    fn control_owner(&self) -> staged_streams::ControlResult<Box<MediaOwner>> {
        if self.denied.load(Ordering::SeqCst) {
            return Err(StreamControlError::Authority);
        }
        Ok(Box::new(self.owner.clone()))
    }
    fn control_prepared(
        &self,
        observation: &Arc<PreparedMediaObservation>,
    ) -> staged_streams::ControlResult<Box<MediaOwner>> {
        let owner = observation.activate_for_commit(*self.control_owner()?)?;
        self.state.lock().unwrap().observation = Some(observation.clone());
        Ok(Box::new(owner))
    }
}
impl LivePolicy {
    fn lease(
        &self,
        s: &MediaScope<'_>,
    ) -> staged_streams::ControlResult<Box<dyn MediaAuthorityLease>> {
        if self.control.denied.load(Ordering::SeqCst) {
            return Err(StreamControlError::Authority);
        }
        let state = self.state.lock().unwrap();
        let p = state
            .prepared
            .as_ref()
            .ok_or(StreamControlError::NotReady)?;
        if !p.is_current()
            || s.connection != p.binding().connection()
            || s.session != p.appearance().session()
            || s.conversation != p.member().conversation()
        {
            return Err(StreamControlError::Authority);
        }
        p.member()
            .revalidate_principal(s.principal, Utc::now())
            .map_err(|_| StreamControlError::Authority)?;
        let mut owner = MediaOwner {
            actor: p.member().clone(),
            member: p.member().clone(),
            presence: p.appearance().clone(),
            endpoint: p.endpoint().clone(),
            connection: p.binding().clone(),
            may_control: true,
            send_media: [Medium::Audio].into_iter().collect(),
            receive_media: [Medium::Audio].into_iter().collect(),
            local: capability(),
            remote: capability(),
            may_route_source: false,
        };
        if let Some(observation) = state.observation.as_ref().filter(|p| p.is_committed()) {
            owner = observation.activate_for_commit(owner)?;
        }
        Ok(Box::new(MediaLease {
            owner,
            state: self.state.clone(),
            denied: self.control.denied.clone(),
        }))
    }
}
impl ParticipantPolicy for LivePolicy {
    fn current(
        &self,
        s: &ParticipantScope<'_>,
    ) -> rvoip_core::orchestrator::participant_control::ControlResult<
        Box<dyn ParticipantAuthorityLease + '_>,
    > {
        ParticipantPolicy::current(self.control.as_ref(), s)
    }
    fn current_control(
        &self,
        s: &ParticipantScope<'_>,
        held: &(dyn Any + Send + Sync),
    ) -> rvoip_core::orchestrator::participant_control::ControlResult<ParticipantControl> {
        ParticipantPolicy::current_control(self.control.as_ref(), s, held)
    }
}
impl MediaPolicy for LivePolicy {
    fn current(
        &self,
        s: &MediaScope<'_>,
    ) -> staged_streams::ControlResult<Box<dyn MediaAuthorityLease>> {
        self.lease(s)
    }
    fn current_control(
        &self,
        s: &MediaScope<'_>,
        intent: &MediaControlIntent<'_>,
        held: &(dyn Any + Send + Sync),
    ) -> staged_streams::ControlResult<MediaControl> {
        let i = held
            .downcast_ref::<Invocation>()
            .filter(|i| Arc::ptr_eq(&i.domain, &self.control.domain))
            .ok_or(StreamControlError::Authority)?;
        if s.connection != &i.connection || i.denied.load(Ordering::SeqCst) {
            return Err(StreamControlError::Authority);
        }
        match intent {
            MediaControlIntent::Initialize => {}
            MediaControlIntent::Change {
                expected: 0,
                desired,
                replace,
            } if desired.len() == 1 && desired[0].medium == Medium::Audio && replace.is_empty() => {
            }
            _ => return Err(StreamControlError::Authority),
        }
        MediaControl::new(
            self.lease(s)?,
            CancellationToken::new(),
            tokio::time::Instant::now() + Duration::from_secs(5),
        )
    }
}
#[tokio::test]
async fn actual_initial_sip_admits_guarded_media_on_empty_checked_core_without_replacement_or_native_peer(
) {
    initial_sip_roundtrip(false, false).await;
}
#[tokio::test]
async fn initial_sip_ringing_is_not_activation_and_cancelled_or_expired_capture_preserves_the_actual_answer(
) {
    initial_sip_roundtrip(true, false).await;
}
#[tokio::test]
async fn initial_sip_remote_decline_cannot_mint_an_answer_or_admit_media() {
    initial_sip_roundtrip(true, true).await;
}
async fn initial_sip_roundtrip(delayed_answer: bool, reject_answer: bool) {
    let release_answer = Arc::new(tokio::sync::Notify::new());
    let peer_release = release_answer.clone();
    let progress_seen = Arc::new(tokio::sync::Notify::new());
    let peer_progress = progress_seen.clone();
    let wire = Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let address = wire.local_addr().unwrap();
    let media = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let media_port = media.local_addr().unwrap().port();
    let calls = Arc::new(Mutex::new(HashSet::new()));
    let seen = calls.clone();
    let peer = wire.clone();
    let byes = Arc::new(Mutex::new(HashSet::new()));
    let ended = byes.clone();
    let task = tokio::spawn(async move {
        let mut data = [0; 16384];
        loop {
            let (n, from) = peer.recv_from(&mut data).await.unwrap();
            let Ok(SipMessage::Request(request)) = parse_message(&data[..n]) else {
                continue;
            };
            if request.method() == Method::Ack {
                continue;
            }
            let mut response =
                rvoip_sip_dialog::transaction::utils::response_builders::create_response(
                    &request,
                    if reject_answer && request.method() == Method::Invite {
                        StatusCode::BusyHere
                    } else {
                        StatusCode::Ok
                    },
                );
            if request.method() == Method::Invite {
                seen.lock()
                    .unwrap()
                    .insert(request.call_id().unwrap().value());
                if let Some(rvoip_sip_core::TypedHeader::To(to)) = response
                    .headers
                    .iter_mut()
                    .find(|h| matches!(h, rvoip_sip_core::TypedHeader::To(_)))
                {
                    to.set_tag("initial-sip-peer");
                }
                response.headers.push(rvoip_sip_core::TypedHeader::Contact(
                    format!("<sip:phone@{address}>").parse().unwrap(),
                ));
                response
                    .headers
                    .push(rvoip_sip_core::TypedHeader::ContentType(
                        rvoip_sip_core::types::ContentType::sdp(),
                    ));
                response.body=format!("v=0\r\no=initial-sip 1 1 IN IP4 127.0.0.1\r\ns=initial-sip\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio {media_port} RTP/AVP 0 101\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:101 telephone-event/8000\r\na=fmtp:101 0-15\r\na=sendrecv\r\n").into();
                response
                    .headers
                    .retain(|h| !matches!(h, rvoip_sip_core::TypedHeader::ContentLength(_)));
                response
                    .headers
                    .push(rvoip_sip_core::TypedHeader::ContentLength(
                        rvoip_sip_core::types::ContentLength::new(response.body.len() as u32),
                    ));
            } else if request.method() == Method::Bye {
                ended
                    .lock()
                    .unwrap()
                    .insert(request.call_id().unwrap().value());
            }
            if delayed_answer && request.method() == Method::Invite {
                let progress =
                    rvoip_sip_dialog::transaction::utils::response_builders::create_response(
                        &request,
                        StatusCode::Ringing,
                    );
                peer.send_to(&SipMessage::Response(progress).to_bytes(), from)
                    .await
                    .unwrap();
                peer_progress.notify_one();
                peer_release.notified().await;
            }
            peer.send_to(&SipMessage::Response(response).to_bytes(), from)
                .await
                .unwrap();
        }
    });
    let coordinator = UnifiedCoordinator::new(SipConfig::local("initial-sip-fixture", 0))
        .await
        .unwrap();
    let adapter = SipAdapter::new(coordinator.clone()).await.unwrap();
    let core = Orchestrator::new(Default::default());
    core.register(adapter.clone()).unwrap();
    let principal = AuthenticatedPrincipal {
        subject: "phone".into(),
        tenant: Some("tenant".into()),
        issuer: Some("fixture".into()),
        scopes: vec![],
        expires_at: Some(Utc::now() + chrono::Duration::minutes(5)),
        method: AuthenticationMethod::Bearer,
        assurance: IdentityAssurance::Identified {
            credential_kind: CredentialKind::Bearer,
        },
    };
    let identity = IdentityBinding::from_authenticated(
        &principal,
        IdentityId::from_string("phone"),
        Utc::now(),
    )
    .unwrap();
    let cid = ConversationId::from_string("actual-cid");
    let sid = SessionId::from_string("actual-sid");
    let member = Membership::admit(
        identity.clone(),
        cid.clone(),
        ParticipantId::from_string("actual-phone"),
        ParticipantKind::Human,
        ParticipantRole::Customer,
    )
    .unwrap();
    let mut endpoint = EndpointBinding::provision(
        identity.clone(),
        EndpointId::from_string("actual-ep"),
        DeviceId::from_string("actual-dev"),
        Transport::Sip,
        format!("sip:phone@{address}"),
    )
    .unwrap();
    endpoint
        .verify(&identity, "loopback-proof".into(), Utc::now())
        .unwrap();
    let control = Arc::new(Policy {
        domain: Arc::new(()),
        member,
        endpoint,
        denied: Arc::new(AtomicBool::new(false)),
    });
    let policy = Arc::new(LivePolicy {
        control: control.clone(),
        state: Arc::new(Mutex::new(LiveState::default())),
    });
    core.project_external_session(
        cid,
        sid.clone(),
        TenantId::from_string("tenant"),
        SessionMedium::Mixed,
        [],
    )
    .unwrap();
    core.enable_staged_participant_control(
        &sid,
        policy.clone(),
        policy.clone(),
        ParticipantLimits::default(),
    )
    .unwrap();
    let prepared = core
        .prepare_outbound_connection(
            OriginateRequest::new(
                sid.clone(),
                control.member.participant().clone(),
                control.endpoint.address(),
                Direction::Outbound,
                CapabilityDescriptor::default(),
            )
            .with_transport(Transport::Sip)
            .with_context(SipOriginateContext::default()),
        )
        .await
        .unwrap();
    let connection = prepared.connection_id().clone();
    let invocation = Invocation {
        domain: control.domain.clone(),
        operation: "initial-call".into(),
        connection: connection.clone(),
        member: control.member.clone(),
        endpoint: control.endpoint.clone(),
        denied: control.denied.clone(),
    };
    let mutation = ParticipantMutation {
        operation_id: "initial-call".into(),
        correlation_id: "actual-wire".into(),
        expected_revision: 1,
        participant: control.member.participant().clone(),
        change: ParticipantChange::Admit {
            connection: connection.clone(),
            role_override: None,
        },
    };
    let ordinary_core = core.clone();
    let media_adapter = adapter.clone();
    let media_policy = policy.clone();
    let media_invocation = invocation.clone();
    let capture = Arc::new(Mutex::new(None));
    let captured = capture.clone();
    let captured_answer = Arc::new(Mutex::new(None));
    let saved_answer = captured_answer.clone();
    let admission = prepared.commit_staged_participant_with_held(
        principal,
        mutation,
        tokio::time::Instant::now() + Duration::from_secs(5),
        CancellationToken::new(),
        Box::new(invocation),
        move |p| async move {
            assert!(p.binding().replaces().is_none());
            assert!(
                matches!(
                    ordinary_core.hold(p.binding().connection().clone()).await,
                    Err(RvoipError::AdmissionRejected(
                        "connection is not operational"
                    ))
                ),
                "provisional ticket must not expose ordinary operational control"
            );
            media_policy.state.lock().unwrap().prepared = Some(p.clone());
            let context = media_adapter.initial_participant_context(p.clone())?;
            *captured.lock().unwrap() = Some(context.clone());
            if delayed_answer {
                assert!(matches!(
                    context.prepare_media_controller(media_policy.clone(), 4, &media_invocation),
                    Err(StreamControlError::NotReady)
                ));
                let cancelled = CancellationToken::new();
                cancelled.cancel();
                assert!(context
                    .wait_answered(
                        tokio::time::Instant::now() + Duration::from_secs(1),
                        cancelled,
                    )
                    .await
                    .is_err());
                assert!(
                    context
                        .wait_answered(
                            tokio::time::Instant::now() - Duration::from_millis(1),
                            CancellationToken::new(),
                        )
                        .await
                        .is_err(),
                    "an expired capture deadline must not consume the actual answer"
                );
            }
            let answer = context
                .wait_answered(
                    tokio::time::Instant::now() + Duration::from_secs(2),
                    CancellationToken::new(),
                )
                .await?;
            assert_eq!(answer.connection(), p.binding().connection());
            assert_eq!(answer.external_reference().kind(), "sip.call-id");
            assert!(answer.is_current_for(&context));
            assert!(
                context
                    .wait_answered(
                        tokio::time::Instant::now() + Duration::from_secs(1),
                        CancellationToken::new(),
                    )
                    .await
                    .is_err(),
                "one actual answer must yield at most one captured fact"
            );
            *saved_answer.lock().unwrap() = Some(answer);
            let until = tokio::time::Instant::now() + Duration::from_secs(2);
            let codec = loop {
                match context.negotiated_audio_codec() {
                    Ok(codec) => break codec,
                    Err(_) if tokio::time::Instant::now() < until => {
                        tokio::time::sleep(Duration::from_millis(5)).await
                    }
                    Err(error) => return Err(error.into()),
                }
            };
            assert_eq!(codec.name.to_ascii_lowercase(), "g.711-mu");
            let controller =
                context.prepare_media_controller(media_policy, 4, &media_invocation)?;
            assert!(controller.is_bound_to_outbound(&p));
            controller
                .change_with_held(
                    0,
                    &[DesiredStream {
                        logical_id: StreamId::from_string("audio"),
                        medium: Medium::Audio,
                        direction: StreamDirection::Both,
                    }],
                    &BTreeSet::new(),
                    tokio::time::Instant::now() + Duration::from_secs(3),
                    CancellationToken::new(),
                    Arc::new(media_invocation.clone()),
                )
                .await?;
            *captured.lock().unwrap() = Some(context);
            Ok(Arc::new(media_invocation) as Arc<dyn Any + Send + Sync>)
        },
    );
    tokio::pin!(admission);
    let admission = if delayed_answer {
        tokio::select! {
            result = &mut admission => panic!("initial activation finished before peer progress: {:?}", result.err()),
            progress = tokio::time::timeout(Duration::from_secs(2), progress_seen.notified()) => progress.unwrap(),
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(30), &mut admission)
                .await
                .is_err(),
            "180 Ringing cannot finish activation or invoke the media callback"
        );
        assert!(capture.lock().unwrap().is_none());
        assert!(captured_answer.lock().unwrap().is_none());
        release_answer.notify_one();
        admission.await
    } else {
        admission.await
    };
    if reject_answer {
        assert!(
            admission.is_err(),
            "a real remote decline cannot admit media"
        );
        assert!(captured_answer.lock().unwrap().is_none());
        core.drain_prepared_outbound_connections().await;
        core.drain_connection_lifecycle_tasks().await;
        assert_eq!(core.participant_capacity(&sid).unwrap(), (0, 0));
        assert!(core.session_of(&connection).is_none());
        assert_eq!(calls.lock().unwrap().len(), 1);
        assert!(
            capture.lock().unwrap().is_none(),
            "decline must not invoke the media callback"
        );
        core.end_session(sid, EndReason::Normal).await.unwrap();
        adapter.shutdown().await.unwrap();
        coordinator.shutdown();
        task.abort();
        assert_eq!(
            byes.lock().unwrap().len(),
            0,
            "decline never established a BYE dialog"
        );
        return;
    }
    let (handle, observation) = admission.unwrap_or_else(|error| match error {
        ParticipantControlError::Core(RvoipError::AdmissionRejected(detail)) => {
            panic!("initial SIP admission refused: {detail}")
        }
        other => panic!("initial SIP admission failed: {other:?}"),
    });
    assert_eq!(observation.kind, ParticipantObservationKind::Joined);
    assert_eq!(core.session_of(&connection), Some(sid.clone()));
    assert_eq!(
        core.connection_principal(&connection).unwrap().subject,
        "phone"
    );
    assert_eq!(core.participant_capacity(&sid).unwrap(), (1, 0));
    let references = handle.outbound_activation().external_references();
    assert_eq!(references.len(), 1);
    assert_eq!(references[0].kind(), "sip.call-id");
    assert!(calls
        .lock()
        .unwrap()
        .contains(references[0].expose_secret()));
    assert_eq!(
        captured_answer
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .external_reference()
            .expose_secret(),
        references[0].expose_secret()
    );
    assert_eq!(calls.lock().unwrap().len(), 1);
    assert!(
        adapter.streams(connection.clone()).await.is_err(),
        "raw access must not bypass guarded initial media"
    );
    assert!(adapter
        .renegotiate_media(connection, CapabilityDescriptor::default())
        .await
        .is_err());
    // A retained operation cannot originate a new physical route under the
    // same logical intent after admission, even if its caller allocates again.
    let duplicate = core
        .prepare_outbound_connection(
            OriginateRequest::new(
                sid.clone(),
                control.member.participant().clone(),
                control.endpoint.address(),
                Direction::Outbound,
                CapabilityDescriptor::default(),
            )
            .with_transport(Transport::Sip)
            .with_context(SipOriginateContext::default()),
        )
        .await
        .unwrap();
    let original = policy
        .state
        .lock()
        .unwrap()
        .prepared
        .as_ref()
        .unwrap()
        .principal()
        .clone();
    let replay = Invocation {
        domain: control.domain.clone(),
        operation: "initial-call".into(),
        connection: duplicate.connection_id().clone(),
        member: control.member.clone(),
        endpoint: control.endpoint.clone(),
        denied: control.denied.clone(),
    };
    let replay_mutation = ParticipantMutation {
        operation_id: "initial-call".into(),
        correlation_id: "replayed-wire".into(),
        expected_revision: core.participant_revision(&sid).unwrap(),
        participant: control.member.participant().clone(),
        change: ParticipantChange::Admit {
            connection: duplicate.connection_id().clone(),
            role_override: None,
        },
    };
    assert!(duplicate
        .commit_staged_participant_with_held(
            original,
            replay_mutation,
            tokio::time::Instant::now() + Duration::from_secs(2),
            CancellationToken::new(),
            Box::new(replay),
            |_| async { panic!("replay must not create another media route") }
        )
        .await
        .is_err());
    assert_eq!(calls.lock().unwrap().len(), 1);
    assert_eq!(core.participant_capacity(&sid).unwrap(), (1, 0));
    core.end_session(sid, EndReason::Normal).await.unwrap();
    core.drain_prepared_outbound_connections().await;
    core.drain_connection_lifecycle_tasks().await;
    adapter.shutdown().await.unwrap();
    coordinator.shutdown();
    task.abort();
    assert_eq!(byes.lock().unwrap().len(), 1);
    assert!(
        !captured_answer
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .is_current_for(capture.lock().unwrap().as_ref().unwrap()),
        "a retained answer cannot authorize a retired route"
    );
    assert!(capture
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .negotiated_audio_codec()
        .is_err());
}
