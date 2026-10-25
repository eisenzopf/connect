//! Actual committed native metadata, current read authority and restart cleanup.
use super::super::canonical_native_connections::{
    negotiate_media_payload, party_with_scope, ready_endpoint_with_capabilities, ControlReception,
};
use super::*;
fn capabilities() -> Value {
    json!({"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}},{"name":"pcma","params":{"sample_rate":8000,"channels":1}}],"video_codecs":[{"name":"vp8","params":{"sample_rate":90000,"channels":0}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"})
}
async fn control(
    owner: &mut CanonicalClient,
    cid: &str,
    operation: &str,
    revision: Option<u64>,
    kind: &str,
    payload: Value,
) -> Value {
    let scope = CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        connection_id: Some("metadata-connection".into()),
        operation_id: Some(operation.into()),
        expected_revision: revision,
    };
    let id = owner
        .send_value(kind, payload, scope.clone())
        .await
        .unwrap();
    let mut result = None;
    loop {
        let frame = owner.receive_control().await.unwrap();
        match frame.kind() {
            "connection.offer" => {
                let p = &frame.original()["payload"];
                let streams=p["substrate_setup"]["streams"].as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":match s["direction"].as_str().unwrap(){"sendonly"=>"recvonly","recvonly"=>"sendonly","sendrecv"=>"sendrecv",_=>panic!("inactive")},"codec":s["codec"]})).collect::<Vec<_>>();
                owner.answer_connection_offer(&frame,serde_json::from_value(json!({"by_participant":p["by_participant"],"substrate":"quic","capabilities":capabilities(),"streams_answered":streams,"substrate_setup":p["substrate_setup"]})).unwrap(),&scope).await.unwrap();
            }
            "operation.updated" => {
                assert_eq!(frame.scope(rvoip_uctp::v2::Scope::Reply), Some(id.as_str()));
                assert_eq!(
                    frame.original()["payload"]["state"],
                    "completed",
                    "{:?}",
                    frame.original()
                );
                result = Some(frame.original()["payload"]["result"].clone());
            }
            "ack" => return result.unwrap(),
            _ => panic!("unexpected control reply {:?}", frame.original()),
        }
    }
}
#[tokio::test]
async fn public_sdk_stream_metadata_tracks_controls_scoped_ids_authorization_and_normal_restart() {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 16).await;
        ready_endpoint_with_capabilities(&f, vec!["audio".into(), "video".into()]).await;
        let observer = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let room = observer
            .create_conversation(
                vec![f.peer.identity_id.parse().unwrap()],
                ConversationPolicy::Persistent,
                mutation("metadata-create"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        let view = room.refresh(options()).await.unwrap();
        let recipient = view
            .participants
            .iter()
            .find(|p| p.identity_id.as_str() == f.peer.identity_id)
            .unwrap()
            .id
            .clone();
        let caps = json!({"audio":["g.711-mu","g.711-a"],"video":["vp8"]});
        let session = room
            .invite(
                "native-session".parse().unwrap(),
                vec![recipient.clone()],
                sdk::SessionMedium::VoiceVideo,
                "conference".into(),
                caps.clone(),
                mutation("metadata-invite"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        let peer_config = party_with_scope(
            &f,
            &f.peer.identity_id,
            181,
            vec!["audio".into(), "video".into()],
            Scope::collaboration(vec!["*".into()]),
        );
        let mut peer = CanonicalClient::open(peer_config.clone()).await.unwrap();
        peer.session_command(connect_client::session_control::Request {
            operation_id: "metadata-accept".into(),
            conversation_id: room.id().as_str().into(),
            session_id: "native-session".into(),
            by_participant: None,
            expected_revision: Some(0),
            action: connect_client::session_control::Action::Accept { capabilities: caps },
        })
        .await
        .unwrap();
        let mut owner = CanonicalClient::open(f.client.clone()).await.unwrap();
        negotiate_media_payload(&mut owner,room.id().as_str(),"metadata-offer","metadata-connection",json!({"substrate":"quic","capabilities":capabilities(),"streams_offered":[{"id":"metadata-audio","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]},{"id":"metadata-video","kind":"video","direction":"recvonly","codec_preferences":["vp8"]}]})).await;
        let connection = session.connection_by_id("metadata-connection".parse().unwrap());
        let audio = connection.stream_by_id("metadata-audio".parse().unwrap());
        let video = connection.stream_by_id("metadata-video".parse().unwrap());
        let initial = audio.refresh(options()).await.unwrap();
        assert_eq!(initial.state, "negotiated");
        assert_eq!(initial.revision.get(), 1);
        assert_eq!(initial.medium, sdk::Medium::Audio);
        assert!(connection.native_negotiation().unwrap().is_none()); // Metadata read grants no physical ownership.
        let b = video.refresh(options()).await.unwrap();
        assert_eq!(b.medium, sdk::Medium::Video);
        let hold = control(
            &mut owner,
            room.id().as_str(),
            "metadata-hold",
            Some(1),
            "connection.update",
            json!({"action":"hold","streams":["metadata-audio"],"details":{"direction":"inactive"}}),
        )
        .await;
        let held = audio.refresh(options()).await.unwrap();
        assert_eq!(held.state, "paused");
        assert!(held.revision.get() > initial.revision.get());
        assert_eq!(video.refresh(options()).await.unwrap().revision, b.revision);
        let resume = control(
            &mut owner,
            room.id().as_str(),
            "metadata-resume",
            Some(hold["stream_revision"].as_u64().unwrap()),
            "connection.update",
            json!({"action":"resume","streams":["metadata-audio"]}),
        )
        .await;
        let resumed = audio.refresh(options()).await.unwrap();
        assert_eq!(resumed.state, "active");
        assert!(resumed.revision.get() > held.revision.get());
        assert_ne!(
            video.refresh(options()).await.unwrap().revision.get(),
            resume["stream_revision"].as_u64().unwrap()
        );
        control(
            &mut owner,
            room.id().as_str(),
            "metadata-codec",
            Some(resume["stream_revision"].as_u64().unwrap()),
            "connection.update",
            json!({"action":"codec-renegotiate","streams":["metadata-audio"],"codec_preferences":["pcma"]}),
        )
        .await;
        let changed = audio.refresh(options()).await.unwrap();
        assert!(changed.revision.get() > resumed.revision.get());
        assert_eq!(changed.id, initial.id);
        assert_eq!(changed.state, "active");
        assert!(changed.format.is_none());
        let peer_reader = Client::connect(peer_config.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let peer_stream = peer_reader
            .conversation_by_id(room.id().clone())
            .session_by_id(session.id().clone())
            .connection_by_id(connection.id().clone())
            .stream_by_id(audio.id().clone());
        assert_eq!(
            peer_stream.refresh(options()).await.unwrap().id,
            *audio.id()
        );
        let narrow_config = party_with_scope(
            &f,
            &f.peer.identity_id,
            182,
            vec!["audio".into()],
            Scope::collaboration(vec![f.peer.identity_id.clone()]),
        );
        let narrow = Client::connect(narrow_config, "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let hidden = narrow
            .conversation_by_id(room.id().clone())
            .session_by_id(session.id().clone())
            .connection_by_id(connection.id().clone())
            .stream_by_id(audio.id().clone());
        assert!(hidden.refresh(options()).await.is_err());
        narrow.close(options()).await.unwrap();
        let cached = peer
            .conversation_snapshot(
                room.id().as_str(),
                serde_json::from_value(json!({"limit":1})).unwrap(),
            )
            .await
            .unwrap();
        let page = cached.next_page.unwrap();
        let current = room.refresh(options()).await.unwrap();
        room.participant_by_id(recipient)
            .remove(current.revision, mutation("metadata-remove-peer"))
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        assert!(peer_stream.refresh(options()).await.is_err());
        assert!(peer
            .conversation_snapshot(
                room.id().as_str(),
                serde_json::from_value(json!({"limit":1,"page":page})).unwrap()
            )
            .await
            .is_err());
        control(
            &mut owner,
            room.id().as_str(),
            "metadata-end",
            None,
            "connection.end",
            json!({"reason_code":200,"reason":"normal"}),
        )
        .await;
        // Core termination completes before the offer worker journals retirement.
        // Await that distinct durable observation without repeating connection.end.
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let current = connection.refresh(options()).await.unwrap();
                if current.state == "retired" {
                    assert!(current.generation > 1);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("actual native retirement must reach the authorized snapshot");
        let ended = audio.refresh(options()).await.unwrap();
        assert_eq!(ended.state, "ended");
        assert!(ended.revision.get() > changed.revision.get());
        assert_eq!(video.refresh(options()).await.unwrap().state, "ended");
        owner.shutdown().await.unwrap();
        peer.shutdown().await.unwrap();
        peer_reader.close(options()).await.unwrap();
        observer.close(options()).await.unwrap();
        let Fixture {
            temp,
            config,
            server,
            mut client,
            ..
        } = f;
        server.shutdown().await.unwrap();
        let restarted = RunningCanonicalServer::start(config).await.unwrap();
        client.endpoint = restarted.address;
        let fresh = Client::connect(client, "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let restored = fresh
            .conversation_by_id(room.id().clone())
            .session_by_id(session.id().clone())
            .connection_by_id(connection.id().clone())
            .stream_by_id(audio.id().clone());
        let after = restored.refresh(options()).await.unwrap();
        assert_eq!(after.state, "ended");
        assert_eq!(after.revision, ended.revision);
        fresh.close(options()).await.unwrap();
        restarted.shutdown().await.unwrap();
        drop(temp);
    }
}
