#![allow(clippy::unwrap_used)]
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_media.rs"]
mod media;
#[path = "support/native_tasks.rs"]
mod native_tasks;
mod support;
use connect_client::ConnectClient;
use media::Fixture;
use rvoip_uctp::{envelope::UctpEnvelope, types::MessageType};
use serde_json::json;

#[tokio::test]
async fn scoped_media_envelopes_cannot_amplify_authority_or_forge_context() {
    let mut f = Fixture::start().await;
    let (grant, config) = f.grant("media-policy", 120, Fixture::permissions()).await;
    let mut sdk = ConnectClient::open(config.clone()).await.unwrap();
    let session = f.session(&mut sdk, "invite").await;
    let mut wire = support::Peer::quic(&config).await;
    assert!(wire.authenticate(grant.expose_secret()).await);
    let participant = connect_protocol::tasks::task_grant_subject(&grant.grant.id).unwrap();
    let invite = json!({"from":participant,"to":[f.h.peer_id],"medium":"voice","intent":"connect-session","capabilities_offer":{"audio":["g.711-mu"]}});
    let offer = json!({"by_participant":participant,"substrate":"quic","capabilities":{},"substrate_setup":null,"streams_offered":[{"id":"audio","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]});
    let mut wrong_sender = invite.clone();
    wrong_sender["from"] = json!(f.h.peer_id);
    let mut wrong_target = invite.clone();
    wrong_target["to"] = json!([config.identity_id]);
    let mut wrong_direction = offer.clone();
    wrong_direction["streams_offered"][0]["direction"] = json!("invalid");
    let mut video = offer.clone();
    video["streams_offered"][0]["kind"] = json!("video");
    let mut wrong_codec = offer.clone();
    wrong_codec["streams_offered"][0]["codec_preferences"] = json!(["opus"]);
    let mut wrong_substrate = offer.clone();
    wrong_substrate["substrate"] = json!("websocket+webrtc");
    let mut wrong_publisher = offer.clone();
    wrong_publisher["by_participant"] = json!(f.h.peer_id);
    let before = connect_store::Store::inspect_snapshot(f.h.server.store.path()).unwrap();
    let cases = vec![
        UctpEnvelope::new(MessageType::ConversationCreate, json!({})),
        UctpEnvelope::new(MessageType::ConversationClose, json!({})).with_cid(&f.h.cid),
        UctpEnvelope::new(MessageType::IdentityStepUpResponse, json!({"method":"bearer","credential":"forged"})),
        UctpEnvelope::new(MessageType::MessageSend, json!({})).with_cid(&f.h.cid),
        UctpEnvelope::new(MessageType::SessionInvite, invite.clone()).with_cid("wrong-conversation").with_sid(&session.id),
        UctpEnvelope::new(MessageType::SessionInvite, invite.clone()).with_cid(&f.h.cid).with_sid("unowned-session"),
        UctpEnvelope::new(MessageType::SessionInvite, invite).with_sid(&session.id),
        UctpEnvelope::new(MessageType::SessionInvite, wrong_sender).with_cid(&f.h.cid).with_sid(&session.id),
        UctpEnvelope::new(MessageType::SessionInvite, wrong_target).with_cid(&f.h.cid).with_sid(&session.id),
        UctpEnvelope::new(MessageType::ConnectionOffer, offer).with_sid(&session.id),
        UctpEnvelope::new(MessageType::ConnectionOffer, wrong_direction).with_sid(&session.id).with_connid("connection"),
        UctpEnvelope::new(MessageType::ConnectionOffer, video).with_sid(&session.id).with_connid("connection"),
        UctpEnvelope::new(MessageType::ConnectionOffer, wrong_codec).with_sid(&session.id).with_connid("connection"),
        UctpEnvelope::new(MessageType::ConnectionOffer, wrong_substrate).with_sid(&session.id).with_connid("connection"),
        UctpEnvelope::new(MessageType::ConnectionOffer, wrong_publisher).with_sid(&session.id).with_connid("connection"),
        UctpEnvelope::new(MessageType::StreamSubscribe, json!({"by_participant":f.h.peer_id,"subscriptions":[{"from_participant":config.identity_id}]})).with_sid(&session.id).with_connid("connection"),
        UctpEnvelope::new(MessageType::StreamSubscribe, json!({"by_participant":participant,"subscriptions":[{"from_participant":config.identity_id}]})).with_sid(&session.id).with_connid("connection"),
        UctpEnvelope::new(MessageType::ConnectionEnd, json!({"reason_code":200,"reason":"foreign"})).with_cid("wrong").with_sid(&session.id).with_connid("connection"),
        UctpEnvelope::new(MessageType::SessionEnd, json!({"by":config.identity_id,"reason_code":200,"reason":"forged"})).with_sid(&session.id),
    ];
    for request in cases {
        let kind = request.msg_type.clone();
        let reply = wire.exchange(request).await;
        assert_eq!(reply.msg_type, MessageType::Error, "{kind:?}");
        assert_eq!(reply.payload["code"], 403, "{kind:?}");
    }
    assert_eq!(
        connect_store::Store::inspect_snapshot(f.h.server.store.path()).unwrap(),
        before,
        "denied wire attempts created durable work"
    );
    // The denials neither poison media admission nor promote the grant to the
    // parent's raw scopes. A new authorized SDK peer can still attach.
    let audio = sdk
        .open_native_audio(&f.h.cid, &session.id, &f.h.peer_id)
        .await
        .unwrap();
    f.binding(&grant.grant.id, "ready").await;
    let foreign_end = wire
        .exchange(
            UctpEnvelope::new(
                MessageType::ConnectionEnd,
                json!({"reason_code":200,"reason":"foreign-physical-peer"}),
            )
            .with_sid(&session.id)
            .with_connid(&audio.connection_id),
        )
        .await;
    assert_eq!(foreign_end.msg_type, MessageType::Error);
    // Connection IDs belong to a physical peer, even when both peers carry
    // the same grant. Stock UCTP conceals foreign connections as unknown.
    assert_eq!(foreign_end.payload["code"], 404);
    assert_eq!(foreign_end.payload["reason"], "unknown-connid");
    let foreign_subscription = wire.exchange(UctpEnvelope::new(
        MessageType::StreamSubscribe, json!({"by_participant":participant,"subscriptions":[{"strm_id":audio.publisher_stream_id}]}),
    ).with_sid(&session.id).with_connid(&audio.connection_id)).await;
    assert_eq!(foreign_subscription.msg_type, MessageType::Error);
    assert_eq!(foreign_subscription.payload["code"], 404);
    assert_eq!(foreign_subscription.payload["reason"], "unknown-connid");
    f.binding(&grant.grant.id, "ready").await;

    sdk.close_native_audio(&audio).await.unwrap();
    wire.close().await;
    sdk.close().await;
    f.close().await;
}
