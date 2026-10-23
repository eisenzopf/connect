//! A returned engagement cannot invent the departed primary. Surviving peers
//! may make independent current public subscription decisions afterward.
use super::*;
pub(super) async fn prove(
    f: &Fixture,
    cid: &str,
    public_config: &CanonicalClientConfig,
    peers: &mut [CanonicalClient],
    public_handle: u16,
    remote_handle: u16,
    remote_clock: tokio::time::Instant,
) {
    // Current order: old public AI context, retained remote. No new program
    // has been selected; the old primary and private candidate are both gone.
    let deadline = tokio::time::sleep(Duration::from_millis(250));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _=&mut deadline=>break,
            packet=peers[1].receive_native_rtp_datagram()=>{
                let p=rvoip_uctp::substrate::unpack_rtp_datagram(&packet.unwrap()).unwrap();
                assert!(p.rtp.payload.iter().all(|b|matches!(*b,0xff|0x7f)),"departed private source leaked during restoration");
            }
        }
    }
    let page = peers[1]
        .list_conversations(Default::default())
        .await
        .unwrap();
    let room = page.conversations.iter().find(|c| c.id == cid).unwrap();
    let ai = room
        .participants
        .iter()
        .find(|p| p.identity_id == public_config.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let remote = room
        .participants
        .iter()
        .find(|p| p.identity_id == f.peer.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let revision = |connection: &str| -> u64 {
        rusqlite::Connection::open(&f.config.database).unwrap().query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",[connection],|r|r.get(0)).unwrap()
    };
    stream_control(
        &mut peers[1],
        cid,
        "codec-source",
        "surviving-remote-selects-public-ai",
        revision("codec-source"),
        "stream.subscribe",
        json!({"subscriptions":[{"from_participant":ai,"kinds":["audio"]}]}),
    )
    .await;
    let public_clock = tokio::time::Instant::now();
    clocked_audio_reaches(peers, public_handle, remote_handle, 125100, public_clock).await;
    peers.swap(0, 1);
    stream_control(
        &mut peers[1],
        cid,
        "public-consultant",
        "public-ai-selects-surviving-remote",
        revision("public-consultant"),
        "stream.subscribe",
        json!({"subscriptions":[{"from_participant":remote,"kinds":["audio"]}]}),
    )
    .await;
    clocked_audio_reaches(peers, remote_handle, public_handle, 126100, remote_clock).await;
    peers.swap(0, 1);
    let history = peers[1]
        .history(cid, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    assert!(!serde_json::to_string(&history.messages)
        .unwrap()
        .contains("private provider transcript marker"));
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let dead:u64=c.query_row("SELECT COUNT(*) FROM connections WHERE id='codec-target' AND state IN ('retired','failed')",[],|r|r.get(0)).unwrap();
    assert_eq!(dead, 1);
    let attempts: u64 = c
        .query_row(
            "SELECT COUNT(*) FROM native_consultation_returns WHERE started=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(attempts, 1);
}
