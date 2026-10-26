//! Controlled signed post-auth streams isolate collector failure behavior.
//! Production runtime tests separately prove enrollment and authentication.
#![allow(clippy::unwrap_used)]
use super::*;
use tokio::sync::Notify;
mod auth_refresh;
pub(super) mod native_media;
mod owned_signaling;
mod sdk_consultations;
mod sdk_decisions;
mod sdk_multiplex;
mod sdk_native;
mod sdk_worker;
mod step_up;
use auth_refresh::RefreshReply;

#[test]
fn canonical_cursor_facts_bind_outer_resources_to_the_authoritative_journal_event() {
    let data = json!({"event_id":"event","cursor":"cursor","conversation_id":"room","session_id":"session","connection_id":null,
        "participant_id":null,"operation_id":"original-operation","kind":"changed","revision":1,
        "source":{"kind":"connect-journal","correlation_id":"room"},"observed_at":1,"audience":[],
        "data":{"kind":"changed","data":{"resource_kind":"session","resource_id":"session"}}});
    let value = json!({"v":2,"type":"conversation.event","id":"fact","ts":chrono::Utc::now(),"cid":"room","sid":"session",
        "operation_id":"original-operation","in_reply_to":"subscription","payload":{"event_id":"event","cursor":"cursor","kind":"changed","revision":1,"data":data}});
    let scope = CommandScope {
        conversation_id: Some("room".into()),
        ..Default::default()
    };
    assert!(
        check_reply_scope(&Envelope::from_value(value.clone()).unwrap(), &scope, false).is_ok()
    );
    for field in [
        "conversation_id",
        "session_id",
        "operation_id",
        "event_id",
        "cursor",
    ] {
        let mut bad = value.clone();
        bad["payload"]["data"][field] = json!("foreign");
        assert!(
            check_reply_scope(&Envelope::from_value(bad).unwrap(), &scope, false).is_err(),
            "{field}"
        );
    }
    let started = json!({"v":2,"type":"session.started","id":"started","ts":chrono::Utc::now(),"cid":"room","sid":"session","in_reply_to":"subscription","payload":{"started_at":chrono::Utc::now(),"participants_present":["participant"],"active_connections":[{"connid":"connection","participant_id":"participant","transport":"quic"}],"negotiated_capabilities":{}}});
    assert!(check_reply_scope(
        &Envelope::from_value(started.clone()).unwrap(),
        &scope,
        false
    )
    .is_ok());
    let mut foreign = started.clone();
    foreign["cid"] = json!("foreign");
    assert!(check_reply_scope(&Envelope::from_value(foreign).unwrap(), &scope, false).is_err());
    for field in ["connid", "operation_id"] {
        let mut invalid = started.clone();
        invalid[field] = json!("unrequested");
        if let Ok(frame) = Envelope::from_value(invalid) {
            assert!(check_reply_scope(&frame, &scope, false).is_err());
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Fault {
    SdkWorker(sdk_worker::Traffic),
    SdkMultiplex(sdk_multiplex::Case),
    SdkNative(sdk_native::Case),
    SdkDecision(sdk_decisions::Case),
    SdkConsultation(sdk_consultations::Case),
    ForeignScope,
    ReplyCount,
    ReplyBytes,
    InboxCount,
    InboxBytes,
    MissingAck,
    StreamingDeadline,
    UnknownComplete,
    Refresh(RefreshReply),
    Owned(owned_signaling::Traffic),
    StepUp(step_up::Reply),
    NativeDatagrams(native_media::Traffic),
}
pub(super) async fn controlled(
    fault: Fault,
) -> (CanonicalClient, tokio::task::JoinHandle<()>, Arc<Notify>) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (cert, key) = rvoip_uctp::substrate::self_signed_for_dev(&["localhost".into()]).unwrap();
    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.clone()], key)
        .unwrap();
    tls.alpn_protocols = vec![RAW_QUIC_ALPN.to_vec()];
    let server = quinn::Endpoint::server(
        quinn::ServerConfig::with_crypto(Arc::new(
            quinn::crypto::rustls::QuicServerConfig::try_from(tls).unwrap(),
        )),
        "127.0.0.1:0".parse().unwrap(),
    )
    .unwrap();
    let address = server.local_addr().unwrap();
    let signing = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
    let signer = Sig9421Signer::ed25519("server", &signing.serialize_der()).unwrap();
    let verification = SignatureVerificationContext::new("connect-server", "server").unwrap();
    let mut resolver = StaticKeyResolver::new();
    resolver.insert_contextual(
        &verification,
        "server",
        ResolvedVerificationKey {
            public_key: signing.public_key_raw().to_vec(),
            algorithm: "EdDSA".into(),
            not_before: None,
            expires_at: None,
            revoked: false,
        },
    );
    let ready = Arc::new(Notify::new());
    let signal = ready.clone();
    let task = tokio::spawn(async move {
        let connection = server.accept().await.unwrap().await.unwrap();
        let Ok((mut writer, mut reader)) = connection.accept_bi().await else {
            return;
        };
        let mut prefix = [0; 4];
        if reader.read_exact(&mut prefix).await.is_err() {
            return;
        }
        let mut bytes = vec![0; u32::from_be_bytes(prefix) as usize];
        reader.read_exact(&mut bytes).await.unwrap();
        let request = Envelope::parse(&bytes).unwrap();
        if let Fault::SdkConsultation(case) = fault {
            sdk_consultations::emit(case, request, &mut writer, &mut reader, &signer, &signal)
                .await;
            connection.closed().await;
            server.close(0u32.into(), b"SDK consultation fixture complete");
            return;
        }
        if let Fault::SdkDecision(case) = fault {
            sdk_decisions::emit(case, request, &mut writer, &mut reader, &signer).await;
            connection.closed().await;
            server.close(0u32.into(), b"SDK decision fixture complete");
            return;
        }
        if let Fault::SdkNative(case) = fault {
            sdk_native::emit(
                &connection,
                case,
                request,
                &mut writer,
                &mut reader,
                &signer,
                &signal,
            )
            .await;
            connection.closed().await;
            server.close(0u32.into(), b"SDK native fixture complete");
            return;
        }
        if let Fault::SdkMultiplex(case) = fault {
            sdk_multiplex::emit(case, request, &mut writer, &mut reader, &signer, &signal).await;
            connection.closed().await;
            server.close(0u32.into(), b"SDK multiplex fixture complete");
            return;
        }
        if let Fault::SdkWorker(traffic) = fault {
            sdk_worker::emit(traffic, request, &mut writer, &mut reader, &signer, &signal).await;
            connection.closed().await;
            server.close(0u32.into(), b"SDK worker fixture complete");
            return;
        }
        if let Fault::Owned(traffic) = fault {
            owned_signaling::emit(traffic, &mut writer, &signer, &signal, request.id()).await;
            signal.notify_one();
            connection.closed().await;
            server.close(0u32.into(), b"owned reader test complete");
            return;
        }
        if let Fault::StepUp(reply) = fault {
            step_up::emit(reply, request, &mut writer, &mut reader, &signer, &signal).await;
            signal.notify_one();
            connection.closed().await;
            server.close(0u32.into(), b"step-up test complete");
            return;
        }
        let count = match fault {
            Fault::ReplyCount | Fault::InboxCount => 129,
            Fault::ReplyBytes | Fault::InboxBytes => 3,
            Fault::StreamingDeadline | Fault::Refresh(RefreshReply::Streaming) => 40,
            Fault::Refresh(RefreshReply::Missing) => 0,
            _ => 1,
        };
        for _ in 0..count {
            let unrelated = matches!(fault, Fault::InboxCount | Fault::InboxBytes);
            let mut value = envelope(
                "operation.updated",
                json!({"operation_id":"op_probe","state":"unknown","revision":1}),
            );
            value["cid"] = json!(if matches!(fault, Fault::ForeignScope) {
                "conv_foreign"
            } else {
                "conv_one"
            });
            value["in_reply_to"] = json!(if unrelated {
                "other_request"
            } else {
                request.id()
            });
            if matches!(fault, Fault::ReplyBytes | Fault::InboxBytes) {
                value["payload"]["state"] = json!("completed");
                value["payload"]["result"] = json!({"opaque":"x".repeat(750_000)});
            }
            if let Fault::Refresh(reply) = fault {
                assert_eq!(request.kind(), "auth.refresh");
                value = reply.value(request.id());
            }
            let signed = signer.sign(&value).unwrap();
            // Do not rely on malformed payloads to trigger the collector guard.
            if !matches!(fault, Fault::Refresh(_)) {
                Envelope::from_value(signed.clone()).unwrap();
            }
            let raw = serde_json::to_vec(&signed).unwrap();
            if writer
                .write_all(&(raw.len() as u32).to_be_bytes())
                .await
                .is_err()
                || writer.write_all(&raw).await.is_err()
            {
                break;
            }
            if matches!(
                fault,
                Fault::StreamingDeadline | Fault::Refresh(RefreshReply::Streaming)
            ) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        if matches!(fault, Fault::UnknownComplete | Fault::NativeDatagrams(_)) {
            let mut ack = envelope("ack", json!({"details":{"reply_complete":true}}));
            ack["cid"] = json!("conv_one");
            ack["in_reply_to"] = json!(request.id());
            let raw = serde_json::to_vec(&signer.sign(&ack).unwrap()).unwrap();
            writer
                .write_all(&(raw.len() as u32).to_be_bytes())
                .await
                .unwrap();
            writer.write_all(&raw).await.unwrap();
        }
        if let Fault::NativeDatagrams(traffic) = fault {
            native_media::emit(traffic, &connection, &signal).await;
        } else {
            signal.notify_one();
        }
        connection.closed().await;
        server.close(0u32.into(), b"test complete");
    });
    let mut tls = rvoip_uctp::substrate::dev_client_config_trusting(&cert).unwrap();
    tls.alpn_protocols = vec![RAW_QUIC_ALPN.to_vec()];
    let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
    endpoint.set_default_client_config(quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls).unwrap(),
    )));
    let connection = endpoint
        .connect(address, "localhost")
        .unwrap()
        .await
        .unwrap();
    let (writer, reader) = connection.open_bi().await.unwrap();
    let signing = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
    let client = CanonicalClient {
        network_profile: NativeNetworkProfile::Local,
        server_address: address,
        endpoint,
        connection,
        writer,
        reader: Some(reader),
        signer: Sig9421Signer::ed25519("client", &signing.serialize_der()).unwrap(),
        verifier: Arc::new(Sig9421Verifier::new(Arc::new(resolver))),
        verification,
        deadline: Duration::from_millis(if matches!(fault, Fault::StreamingDeadline) {
            120
        } else {
            2_000
        }),
        closed: Arc::new(AtomicBool::new(false)),
        authenticated_until: chrono::Utc::now() + chrono::Duration::minutes(1),
        session_token: "controlled-post-auth".into(),
        record_download: records::RecordDownload::new(
            address.ip(),
            "localhost",
            cert.as_ref(),
            "controlled-credential".into(),
            NativeNetworkProfile::Local,
        )
        .unwrap(),
        identity_id: "id_one".into(),
        assurance: "identified".into(),
        step_up: None,
        events: VecDeque::new(),
        event_bytes: 0,
        server_key_expires_at: chrono::Utc::now() + chrono::Duration::minutes(1),
        owned_signaling: None,
    };
    (client, task, ready)
}
fn query() -> Payload {
    serde_json::from_value(json!({"type":"operation.get","payload":{"operation_id":"op_probe"}}))
        .unwrap()
}
fn scope() -> CommandScope {
    CommandScope {
        conversation_id: Some("conv_one".into()),
        ..Default::default()
    }
}
async fn rejected(fault: Fault) {
    let (mut client, server, _) = controlled(fault).await;
    assert!(matches!(
        client.request(query(), scope()).await,
        Err(ClientError::Protocol)
    ));
    assert!(matches!(
        client.send(query(), scope()).await,
        Err(ClientError::Transport)
    ));
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn signed_foreign_scope_retires_collector_peer() {
    rejected(Fault::ForeignScope).await;
}
#[tokio::test]
async fn signed_reply_count_and_byte_bounds_retire_collector_peer() {
    rejected(Fault::ReplyCount).await;
    rejected(Fault::ReplyBytes).await;
}
#[tokio::test]
async fn signed_live_inbox_count_and_byte_bounds_retire_collector_peer() {
    rejected(Fault::InboxCount).await;
    rejected(Fault::InboxBytes).await;
}
#[tokio::test]
async fn signed_missing_completion_times_out_without_reporting_logical_success() {
    let (mut client, server, _) = controlled(Fault::MissingAck).await;
    client.deadline = Duration::from_millis(100);
    assert!(matches!(
        client.request(query(), scope()).await,
        Err(ClientError::Timeout)
    ));
    assert!(client.closed.load(Ordering::Acquire));
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn signed_ready_stream_cannot_extend_absolute_batch_deadline() {
    let (mut client, server, _) = controlled(Fault::StreamingDeadline).await;
    let start = tokio::time::Instant::now();
    assert!(matches!(
        client.request(query(), scope()).await,
        Err(ClientError::Timeout)
    ));
    assert!(start.elapsed() < Duration::from_secs(1));
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn cancellation_after_signed_partial_batch_retires_physical_peer() {
    let (mut client, server, ready) = controlled(Fault::MissingAck).await;
    let token = tokio_util::sync::CancellationToken::new();
    let cancel = token.clone();
    let (result, ()) = tokio::join!(client.request_with_cancel(query(), scope(), token), async {
        ready.notified().await;
        tokio::task::yield_now().await;
        cancel.cancel();
    });
    assert!(matches!(result, Err(ClientError::Cancelled)));
    assert!(client.closed.load(Ordering::Acquire));
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn original_credential_and_server_pin_expiry_bound_the_entire_batch() {
    for credential in [true, false] {
        let (mut client, server, _) = controlled(Fault::MissingAck).await;
        let expiry = chrono::Utc::now() + chrono::Duration::milliseconds(100);
        if credential {
            client.authenticated_until = expiry;
        } else {
            client.server_key_expires_at = expiry;
        }
        let start = tokio::time::Instant::now();
        assert!(matches!(
            client.request(query(), scope()).await,
            Err(ClientError::Timeout)
        ));
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(client.closed.load(Ordering::Acquire));
        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
async fn typed_operation_get_keeps_signed_unknown_state_after_batch_ack() {
    let (mut client, server, _) = controlled(Fault::UnknownComplete).await;
    let record = client.operation_get("op_probe", scope()).await.unwrap();
    assert_eq!(
        serde_json::to_value(record.state).unwrap(),
        json!("unknown")
    );
    assert!(record.result.is_none());
    client.close();
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn atomic_native_datagram_wait_does_not_retire_signed_control_peer() {
    let (mut client, server, _) = controlled(Fault::UnknownComplete).await;
    client.deadline = Duration::from_millis(50);
    assert!(matches!(
        client.receive_native_media_datagram().await,
        Err(ClientError::Timeout)
    ));
    assert!(!client.closed.load(Ordering::Acquire));
    assert!(client.connection.close_reason().is_none());
    // Cancelling an atomic datagram read also leaves the signaling frame intact.
    client.deadline = Duration::from_secs(2);
    assert!(tokio::time::timeout(
        Duration::from_millis(20),
        client.receive_native_media_datagram()
    )
    .await
    .is_err());
    let record = client.operation_get("op_probe", scope()).await.unwrap();
    assert_eq!(
        serde_json::to_value(record.state).unwrap(),
        json!("unknown")
    );
    assert!(record.result.is_none());
    client.close();
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
}

#[test]
fn original_command_values_preserve_extensions_without_bypassing_validation_or_auth() {
    let payload = json!({"subscriptions":[{"strm_id":"second","future":{"weight":2}},{"strm_id":"first"}],"future_extension":{"version":7}});
    let scope = CommandScope {
        conversation_id: Some("conversation".into()),
        session_id: Some("session".into()),
        connection_id: Some("receiver".into()),
        operation_id: Some("select".into()),
        expected_revision: Some(1),
    };
    let value = command_value("stream.subscribe", payload.clone(), scope).unwrap();
    let checked = Envelope::from_value(value).unwrap();
    assert_eq!(checked.original()["payload"], payload);
    assert_eq!(checked.original()["expected_revision"], 1);
    assert!(command_value(
        "stream.subscribe",
        json!({"subscriptions":"bad-required-type"}),
        CommandScope::default()
    )
    .is_err());
    assert!(command_value(
        "auth.keepalive",
        json!({"session_token":"untrusted"}),
        CommandScope::default()
    )
    .is_err());
}
