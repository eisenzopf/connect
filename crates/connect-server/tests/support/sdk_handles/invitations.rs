//! Public SDK consumes actual server invitations without copied callee IDs.
use super::*;
async fn connect(config: CanonicalClientConfig) -> Client {
    Client::connect(config, "tnt_default".parse().unwrap())
        .await
        .unwrap()
}
async fn room(client: &Client, peer: &str, id: &str) -> sdk::Conversation {
    client
        .create_conversation(
            vec![peer.parse().unwrap()],
            ConversationPolicy::Persistent,
            mutation(id),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value
}
async fn target(room: &sdk::Conversation, identity: &str) -> sdk::ParticipantId {
    room.refresh(options())
        .await
        .unwrap()
        .participants
        .into_iter()
        .find(|p| p.identity_id.as_str() == identity)
        .unwrap()
        .id
}
async fn invite(
    room: &sdk::Conversation,
    to: sdk::ParticipantId,
    index: usize,
    medium: sdk::SessionMedium,
    caps: Value,
) -> sdk::Session {
    room.invite(
        format!("sdk-incoming-session-{index}").parse().unwrap(),
        vec![to],
        medium,
        "synchronous-engagement".into(),
        caps,
        mutation(&format!("sdk-incoming-original-{index}")),
    )
    .await
    .unwrap()
    .wait(options())
    .await
    .unwrap()
    .value
}
#[tokio::test]
async fn public_sdk_incoming_invitations_keep_original_metadata_and_decide_without_copied_callee_ids(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 16).await;
        let peer_config = super::super::canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            237,
            vec!["data".into()],
            Scope::collaboration(vec!["*".into()]),
        );
        let caller = connect(f.client.clone()).await;
        let callee = connect(peer_config).await;
        let room = room(&caller, &f.peer.identity_id, "sdk-incoming-room").await;
        let from = target(&room, &f.client.identity_id).await;
        let to = target(&room, &f.peer.identity_id).await;
        let caps = json!({"audio":["g.711-mu"],"video":["vp8"],"screen":["vp8"],"data":["reliable-ordered"],"future":{"claimed_actor":"not a grant"}});
        let kinds = [
            sdk::SessionMedium::Voice,
            sdk::SessionMedium::Video,
            sdk::SessionMedium::VoiceVideo,
            sdk::SessionMedium::ScreenShare,
            sdk::SessionMedium::TextChat,
            sdk::SessionMedium::Mixed,
        ];
        for (index, medium) in kinds.into_iter().enumerate() {
            let original = invite(&room, to.clone(), index, medium, caps.clone()).await;
            // Attach after committed invitation; reception stays owned by the SDK worker.
            let mut incoming = callee.incoming_invitations(options());
            let hint = incoming.next().await.unwrap().unwrap();
            assert_eq!(hint.from_participant, from);
            assert_eq!(hint.to_participant, to);
            assert_eq!(
                hint.operation_id.as_str(),
                format!("sdk-incoming-original-{index}")
            );
            assert_eq!(hint.source.kind, "session.invite");
            assert_eq!(hint.source.correlation_id, hint.operation_id.as_str());
            assert_eq!(hint.capabilities_offer, caps);
            assert_eq!(hint.intent, "synchronous-engagement");
            assert_eq!(hint.session.id(), original.id());
            assert_eq!(hint.session.conversation_id(), room.id());
            assert!(hint.session.receipt_snapshot().is_none());
            assert!(!format!("{hint:?}").contains("claimed_actor"));
            let current = hint.session.refresh(options()).await.unwrap();
            assert_eq!(current.state, "invited");
            if index % 2 == 0 {
                let accepted = hint
                    .session
                    .accept(
                        caps.clone(),
                        current.topology_revision,
                        mutation(&format!("sdk-incoming-accept-{index}")),
                    )
                    .await
                    .unwrap()
                    .wait(options())
                    .await
                    .unwrap();
                assert_eq!(
                    accepted.value.refresh(options()).await.unwrap().state,
                    "accepted"
                );
                let current = original.refresh(options()).await.unwrap();
                original
                    .end(
                        200,
                        "complete".into(),
                        current.topology_revision,
                        mutation(&format!("sdk-incoming-end-{index}")),
                    )
                    .await
                    .unwrap()
                    .wait(options())
                    .await
                    .unwrap();
            } else {
                let rejected = hint
                    .session
                    .reject(
                        486,
                        "Unavailable".into(),
                        current.topology_revision,
                        mutation(&format!("sdk-incoming-reject-{index}")),
                    )
                    .await
                    .unwrap()
                    .wait(options())
                    .await
                    .unwrap();
                assert_eq!(rejected.value.state, "rejected");
            }
            incoming.close().await.unwrap();
            assert!(incoming.next().await.unwrap().is_none());
        }
        let counts=f.server.store.connection().unwrap().query_row("SELECT (SELECT COUNT(*) FROM native_connection_lifetimes),(SELECT COUNT(*) FROM sessions WHERE state IN ('invited','accepted','active'))",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).unwrap();
        assert_eq!(counts,(0,0),"metadata/decisions cannot allocate native ownership and all logical Sessions are terminal");
        callee.close(options()).await.unwrap();
        caller.close(options()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), f.server.shutdown())
            .await
            .unwrap()
            .unwrap();
        eprintln!("SDK incoming {algorithm}: all6 media intents, original metadata, callee discovers all IDs from stream, fresh current accept/reject decisions, no native ownership, terminal Sessions and joined shutdown");
    }
}
#[tokio::test]
async fn public_sdk_stale_invitation_hint_grants_no_acceptance_after_cancel_or_membership_removal()
{
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 16).await;
        let config = super::super::canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            238,
            vec!["data".into()],
            Scope::collaboration(vec!["*".into()]),
        );
        let caller = connect(f.client.clone()).await;
        let callee = connect(config).await;
        let room = room(&caller, &f.peer.identity_id, "sdk-incoming-stale-room").await;
        let to = target(&room, &f.peer.identity_id).await;
        let caps = json!({"audio":["g.711-mu"]});
        let original = invite(
            &room,
            to.clone(),
            0,
            sdk::SessionMedium::Voice,
            caps.clone(),
        )
        .await;
        let request = options();
        let cancellation = request.signal.clone();
        let mut incoming = callee.incoming_invitations(request);
        let stale = incoming.next().await.unwrap().unwrap();
        let before = original.refresh(options()).await.unwrap();
        original
            .cancel_invitation(
                487,
                "cancelled".into(),
                before.topology_revision,
                mutation("sdk-incoming-cancel"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        let current = stale.session.refresh(options()).await.unwrap();
        assert_eq!(current.state, "cancelled");
        assert!(stale
            .session
            .accept(
                caps.clone(),
                current.topology_revision,
                mutation("sdk-incoming-late-accept")
            )
            .await
            .is_err());
        assert_eq!(
            original.refresh(options()).await.unwrap().state,
            "cancelled"
        );
        cancellation.cancel();
        assert!(matches!(incoming.next().await,Err(e) if e.kind==sdk::ErrorKind::Cancelled));
        incoming.close().await.unwrap();
        room.refresh(options()).await.unwrap();
        let second = invite(
            &room,
            to.clone(),
            1,
            sdk::SessionMedium::Voice,
            caps.clone(),
        )
        .await;
        let mut replacement = callee.incoming_invitations(options());
        let stale = replacement.next().await.unwrap().unwrap();
        assert_eq!(stale.session.id(), second.id());
        let before = room.refresh(options()).await.unwrap();
        room.participant_by_id(to)
            .remove(before.revision, mutation("sdk-incoming-remove"))
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        assert!(stale.session.refresh(options()).await.is_err());
        assert!(stale
            .session
            .accept(
                caps,
                sdk::TopologyRevision::try_from(0).unwrap(),
                mutation("sdk-incoming-removed-accept")
            )
            .await
            .is_err());
        replacement.close().await.unwrap();
        callee.close(options()).await.unwrap();
        caller.close(options()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), f.server.shutdown())
            .await
            .unwrap()
            .unwrap();
        eprintln!("SDK stale incoming {algorithm}: original cancelled hint and removed membership cannot acquire acceptance/custody; reader cancellation/replacement preserves ordinary control; shutdown joins pending notification work");
    }
}
