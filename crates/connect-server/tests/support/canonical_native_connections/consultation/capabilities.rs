//! Real signed descriptor delivery follows the current private media scene.
use super::*;

async fn declare(
    f: &Fixture,
    client: &mut CanonicalClient,
    cid: &str,
    connection: &str,
    operation: &str,
) -> (String, Value) {
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let (participant, revision): (String, u64) = c.query_row(
        "SELECT n.participant_id,json_extract(c.readiness_json,'$.stream_revision') FROM native_admission_sources n JOIN connections c ON c.id=n.connection_id WHERE n.connection_id=?1 AND n.state='completed' ORDER BY n.generation DESC LIMIT 1",
        [connection], |r| Ok((r.get(0)?, r.get(1)?)),
    ).unwrap();
    drop(c);
    let descriptor = json!({"audio_codecs":[{"name":"pcmu","params":{}}],"future":{"private_descriptor":operation,"exact":[true,37]}});
    let result = client
        .advertise_native_capabilities(
            CommandScope {
                conversation_id: Some(cid.into()),
                session_id: Some("native-session".into()),
                connection_id: Some(connection.into()),
                operation_id: Some(operation.into()),
                expected_revision: Some(revision),
            },
            &participant,
            descriptor.clone(),
            "endpoint-change",
        )
        .await
        .unwrap();
    assert_eq!(
        result.state,
        rvoip_uctp::v2::payloads::OperationState::Completed
    );
    (participant, descriptor)
}

async fn received(
    client: &mut CanonicalClient,
    algorithm: &str,
    cid: &str,
    source: &str,
    participant: &str,
    descriptor: &Value,
) {
    let frame = super::super::capabilities::next_advertisement(client, algorithm).await;
    assert_eq!(frame.original()["cid"], cid);
    assert_eq!(frame.original()["sid"], "native-session");
    assert_eq!(frame.original()["connid"], source);
    assert_eq!(frame.original()["payload"]["by_participant"], participant);
    assert_eq!(frame.original()["payload"]["capabilities"], *descriptor);
    assert!(frame.original().get("operation_id").is_none());
    assert!(frame.original().get("in_reply_to").is_none());
}

pub(super) async fn private(
    f: &Fixture,
    probes: &mut [CanonicalClient],
    cid: &str,
    algorithm: &str,
) {
    // Actual fixture order: private consultant, primary, retained remote,
    // public consultant. The last shares the private consultant's Participant.
    for (source, target, connection, operation) in [
        (1, 0, "codec-target", "private-primary-descriptor"),
        (0, 1, "private-consultant", "private-consultant-descriptor"),
    ] {
        let (participant, descriptor) =
            declare(f, &mut probes[source], cid, connection, operation).await;
        received(
            &mut probes[target],
            algorithm,
            cid,
            connection,
            &participant,
            &descriptor,
        )
        .await;
        for _ in 0..3 {
            super::super::capabilities::barrier_without_advertisement(&mut probes[2]).await;
            super::super::capabilities::barrier_without_advertisement(&mut probes[3]).await;
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }
}

pub(super) async fn returned(
    f: &Fixture,
    probes: &mut [CanonicalClient],
    cid: &str,
    algorithm: &str,
) {
    // Actual return order: retained remote, primary, retired private, retired
    // public. A fresh public descriptor must reach the restored original peer.
    super::super::capabilities::barrier_without_advertisement(&mut probes[0]).await;
    let (participant, descriptor) = declare(
        f,
        &mut probes[1],
        cid,
        "codec-target",
        "returned-public-descriptor",
    )
    .await;
    received(
        &mut probes[0],
        algorithm,
        cid,
        "codec-target",
        &participant,
        &descriptor,
    )
    .await;
}
