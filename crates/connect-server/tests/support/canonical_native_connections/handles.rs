//! Actual peer-local handles across two Connections and an independent peer.
use super::lifecycle::session_start;
use super::*;
use rvoip_uctp::substrate::{pack_rtp_datagram, unpack_rtp_datagram, RtpDatagram, RtpMediaPayload};
use std::collections::{BTreeMap, BTreeSet};

fn handle(answer: &Envelope) -> u16 {
    let id = answer.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
        .as_u64()
        .unwrap();
    assert!((1..=u64::from(u16::MAX)).contains(&id));
    id as u16
}
async fn publish(client: &CanonicalClient, handles: &[u16], sequence: u32) {
    for seq in sequence..sequence + 60 {
        for &stream_local_id in handles {
            client
                .send_native_media_datagram(
                    &pack_rtp_datagram(&RtpDatagram {
                        flags: 0,
                        stream_local_id,
                        seq,
                        rtp: RtpMediaPayload {
                            payload: (0..160)
                                .map(|n| if n % 16 < 8 { 0x80 } else { 0x00 })
                                .collect::<Vec<_>>()
                                .into(),
                            payload_type: 0,
                            sequence_number: seq as u16,
                            timestamp: seq.wrapping_mul(160),
                            ssrc: 0x4830_0000 + u32::from(stream_local_id),
                        },
                    })
                    .unwrap(),
                )
                .unwrap();
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
fn decoded(payload: &[u8]) -> usize {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let mut decoder = Command::new("ffmpeg")
        .args([
            "-v", "error", "-f", "mulaw", "-ar", "8000", "-ac", "1", "-i", "pipe:0", "-f", "s16le",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    decoder.stdin.take().unwrap().write_all(payload).unwrap();
    let output = decoder.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout.len(), payload.len() * 2);
    let loud = output
        .stdout
        .chunks_exact(2)
        .filter(|v| i16::from_le_bytes([v[0], v[1]]).unsigned_abs() > 500)
        .count();
    assert!(loud > 500);
    loud
}
async fn flow(
    source: &CanonicalClient,
    target: &mut CanonicalClient,
    sending: &[u16],
    receiving: &[u16],
    sequence: u32,
) -> Value {
    let receive = async {
        let mut counts = receiving
            .iter()
            .map(|&h| (h, 0usize))
            .collect::<BTreeMap<_, _>>();
        let mut payloads = receiving
            .iter()
            .map(|&h| (h, Vec::new()))
            .collect::<BTreeMap<_, _>>();
        while counts.values().any(|n| *n <= 10) {
            let bytes = target.receive_native_media_datagram().await.unwrap();
            let packet = unpack_rtp_datagram(&bytes).unwrap();
            assert_eq!(packet.rtp.payload_type, 0);
            assert!(receiving.contains(&packet.stream_local_id));
            if packet
                .rtp
                .payload
                .iter()
                .any(|b| !matches!(*b, 0xff | 0x7f))
            {
                *counts.get_mut(&packet.stream_local_id).unwrap() += 1;
                payloads
                    .get_mut(&packet.stream_local_id)
                    .unwrap()
                    .extend_from_slice(&packet.rtp.payload);
            }
        }
        counts.iter().map(|(h,n)| json!({"stream_local_id":h,"marked_frames":n,"decoded_loud_samples":decoded(&payloads[h])})).collect::<Vec<_>>()
    };
    let (_, observed) = tokio::join!(
        publish(source, sending, sequence),
        tokio::time::timeout(Duration::from_secs(4), receive)
    );
    json!(observed.expect("both current peer-local mappings must carry decoded audio"))
}
async fn barrier(client: &mut CanonicalClient) {
    let id = client.keepalive().await.unwrap();
    loop {
        let frame = client.receive().await.unwrap();
        if session_start(&frame) {
            continue;
        }
        assert_eq!(
            frame.kind(),
            "ack",
            "unexpected pre-flow event: {:?}",
            frame.original()
        );
        assert_eq!(frame.original()["in_reply_to"], id);
        break;
    }
}
async fn opened(client: &mut CanonicalClient, expected: &[(&str, &str, u16)]) -> Vec<Value> {
    let mut ready = BTreeSet::new();
    let mut opened = BTreeSet::new();
    let mut frames = Vec::new();
    while opened.len() < expected.len() {
        let frame = client.receive().await.unwrap();
        if session_start(&frame) {
            continue;
        }
        let conn = frame.original()["connid"].as_str().unwrap();
        let (_, logical, handle) = expected
            .iter()
            .find(|(id, _, _)| *id == conn)
            .expect("foreign physical lifecycle fact");
        match frame.kind() {
            "connection.ready" => {
                assert!(ready.insert(conn.to_owned()));
            }
            "stream.opened" => {
                assert!(ready.contains(conn));
                assert!(opened.insert(conn.to_owned()));
                let stream = &frame.original()["payload"]["stream"];
                assert_eq!(stream["strm_id"], *logical);
                assert_eq!(stream["stream_local_id"], *handle);
                assert_eq!(stream["kind"], "audio");
                assert_eq!(stream["codec"]["name"], "pcmu");
                assert_eq!(stream["direction"], "sendrecv");
                frames.push(frame.original().clone());
            }
            "connection.quality" => (),
            other => panic!("unexpected handle lifecycle fact: {other}"),
        }
    }
    frames
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_native_stream_handles_keep_same_peer_connections_distinct_and_foreign_peer_cannot_borrow_them(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 12).await;
        ready_endpoint(&f).await;
        let source_config = party_with_streams(&f, &f.peer.identity_id, 147, true);
        let mut target = CanonicalClient::open(f.client.clone()).await.unwrap();
        let mut source = CanonicalClient::open(source_config.clone()).await.unwrap();
        let cid = session(&f, &mut target).await;
        let a = negotiate_named(
            &mut target,
            &cid,
            "handles-offer-a",
            "handles-a",
            "handles-monitor-a",
        )
        .await;
        let b = negotiate_named(
            &mut target,
            &cid,
            "handles-offer-b",
            "handles-b",
            "handles-monitor-b",
        )
        .await;
        let s = negotiate_named(
            &mut source,
            &cid,
            "handles-offer-source",
            "handles-source",
            "handles-voice",
        )
        .await;
        for (credential, operation) in [
            (&f.client.credential, "handles-offer-a"),
            (&f.client.credential, "handles-offer-b"),
            (&source_config.credential, "handles-offer-source"),
        ] {
            wait_native_operation(&f, credential, &cid, operation).await;
        }
        let (ha, hb, hs) = (handle(&a), handle(&b), handle(&s));
        assert_ne!(
            ha, hb,
            "one physical peer cannot alias two Connection handles"
        );
        for (conn, operation) in [
            ("handles-a", "handles-select-a"),
            ("handles-b", "handles-select-b"),
        ] {
            assert!(stream_control(
                &mut target,
                &cid,
                conn,
                operation,
                1,
                "stream.subscribe",
                json!({"subscriptions":[{"strm_id":"handles-voice"}]})
            )
            .await
            .is_none());
        }
        assert!(stream_control(&mut source, &cid, "handles-source", "handles-return", 1, "stream.subscribe", json!({"subscriptions":[{"strm_id":"handles-monitor-a"},{"strm_id":"handles-monitor-b"}]})).await.is_none());
        barrier(&mut target).await;
        barrier(&mut source).await;
        let foreign = [ha, hb].into_iter().find(|h| *h != hs).unwrap();
        let foreign_input = [foreign];
        let (_, empty) = tokio::join!(
            publish(&source, &foreign_input, 80000),
            tokio::time::timeout(
                Duration::from_millis(900),
                target.receive_native_media_datagram()
            )
        );
        assert!(empty.is_err(), "foreign physical handle emitted a datagram");
        barrier(&mut target).await;
        barrier(&mut source).await;
        let forward = flow(&source, &mut target, &[hs], &[ha, hb], 80100).await;
        let reverse = flow(&target, &mut source, &[ha, hb], &[hs], 80200).await;
        let target_opened = opened(
            &mut target,
            &[
                ("handles-a", "handles-monitor-a", ha),
                ("handles-b", "handles-monitor-b", hb),
            ],
        )
        .await;
        let source_opened = opened(&mut source, &[("handles-source", "handles-voice", hs)]).await;
        if let Some(path) = std::env::var_os("CONNECT_NATIVE_HANDLE_OBSERVATIONS") {
            use std::io::Write;
            let path = std::path::PathBuf::from(path);
            assert!(
                path.is_absolute()
                    && path.starts_with(
                        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                            .parent()
                            .unwrap()
                            .parent()
                            .unwrap()
                            .join(".local/gate-6")
                    )
            );
            let mut bytes = serde_json::to_vec(&json!({"algorithm":algorithm,"conversation_id":cid,"target_bindings":[a.original(),b.original()],"source_binding":s.original(),"foreign_handle":foreign,"foreign_input_produced_no_output_or_lifecycle":true,"forward":forward,"reverse":reverse,"target_opened":target_opened,"source_opened":source_opened})).unwrap();
            bytes.push(b'\n');
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .unwrap()
                .write_all(&bytes)
                .unwrap();
        }
        target.close();
        source.close();
        f.server.shutdown().await.unwrap();
    }
}
