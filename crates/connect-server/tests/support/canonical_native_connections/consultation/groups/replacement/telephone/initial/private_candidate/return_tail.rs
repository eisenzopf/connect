//! A real SIP prefix must not lend its unfinished private schedule to return.
use super::*;

const DIGITS: &str = "1234567890ABCD*#";
const OPERATION: &str = "private-sip-pending-at-return";
pub(super) struct Pending {
    scope: CommandScope,
    info_before: usize,
    public_dialog: Dialog,
}

fn call_id(packet: &[u8]) -> String {
    std::str::from_utf8(packet)
        .unwrap()
        .lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| name.eq_ignore_ascii_case("call-id"))
                .map(|(_, value)| value.trim().to_owned())
        })
        .unwrap()
}

pub(super) async fn begin(
    f: &Fixture,
    peer: &PeerSip,
    clients: &mut [Peer],
    cid: &str,
    phone: &str,
    private_call: &str,
    public_dialog: Dialog,
) -> Pending {
    let info_before = peer.info.lock().unwrap().len();
    let scope = CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        connection_id: Some(clients[1].id.clone()),
        operation_id: Some(OPERATION.into()),
        ..Default::default()
    };
    clients[1]
        .client
        .send_native_dtmf(scope.clone(), DIGITS, 1000, "info")
        .await
        .unwrap();
    let until = tokio::time::Instant::now() + Duration::from_secs(4);
    loop {
        if peer.info.lock().unwrap().len() > info_before {
            break;
        }
        assert!(
            tokio::time::Instant::now() < until,
            "private INFO prefix missing"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    {
        let packets = peer.info.lock().unwrap();
        assert_eq!(packets.len(), info_before + 1);
        assert_eq!(call_id(&packets[info_before]), private_call);
        assert!(std::str::from_utf8(&packets[info_before])
            .unwrap()
            .contains("Signal=1"));
    }
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let pending: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM native_dtmf_gateways g JOIN operations o ON o.record_id=g.operation_record_id WHERE o.operation_id=?1 AND json_extract(g.target_json,'$.connection_id')=?2 AND g.attempted=1 AND g.disposition IS NULL)",
        rusqlite::params![OPERATION,phone], |r|r.get(0),
    ).unwrap();
    assert!(
        pending,
        "return must overlap an unfinished actual SIP schedule"
    );
    Pending {
        scope,
        info_before,
        public_dialog,
    }
}

pub(super) async fn finish(
    f: &Fixture,
    peer: &PeerSip,
    clients: &mut [Peer],
    status: &mut CanonicalHttpClient,
    cid: &str,
    phone: &str,
    public: &str,
    private_call: &str,
    public_call: &str,
    pending: Pending,
) -> Value {
    let until = tokio::time::Instant::now() + Duration::from_secs(4);
    let stopped = loop {
        let row = settled(status, OPERATION, cid).await;
        if row["state"] == "unknown" && row["problem"]["signal"]["local_work_complete"] == true {
            break row;
        }
        assert!(
            tokio::time::Instant::now() < until,
            "pending private tail did not finish: {row}"
        );
        assert_eq!(row["state"], "unknown");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(stopped["state"], "unknown");
    let summary = &stopped["problem"]["signal"];
    assert_eq!(summary["local_work_complete"], true);
    let recipients = summary["recipients"].as_array().unwrap();
    assert_eq!(recipients.len(), 1);
    assert_eq!(recipients[0]["connection_id"], phone);
    assert_eq!(recipients[0]["disposition"], "outcome_unknown");
    let count_at_return = peer.info.lock().unwrap().len();
    assert!((1..DIGITS.len()).contains(&(count_at_return - pending.info_before)));
    let replay = clients[1]
        .client
        .send_native_dtmf(pending.scope, DIGITS, 1000, "info")
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(replay).unwrap(), stopped);
    // Cross two original tone +70ms gap boundaries after actual captured return.
    tokio::time::sleep(Duration::from_millis(2200)).await;
    assert_eq!(
        peer.info.lock().unwrap().len(),
        count_at_return,
        "old private tail emitted after return/replay"
    );
    for p in clients.iter_mut() {
        assert!(
            tokio::time::timeout(Duration::from_millis(200), signal(&mut p.client))
                .await
                .is_err()
        );
    }
    let private_infos = peer.info.lock().unwrap()[pending.info_before..]
        .iter()
        .map(|packet| call_id(packet))
        .collect::<Vec<_>>();
    assert!(private_infos.iter().all(|id| id == private_call));
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let original_targets: u64 = db.query_row(
        "SELECT count(*) FROM native_dtmf_gateways g JOIN operations o ON o.record_id=g.operation_record_id WHERE o.operation_id=?1 AND json_extract(g.target_json,'$.connection_id')=?2 AND g.attempted=1 AND g.disposition='outcome_unknown'",
        rusqlite::params![OPERATION,phone], |r|r.get(0),
    ).unwrap();
    assert_eq!(original_targets, 1);
    // The single peer serves two physical calls. Match the recovery response
    // to the saved public dialog; the retired private dialog is not its owner.
    assert_eq!(pending.public_dialog.call_id, public_call);
    *peer.dialog.lock().unwrap() = Some(pending.public_dialog);
    let fresh = "private-sip-fresh-public-after-stopped-tail";
    let source = clients[1].id.clone();
    clients[1]
        .client
        .send_native_dtmf(
            CommandScope {
                conversation_id: Some(cid.into()),
                session_id: Some("native-session".into()),
                connection_id: Some(source),
                operation_id: Some(fresh.into()),
                ..Default::default()
            },
            "6",
            100,
            "info",
        )
        .await
        .unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(5), signal(&mut clients[0].client))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(frame.scope(WireScope::Operation), Some(fresh));
    assert_eq!(frame.original()["payload"]["digits"], "6");
    let recovered = settled(status, fresh, cid).await;
    assert_eq!(recovered["state"], "completed");
    let recovered_targets = recovered["result"]["recipients"].as_array().unwrap();
    assert_eq!(recovered_targets.len(), 2);
    assert!(recovered_targets
        .iter()
        .any(|r| r["connection_id"] == public));
    assert!(recovered_targets
        .iter()
        .any(|r| r["connection_id"] == clients[0].id));
    assert!(recovered_targets
        .iter()
        .all(|r| r["connection_id"] != phone));
    let packets = peer.info.lock().unwrap();
    assert_eq!(packets.len(), count_at_return + 1);
    assert_eq!(call_id(packets.last().unwrap()), public_call);
    json!({"stopped":stopped,"replay_preserved":true,"original_target_rows":original_targets,
        "private_info_prefix_count":count_at_return-pending.info_before,"scheduled_digits":DIGITS.len(),
        "tail_quiet_ms":2200,"post_return_private_or_public_tail_count":0,
        "private_prefix_call_ids":private_infos,"fresh":recovered,"fresh_frame":frame.original(),
        "fresh_info_call_id":public_call})
}
