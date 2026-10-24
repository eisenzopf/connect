//! Real signed QUIC frames exercise the owned reader independently of audio.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum Traffic {
    Idle,
    Fragment,
    Partial,
    Invalid,
    Bye,
    Burst,
    AuthSession,
    Speaker,
}
pub(super) async fn emit(
    traffic: Traffic,
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    ready: &Notify,
    request: &str,
) {
    if matches!(traffic, Traffic::Speaker) {
        for active in [json!("part_source"), Value::Null] {
            let mut frame = envelope(
                "stream.active-speaker",
                json!({"active_participant":active,"strm_id":"mix_one","changed_at":chrono::Utc::now()}),
            );
            frame["cid"] = json!("conv_one");
            frame["sid"] = json!("session_one");
            let raw = serde_json::to_vec(&signer.sign(&frame).unwrap()).unwrap();
            writer
                .write_all(&(raw.len() as u32).to_be_bytes())
                .await
                .unwrap();
            writer.write_all(&raw).await.unwrap();
        }
        let mut ack = envelope("ack", json!({}));
        ack["in_reply_to"] = json!(request);
        ack["cid"] = json!("conv_one");
        let raw = serde_json::to_vec(&signer.sign(&ack).unwrap()).unwrap();
        writer
            .write_all(&(raw.len() as u32).to_be_bytes())
            .await
            .unwrap();
        writer.write_all(&raw).await.unwrap();
        return;
    }
    if matches!(traffic, Traffic::Idle) {
        return;
    }
    let count = if matches!(traffic, Traffic::Burst) {
        400
    } else {
        1
    };
    for version in 0..count {
        let mut value = envelope(
            "capability.advertise",
            json!({"by_participant":"part_source","capabilities":{"future":{"version":version,"exact":[true,17]}},"trigger":"endpoint-change"}),
        );
        value["cid"] = json!("conv_one");
        value["sid"] = json!("session_one");
        value["connid"] = json!("source_one");
        if matches!(traffic, Traffic::Bye) {
            value = envelope("auth.bye", json!({"reason":"revoked"}));
        }
        if matches!(traffic, Traffic::AuthSession) {
            value = RefreshReply::Valid.value("unrequested-auth-reply");
        }
        let mut signed = signer.sign(&value).unwrap();
        if matches!(traffic, Traffic::Invalid) {
            signed["payload"]["capabilities"]["future"]["version"] = json!(99);
        }
        let raw = serde_json::to_vec(&signed).unwrap();
        let prefix = (raw.len() as u32).to_be_bytes();
        if matches!(traffic, Traffic::Fragment | Traffic::Partial) {
            writer.write_all(&prefix[..1]).await.unwrap();
            ready.notify_one();
            tokio::time::sleep(Duration::from_millis(
                if matches!(traffic, Traffic::Partial) {
                    300
                } else {
                    60
                },
            ))
            .await;
            if matches!(traffic, Traffic::Partial) {
                return;
            }
            if writer.write_all(&prefix[1..]).await.is_err() {
                return;
            }
        } else if writer.write_all(&prefix).await.is_err() {
            return;
        }
        if writer.write_all(&raw).await.is_err() {
            return;
        }
    }
    if matches!(traffic, Traffic::Burst) {
        for digit in ["3", "7"] {
            let mut frame = envelope(
                "dtmf.received",
                json!({"digits":digit,"duration_ms":100,"received_at":chrono::Utc::now(),"source":"uctp-native"}),
            );
            frame["cid"] = json!("conv_one");
            frame["sid"] = json!("session_one");
            frame["connid"] = json!("source_one");
            let raw = serde_json::to_vec(&signer.sign(&frame).unwrap()).unwrap();
            if writer
                .write_all(&(raw.len() as u32).to_be_bytes())
                .await
                .is_err()
                || writer.write_all(&raw).await.is_err()
            {
                return;
            }
        }
        let mut ack = envelope("ack", json!({}));
        ack["in_reply_to"] = json!(request);
        let raw = serde_json::to_vec(&signer.sign(&ack).unwrap()).unwrap();
        if writer
            .write_all(&(raw.len() as u32).to_be_bytes())
            .await
            .is_ok()
        {
            let _ = writer.write_all(&raw).await;
        }
    }
}

fn assert_current(client: &CanonicalClient) {
    let mut current = client.pending().unwrap();
    current.complete = true;
}

async fn close(client: CanonicalClient, server: tokio::task::JoinHandle<()>) {
    client.close();
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn owned_reader_idle_exceeds_rpc_timeout_without_retiring_audio_peer() {
    let (mut client, server, ready) = controlled(Fault::Owned(Traffic::Idle)).await;
    client.deadline = Duration::from_millis(40);
    client.start_signaling_reader().unwrap();
    client.start_signaling_reader().unwrap();
    client.keepalive().await.unwrap();
    ready.notified().await;
    tokio::time::sleep(Duration::from_millis(130)).await;
    assert_current(&client);
    close(client, server).await;
}

#[tokio::test]
async fn public_signaling_reader_cannot_restart_a_closed_physical_peer() {
    let (mut client, server, _) = controlled(Fault::Owned(Traffic::Idle)).await;
    client.start_signaling_reader().unwrap();
    client.close();
    assert!(client.start_signaling_reader().is_err());
    assert!(client.keepalive().await.is_err());
    close(client, server).await;
}

#[tokio::test]
async fn owned_reader_completes_a_real_fragmented_signed_frame_while_foreground_inbox_waits_are_cancelled(
) {
    let (mut client, server, ready) = controlled(Fault::Owned(Traffic::Fragment)).await;
    client.deadline = Duration::from_millis(200);
    client.start_signaling_reader().unwrap();
    client.keepalive().await.unwrap();
    ready.notified().await;
    for _ in 0..10 {
        assert!(
            tokio::time::timeout(Duration::from_millis(2), client.receive())
                .await
                .is_err()
        );
        assert_current(&client);
    }
    let frame = client.receive().await.unwrap();
    assert_eq!(frame.kind(), "capability.advertise");
    assert_eq!(
        frame.original()["payload"]["capabilities"]["future"]["exact"],
        json!([true, 17])
    );
    close(client, server).await;
}

#[tokio::test]
async fn owned_reader_partial_signature_or_terminal_auth_stops_the_original_peer_without_foreground_signaling_reads(
) {
    for traffic in [Traffic::Partial, Traffic::Invalid, Traffic::Bye] {
        let (mut client, server, ready) = controlled(Fault::Owned(traffic)).await;
        client.deadline = Duration::from_millis(80);
        client.own_signaling().unwrap();
        client.keepalive().await.unwrap();
        ready.notified().await;
        let until = tokio::time::Instant::now() + Duration::from_secs(1);
        while !client.closed.load(Ordering::Acquire) {
            assert!(tokio::time::Instant::now() < until);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(client.keepalive().await.is_err());
        if matches!(traffic, Traffic::Bye) {
            assert_eq!(client.receive().await.unwrap().kind(), "auth.bye");
        }
        assert!(client.receive().await.is_err());
        assert!(matches!(
            client.receive().await,
            Err(ClientError::Transport)
        ));
        close(client, server).await;
    }
}

#[tokio::test]
async fn owned_reader_drains_signed_passive_bursts_and_preserves_ordered_dtmf_and_ack() {
    let (mut client, server, ready) = controlled(Fault::Owned(Traffic::Burst)).await;
    client.own_signaling().unwrap();
    let id = client.keepalive().await.unwrap();
    ready.notified().await;
    let mut latest = None;
    let mut digits = Vec::new();
    loop {
        let frame = client.receive().await.unwrap();
        match frame.kind() {
            "capability.advertise" => {
                let version = frame.original()["payload"]["capabilities"]["future"]["version"]
                    .as_u64()
                    .unwrap();
                assert!(latest.is_none_or(|old| version > old));
                latest = Some(version);
            }
            "dtmf.received" => digits.push(
                frame.original()["payload"]["digits"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            ),
            "ack" => {
                assert_eq!(frame.original()["in_reply_to"], id);
                break;
            }
            _ => panic!("unexpected frame: {:?}", frame.original()),
        }
    }
    assert_eq!(latest, Some(399));
    assert_eq!(digits, ["3", "7"]);
    assert_current(&client);
    close(client, server).await;
}

#[tokio::test]
async fn owned_reader_credential_or_server_key_expiry_retires_an_idle_peer() {
    for key in [false, true] {
        let (mut client, server, ready) = controlled(Fault::Owned(Traffic::Idle)).await;
        let expires = chrono::Utc::now() + chrono::Duration::milliseconds(120);
        if key {
            client.server_key_expires_at = expires;
        } else {
            client.authenticated_until = expires;
        }
        client.own_signaling().unwrap();
        client.keepalive().await.unwrap();
        ready.notified().await;
        let until = tokio::time::Instant::now() + Duration::from_secs(1);
        while !client.closed.load(Ordering::Acquire) {
            assert!(tokio::time::Instant::now() < until);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(client.keepalive().await.is_err());
        close(client, server).await;
    }
}

#[tokio::test]
async fn owned_reader_validated_refresh_replaces_original_expiry_without_replacing_reader() {
    let (mut client, server, _) = controlled(Fault::Refresh(RefreshReply::Valid)).await;
    let original = chrono::Utc::now() + chrono::Duration::milliseconds(400);
    client.authenticated_until = original;
    client.own_signaling().unwrap();
    let authentication = client
        .refresh_credential("explicit-owned-successor".into())
        .await
        .unwrap();
    assert!(authentication.expires_at > original);
    tokio::time::sleep(Duration::from_millis(450)).await;
    assert_current(&client);
    client.keepalive().await.unwrap();
    close(client, server).await;
}

#[tokio::test]
async fn owned_reader_refresh_preserves_original_server_key_expiry() {
    let (mut client, server, _) = controlled(Fault::Refresh(RefreshReply::Valid)).await;
    let pin = chrono::Utc::now() + chrono::Duration::milliseconds(600);
    client.authenticated_until = chrono::Utc::now() + chrono::Duration::milliseconds(300);
    client.server_key_expires_at = pin;
    client.own_signaling().unwrap();
    assert_eq!(
        client
            .refresh_credential("explicit-owned-successor".into())
            .await
            .unwrap()
            .expires_at,
        pin
    );
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_current(&client);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(client.closed.load(Ordering::Acquire));
    close(client, server).await;
}

#[tokio::test]
async fn owned_reader_refused_or_unsolicited_refresh_cannot_extend_current_authority() {
    for unsolicited in [false, true] {
        let (mut client, server, _) = controlled(if unsolicited {
            Fault::Owned(Traffic::AuthSession)
        } else {
            Fault::Refresh(RefreshReply::Refused)
        })
        .await;
        client.authenticated_until = chrono::Utc::now() + chrono::Duration::milliseconds(400);
        let original = (client.session_token.clone(), client.authenticated_until);
        client.own_signaling().unwrap();
        if unsolicited {
            client.keepalive().await.unwrap();
            assert_eq!(client.receive().await.unwrap().kind(), "auth.session");
        } else {
            assert!(client
                .refresh_credential("explicit-refused-successor".into())
                .await
                .is_err());
        }
        assert_eq!(
            (client.session_token.clone(), client.authenticated_until),
            original
        );
        assert_current(&client);
        tokio::time::sleep(Duration::from_millis(450)).await;
        assert!(client.closed.load(Ordering::Acquire));
        close(client, server).await;
    }
}

#[tokio::test]
async fn owned_reader_invalid_refresh_retires_original_peer_without_installing_candidate() {
    let (mut client, server, _) = controlled(Fault::Refresh(RefreshReply::ForeignOwner)).await;
    let original = (client.session_token.clone(), client.authenticated_until);
    client.own_signaling().unwrap();
    assert!(client
        .refresh_credential("explicit-foreign-successor".into())
        .await
        .is_err());
    assert_eq!(
        (client.session_token.clone(), client.authenticated_until),
        original
    );
    assert!(client.closed.load(Ordering::Acquire));
    close(client, server).await;
}

#[tokio::test]
async fn signed_session_speaker_and_explicit_silence_remain_advisory_while_rpc_completes() {
    for owned in [false, true] {
        let (mut client, server, _) = controlled(Fault::Owned(Traffic::Speaker)).await;
        if owned {
            client.own_signaling().unwrap();
        }
        let batch = client.request(query(), scope()).await.unwrap();
        assert_eq!(batch.frames.len(), 1);
        assert_eq!(batch.frames[0].kind(), "ack");
        for expected in [json!("part_source"), Value::Null] {
            let hint = client.receive().await.unwrap();
            assert_eq!(hint.kind(), "stream.active-speaker");
            assert_eq!(hint.scope(Scope::Session), Some("session_one"));
            assert_eq!(hint.scope(Scope::Connection), None);
            assert_eq!(hint.original()["payload"]["active_participant"], expected);
            assert!(hint.original()["payload"]
                .get("active_participant")
                .is_some());
        }
        assert_current(&client);
        close(client, server).await;
    }
}
#[test]
fn incompatible_capabilities_uses_closed_numeric_error_mapping_without_private_diagnostics() {
    let error = connect_protocol::Error::new(
        "incompatible-capabilities",
        "private codec/provider diagnostic",
    );
    let problem = connect_protocol::canonical_http::Problem::from_error(&error);
    assert_eq!(problem.code, 488);
    assert_eq!(problem.category, "media");
    assert_eq!(problem.reason, "incompatible_capabilities");
    let ClientError::Application(remote) = remote_error(488) else {
        panic!()
    };
    assert_eq!(remote.code, "incompatible-capabilities");
    assert!(!serde_json::to_string(&problem).unwrap().contains("private"));
}
