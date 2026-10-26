//! Actual production listener exercises public owned SDK handles and recovery.
#[path = "sdk_handles/administration.rs"]
mod administration;
#[path = "sdk_handles/codec_updates.rs"]
mod codec_updates;
#[path = "sdk_handles/data.rs"]
mod data;
#[path = "sdk_handles/events.rs"]
mod events;
#[path = "sdk_handles/invitations.rs"]
mod invitations;
#[path = "sdk_handles/media.rs"]
mod media;
#[path = "sdk_handles/raw.rs"]
mod raw;
#[path = "sdk_handles/receipts.rs"]
mod receipts;
#[path = "sdk_handles/reconciliations.rs"]
mod reconciliations;
#[path = "sdk_handles/runtime_controls.rs"]
mod runtime_controls;
#[path = "sdk_handles/selections.rs"]
mod selections;
#[path = "sdk_handles/stream_metadata.rs"]
mod stream_metadata;
use super::*;
use connect_client::sdk::{
    self, Client, ConversationPolicy, MutationOptions, NonEmpty, RequestOptions,
};
fn options() -> RequestOptions {
    RequestOptions::new(Duration::from_secs(10)).unwrap()
}
fn mutation(id: &str) -> MutationOptions {
    MutationOptions {
        operation_id: id.parse().unwrap(),
        request: options(),
        extensions: None,
    }
}
#[tokio::test]
async fn public_sdk_native_multistream_offers_share_peer_keep_status_recovery_and_own_only_local_parameters(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 16).await;
        super::canonical_native_connections::ready_endpoint_with_capabilities(
            &f,
            vec!["audio".into(), "video".into(), "data".into()],
        )
        .await;
        let client = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let room = client
            .create_conversation(
                vec![f.peer.identity_id.parse().unwrap()],
                ConversationPolicy::Persistent,
                mutation("native-sdk-create"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        let session = room
            .invite(
                "native-sdk-session".parse().unwrap(),
                vec![],
                sdk::SessionMedium::Mixed,
                "native-sdk-solo".into(),
                json!({"audio":["g.711-mu","g.711-a"],"video":["vp8"],"data":["reliable-ordered"]}),
                mutation("native-sdk-invite"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        assert_eq!(session.refresh(options()).await.unwrap().state, "accepted");
        let stream = |id: &str, kind, direction, codec: &str| sdk::OfferedStream {
            id: id.parse().unwrap(),
            kind,
            direction,
            codecs: NonEmpty {
                first: codec.into(),
                rest: vec![],
            },
        };
        let first = NonEmpty {
            first: stream(
                "native-sdk-audio",
                sdk::Medium::Audio,
                sdk::Direction::Sendonly,
                "g.711-mu",
            ),
            rest: vec![
                stream(
                    "native-sdk-video",
                    sdk::Medium::Video,
                    sdk::Direction::Recvonly,
                    "vp8",
                ),
                stream(
                    "native-sdk-data",
                    sdk::Medium::Data,
                    sdk::Direction::Sendrecv,
                    "json",
                ),
            ],
        };
        let mut first = first;
        first.first.codecs.rest.push("pcma".into());
        let second = NonEmpty {
            first: stream(
                "native-sdk-audio",
                sdk::Medium::Audio,
                sdk::Direction::Sendrecv,
                "g.711-mu",
            ),
            rest: vec![],
        };
        let (one, two) = tokio::join!(
            session.offer_connection(
                "native-sdk-one".parse().unwrap(),
                first.clone(),
                mutation("native-sdk-offer-one")
            ),
            session.offer_connection(
                "native-sdk-two".parse().unwrap(),
                second,
                mutation("native-sdk-offer-two")
            )
        );
        let one = one.unwrap();
        let two = two.unwrap();
        let saved = one.reference_json();
        let one = one.wait(options()).await.unwrap().value;
        let two = two.wait(options()).await.unwrap().value;
        let a = one.native_negotiation().unwrap().unwrap();
        let b = two.native_negotiation().unwrap().unwrap();
        assert_eq!(a.streams.len(), 3);
        assert_eq!(b.streams.len(), 1);
        assert_eq!(a.streams[0].direction, sdk::Direction::Sendonly);
        assert_eq!(a.streams[1].direction, sdk::Direction::Recvonly);
        assert_eq!(a.streams[2].medium, sdk::Medium::Data);
        let handles: std::collections::BTreeSet<_> = a
            .streams
            .iter()
            .chain(&b.streams)
            .filter_map(|s| s.stream_local_id)
            .collect();
        assert_eq!(handles.len(), 3);
        for connection in [&one, &two] {
            let stream = connection.stream_by_id("native-sdk-audio".parse().unwrap());
            let snapshot = stream.refresh(options()).await.unwrap();
            assert_eq!(&snapshot.connection_id, connection.id());
            assert_eq!(snapshot.medium, sdk::Medium::Audio);
        }

        for connection in [&one, &two] {
            let current = connection.refresh(options()).await.unwrap();
            assert_eq!(&current.id, connection.id());
            assert_eq!(&current.session_id, session.id());
            assert_eq!(&current.conversation_id, room.id());
            assert!(current.generation > 0);
            assert_eq!(current.stream_revision, None); // The journal does not echo negotiated revision.
            assert_eq!(connection.receipt_stream_revision().unwrap().get(), 1);
        }
        // Same-ID replay may return a durable result directly, without a second
        // negotiation. Its original intent must remain byte-for-byte unchanged.
        let replay = session
            .offer_connection(
                "native-sdk-one".parse().unwrap(),
                first.clone(),
                mutation("native-sdk-offer-one"),
            )
            .await
            .unwrap();
        assert_eq!(replay.wait(options()).await.unwrap().value.id(), one.id());
        let mut changed = first;
        changed.first.direction = sdk::Direction::Sendrecv;
        let result = session
            .offer_connection(
                "native-sdk-one".parse().unwrap(),
                changed,
                mutation("native-sdk-offer-one"),
            )
            .await;
        assert!(
            matches!(result,Err(ref e) if e.certainty==sdk::Certainty::Refused && e.operation.as_ref().unwrap().operation_id.as_str()=="native-sdk-offer-one")
        );
        let audio = one.stream_by_id("native-sdk-audio".parse().unwrap());
        let targets = || NonEmpty {
            first: audio.id().clone(),
            rest: vec![],
        };
        one.update(
            sdk::ConnectionUpdate::Mute { streams: targets() },
            sdk::StreamRevision::try_from(1).unwrap(),
            mutation("sdk-mixed-codec-mute"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap();
        let codec = one
            .update(
                sdk::ConnectionUpdate::CodecRenegotiate {
                    streams: Some(targets()),
                    codec_preferences: NonEmpty {
                        first: "pcma".into(),
                        rest: vec![],
                    },
                },
                sdk::StreamRevision::try_from(2).unwrap(),
                mutation("sdk-mixed-codec-pcma"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        assert_eq!(codec.stream_revision.get(), 3);
        let sdk::ConnectionUpdateOutcome::Codecs { streams, .. } = codec.outcome else {
            panic!("mixed codec receipt")
        };
        let data = streams
            .iter()
            .find(|s| s.parameters.medium == sdk::Medium::Data)
            .unwrap();
        assert!(data.parameters.stream_local_id.is_none());
        assert_eq!(data.stream_generation, 1);
        let changed = streams
            .iter()
            .find(|s| s.parameters.id == *audio.id())
            .unwrap();
        assert_eq!(changed.parameters.codec["name"], "pcma");
        assert_eq!(changed.stream_generation, 2);
        let mut packet = vec![0x80, 8, 0, 1, 0, 0, 0, 160, 0, 0, 0, 1];
        packet.extend_from_slice(&[0xaa; 160]);
        assert!(
            matches!(audio.send_rtp(sdk::RtpPacket::new(1,packet.clone()).unwrap(),options()).await,Err(e) if e.kind==sdk::ErrorKind::Validation),
            "mute must survive codec replacement"
        );
        one.update(
            sdk::ConnectionUpdate::Unmute { streams: targets() },
            sdk::StreamRevision::try_from(3).unwrap(),
            mutation("sdk-mixed-codec-unmute"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap();
        audio
            .send_rtp(sdk::RtpPacket::new(1, packet).unwrap(), options())
            .await
            .unwrap();
        let fresh = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let restored = fresh
            .restore(saved, fresh.operation_kinds().connection_offer)
            .unwrap();
        let recovered = restored.wait(options()).await.unwrap().value;
        assert_eq!(recovered.id(), one.id());
        assert!(recovered.native_negotiation().unwrap().is_none()); // No physical ownership from a saved reference.
        let ending = one
            .end(200, "normal".into(), None, mutation("native-sdk-end-one"))
            .await
            .unwrap();
        let ended_reference = ending.reference_json();
        assert!(one.native_negotiation().unwrap().is_none());
        assert!(two.native_negotiation().unwrap().is_some());
        assert_eq!(ending.wait(options()).await.unwrap().value.id(), one.id());
        tokio::time::timeout(Duration::from_secs(5), async {
            while one.refresh(options()).await.unwrap().state != "retired" {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("end reaches distinct durable retirement");
        let replay = one
            .end(200, "normal".into(), None, mutation("native-sdk-end-one"))
            .await
            .unwrap();
        assert_eq!(replay.reference_json(), ended_reference);
        assert_eq!(replay.wait(options()).await.unwrap().value.id(), one.id());
        let changed = one
            .end(
                200,
                "changed original reason".into(),
                None,
                mutation("native-sdk-end-one"),
            )
            .await;
        assert!(
            matches!(changed,Err(e) if e.certainty==sdk::Certainty::Refused && e.operation.as_ref().unwrap().operation_id.as_str()=="native-sdk-end-one")
        );
        let restored_end = fresh
            .restore(ended_reference, fresh.operation_kinds().connection_end)
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        assert_eq!(restored_end.id(), one.id());
        assert!(restored_end.native_negotiation().unwrap().is_none());
        assert_ne!(two.refresh(options()).await.unwrap().state, "retired");
        assert_eq!(
            two.native_negotiation().unwrap().unwrap().streams[0].stream_local_id,
            b.streams[0].stream_local_id
        );
        fresh.close(options()).await.unwrap();
        client.close(options()).await.unwrap();
        assert!(one.native_negotiation().is_err());
        f.server.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn public_sdk_concurrent_mutations_and_status_keep_receipts_and_history_distinct() {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_membership_scope(algorithm, 8, true).await;
        let client = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let creating = client
            .create_conversation(
                vec![f.peer.identity_id.parse().unwrap()],
                ConversationPolicy::Persistent,
                mutation("sdk-pipelined-create"),
            )
            .await
            .unwrap();
        let room = creating.wait(options()).await.unwrap().value;
        let current = room.refresh(options()).await.unwrap();
        let recipient = current
            .participants
            .iter()
            .find(|p| p.identity_id.as_str() == f.peer.identity_id)
            .unwrap()
            .id
            .clone();
        enum WorkResult {
            Send(usize, sdk::Result<sdk::Operation<sdk::MessageAcceptance>>),
            Query(sdk::Result<sdk::OperationSnapshot>),
        }
        let mut work = tokio::task::JoinSet::new();
        // The SDK capacity is 16; this listener deliberately keeps its pending
        // capacity of 8. Applications receive explicit refusals, never hidden retries.
        for index in 0..24 {
            let room = room.clone();
            let recipient = recipient.clone();
            work.spawn(async move {
                let result = room
                    .send(
                        format!("sdk-pipelined-message-{index}").parse().unwrap(),
                        NonEmpty {
                            first: recipient,
                            rest: vec![],
                        },
                        format!("Private concurrent fixture {index}"),
                        mutation(&format!("sdk-pipelined-send-{index}")),
                    )
                    .await;
                WorkResult::Send(index, result)
            });
        }
        for _ in 0..8 {
            let creating = creating.clone();
            work.spawn(async move { WorkResult::Query(creating.status(options()).await) });
        }
        let capacity_refusal = |error: &sdk::Error| {
            assert_eq!(error.kind, sdk::ErrorKind::Application);
            assert_eq!(error.certainty, sdk::Certainty::Refused);
            assert_eq!(
                error.problem.as_ref().unwrap().code,
                429,
                "{:?}",
                error.problem
            );
        };
        let mut sends = Vec::new();
        let mut refused_queries = 0;
        while let Some(result) = work.join_next().await {
            match result.unwrap() {
                WorkResult::Send(index, result) => sends.push((index, result)),
                WorkResult::Query(Ok(snapshot)) => {
                    assert_eq!(snapshot.state, sdk::OperationState::Completed)
                }
                WorkResult::Query(Err(error)) => {
                    capacity_refusal(&error);
                    refused_queries += 1;
                }
            }
        }
        let mut refused_sends = 0;
        for (index, result) in sends {
            let id = format!("sdk-pipelined-message-{index}");
            let operation_id = format!("sdk-pipelined-send-{index}");
            let sending = match result {
                Ok(sending) => sending,
                Err(error) => {
                    capacity_refusal(&error);
                    assert_eq!(
                        error.operation.as_ref().unwrap().operation_id.as_str(),
                        operation_id
                    );
                    refused_sends += 1;
                    // Explicit application retry uses the saved ID and identical
                    // intent after all burst requests have settled.
                    room.send(
                        id.parse().unwrap(),
                        NonEmpty {
                            first: recipient.clone(),
                            rest: vec![],
                        },
                        format!("Private concurrent fixture {index}"),
                        mutation(&operation_id),
                    )
                    .await
                    .unwrap()
                }
            };
            let completion = sending.wait(options()).await.unwrap();
            assert_eq!(completion.value.message_id.as_str(), id);
            assert_eq!(
                completion.value.operation.operation_id.as_str(),
                operation_id
            );
            assert_eq!(completion.snapshot.state, sdk::OperationState::Completed);
        }
        for _ in 0..refused_queries {
            assert_eq!(
                creating.status(options()).await.unwrap().state,
                sdk::OperationState::Completed
            );
        }
        println!("SDK concurrency {algorithm}: 24 sends, 8 status reads, {refused_sends} explicit send capacity refusals, {refused_queries} explicit read capacity refusals; original-ID recovery");
        let mut inspection = CanonicalClient::open(f.client.clone()).await.unwrap();
        let history = inspection
            .history(
                room.id().as_str(),
                serde_json::from_value(json!({"limit":100})).unwrap(),
            )
            .await
            .unwrap();
        let ids: std::collections::BTreeSet<_> = history
            .messages
            .iter()
            .map(|m| m.msg_id.as_str().to_owned())
            .collect();
        assert_eq!(history.messages.len(), 24);
        assert_eq!(ids.len(), 24);
        for index in 0..24 {
            assert!(ids.contains(&format!("sdk-pipelined-message-{index}")));
        }
        inspection.shutdown().await.unwrap();
        client.close(options()).await.unwrap();
        client.close(options()).await.unwrap();
        f.server.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn public_sdk_conversation_private_send_recovery_and_owned_shutdown() {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_membership_scope(algorithm, 8, true).await;
        let client = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let creating = client
            .create_conversation(
                vec![f.peer.identity_id.parse().unwrap()],
                ConversationPolicy::Persistent,
                mutation("sdk-create"),
            )
            .await
            .unwrap();
        let saved_creation = creating.reference_json();
        assert!(saved_creation["scope"].as_object().unwrap().is_empty());
        let completed = creating.wait(options()).await.unwrap();
        assert_eq!(completed.snapshot.state, sdk::OperationState::Completed);
        let room = completed.value;
        let current = room.refresh(options()).await.unwrap();
        assert_eq!(current.participants.len(), 2);
        let recipient = current
            .participants
            .iter()
            .find(|p| p.identity_id.as_str() == f.peer.identity_id)
            .unwrap()
            .id
            .clone();
        let sending = room
            .send(
                "sdk-message".parse().unwrap(),
                NonEmpty {
                    first: recipient.clone(),
                    rest: vec![],
                },
                "Private original content".into(),
                mutation("sdk-send"),
            )
            .await
            .unwrap();
        let saved = sending.reference_json();
        let acceptance = sending.wait(options()).await.unwrap();
        assert_eq!(acceptance.value.message_id.as_str(), "sdk-message");
        assert_eq!(acceptance.value.operation.operation_id.as_str(), "sdk-send");
        let cancelled = RequestOptions {
            signal: tokio_util::sync::CancellationToken::new(),
            ..options()
        };
        cancelled.signal.cancel();
        let result = sending.wait(cancelled).await;
        assert!(
            matches!(result,Err(e) if e.kind==sdk::ErrorKind::Cancelled && e.certainty==sdk::Certainty::ReadOnly && e.operation.as_ref().unwrap().operation_id.as_str()=="sdk-send")
        );
        assert_eq!(
            sending.status(options()).await.unwrap().state,
            sdk::OperationState::Completed
        );
        // Changed content keeps the original ID and is refused, never renamed.
        let conflict = room
            .send(
                "sdk-message".parse().unwrap(),
                NonEmpty {
                    first: recipient.clone(),
                    rest: vec![],
                },
                "Changed content".into(),
                mutation("sdk-send"),
            )
            .await;
        assert!(
            matches!(conflict,Err(e) if e.certainty==sdk::Certainty::Refused && e.operation.as_ref().unwrap().operation_id.as_str()=="sdk-send")
        );
        let mut wrong = saved.clone();
        wrong["command"] = json!("conversation.create");
        assert!(client
            .restore(wrong, client.operation_kinds().message_send)
            .is_err());
        let mut bad = saved.clone();
        bad["scope"]["session_id"] = json!("forged-session");
        assert!(client
            .restore(bad, client.operation_kinds().message_send)
            .is_err());
        let mut secret = saved.clone();
        secret["credential"] = json!("must-not-be-accepted");
        assert!(client
            .restore(secret, client.operation_kinds().message_send)
            .is_err());
        client.close(options()).await.unwrap();
        assert!(
            matches!(room.refresh(options()).await,Err(e) if e.kind==sdk::ErrorKind::Cancelled)
        );
        // A fresh authenticated client only restores and queries the original IDs.
        let reconnected = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let original = reconnected
            .restore(saved, reconnected.operation_kinds().message_send)
            .unwrap();
        assert_eq!(
            original.status(options()).await.unwrap().state,
            sdk::OperationState::Completed
        );
        assert_eq!(
            original
                .wait(options())
                .await
                .unwrap()
                .value
                .message_id
                .as_str(),
            "sdk-message"
        );
        let original_create = reconnected
            .restore(
                saved_creation,
                reconnected.operation_kinds().conversation_create,
            )
            .unwrap();
        assert_eq!(
            original_create.wait(options()).await.unwrap().value.id(),
            room.id()
        );
        // The canonical public history remains authoritative: one original message.
        let mut inspection = CanonicalClient::open(f.client.clone()).await.unwrap();
        let history = inspection
            .history(
                room.id().as_str(),
                serde_json::from_value(json!({"limit":100})).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(history.messages.len(), 1);
        assert_eq!(history.messages[0].msg_id.as_str(), "sdk-message");
        inspection.shutdown().await.unwrap();
        let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
        let subject = f
            .server
            .store
            .authenticate(&f.client.credential)
            .unwrap()
            .credential_id;
        f.server
            .store
            .execute_provisioning(
                &admin,
                ProvisioningCommand::RevokeCredential {
                    operation_id: "sdk-revoke".into(),
                    credential_id: subject,
                },
            )
            .unwrap();
        let revoked = original.status(options()).await;
        assert!(
            matches!(revoked,Err(e) if e.operation.as_ref().unwrap().operation_id.as_str()=="sdk-send")
        );
        reconnected.close(options()).await.unwrap();
        reconnected.close(options()).await.unwrap();
        f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn public_sdk_participant_mutations_preserve_revision_epoch_and_receipt_scope() {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_membership_scope(algorithm, 8, true).await;
        let client = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let room = client
            .create_conversation(
                vec![f.peer.identity_id.parse().unwrap()],
                ConversationPolicy::Persistent,
                mutation("members-create"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        let seeded = room.refresh(options()).await.unwrap();
        let target = seeded
            .participants
            .iter()
            .find(|p| p.identity_id.as_str() == f.peer.identity_id)
            .unwrap();
        let kind = target.kind.clone();
        let wrong_kind_value = if kind == sdk::IdentityKind::Human {
            sdk::IdentityKind::Ai
        } else {
            sdk::IdentityKind::Human
        };
        room.participant_by_id(target.id.clone())
            .remove(seeded.revision, mutation("members-reset"))
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        let original = room.refresh(options()).await.unwrap();
        assert_eq!(original.participants.len(), 1);
        let wrong_kind = room
            .add_participant(
                f.peer.identity_id.parse().unwrap(),
                wrong_kind_value,
                "participant".into(),
                original.revision,
                mutation("members-wrong-kind"),
            )
            .await;
        assert!(matches!(wrong_kind,Err(e) if e.certainty==sdk::Certainty::Refused));
        assert_eq!(room.refresh(options()).await.unwrap().participants.len(), 1);
        let added = room
            .add_participant(
                f.peer.identity_id.parse().unwrap(),
                kind.clone(),
                "participant".into(),
                original.revision,
                mutation("members-add"),
            )
            .await
            .unwrap();
        let saved = added.reference_json();
        let member = added.wait(options()).await.unwrap().value;
        assert_eq!(member.receipt_snapshot().unwrap().kind, kind);
        assert!(member.receipt_snapshot().unwrap().active);
        assert_eq!(
            member.receipt_conversation_revision().unwrap().get(),
            original.revision.get() + 1
        );
        let epoch = member.refresh(options()).await.unwrap().membership_epoch;
        let before = room.refresh(options()).await.unwrap();
        let changed = member
            .update("observer".into(), before.revision, mutation("members-role"))
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        assert_eq!(changed.receipt_snapshot().unwrap().role, "observer");
        assert_eq!(member.refresh(options()).await.unwrap().role, "observer");
        let replay = room
            .add_participant(
                f.peer.identity_id.parse().unwrap(),
                kind.clone(),
                "participant".into(),
                original.revision,
                mutation("members-add"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        assert_eq!(replay.receipt_snapshot().unwrap().role, "observer");
        assert_eq!(replay.receipt_snapshot().unwrap().membership_epoch, epoch);
        assert_eq!(room.refresh(options()).await.unwrap().participants.len(), 2);
        let stale = member
            .update(
                "participant".into(),
                original.revision,
                mutation("members-stale"),
            )
            .await;
        assert!(matches!(stale,Err(e) if e.certainty==sdk::Certainty::Refused));
        let before = room.refresh(options()).await.unwrap();
        let removed = member
            .remove(before.revision, mutation("members-remove"))
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        assert!(!removed.receipt_snapshot().unwrap().active);
        assert!(
            matches!(member.refresh(options()).await,Err(e) if e.kind==sdk::ErrorKind::ReceiptUnavailable)
        );
        let before = room.refresh(options()).await.unwrap();
        let returned = room
            .add_participant(
                f.peer.identity_id.parse().unwrap(),
                kind.clone(),
                "participant".into(),
                before.revision,
                mutation("members-readd"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        assert_eq!(returned.id(), member.id());
        assert!(returned.receipt_snapshot().unwrap().membership_epoch > epoch);
        let mut wrong = saved.clone();
        wrong["scope"]["connection_id"] = json!("foreign");
        assert!(client
            .restore(wrong, client.operation_kinds().conversation_participant_add)
            .is_err());
        client.close(options()).await.unwrap();
        let fresh = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let restored = fresh
            .restore(saved, fresh.operation_kinds().conversation_participant_add)
            .unwrap();
        // Current authority is checked by the read. A preserved historical receipt
        // cannot silently become a fresh admission into the new membership epoch.
        let observed = restored.status(options()).await;
        assert!(matches!(observed,Err(e) if e.certainty==sdk::Certainty::Refused));
        let current = fresh
            .conversation_by_id(room.id().clone())
            .participant_by_id(member.id().clone())
            .refresh(options())
            .await
            .unwrap();
        assert!(current.membership_epoch > epoch);
        fresh.close(options()).await.unwrap();
        f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn public_sdk_session_controls_use_topology_revision_and_current_scoped_results() {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_membership_scope(algorithm, 12, true).await;
        let owner = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let peer_config = super::canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            71,
            vec!["audio".into()],
            Scope::collaboration(vec!["*".into()]),
        );
        let peer = Client::connect(peer_config, "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let room = owner
            .create_conversation(
                vec![f.peer.identity_id.parse().unwrap()],
                ConversationPolicy::Persistent,
                mutation("sessions-create"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        let view = room.refresh(options()).await.unwrap();
        let own = view
            .participants
            .iter()
            .find(|p| p.identity_id.as_str() == f.client.identity_id)
            .unwrap()
            .id
            .clone();
        let target = view
            .participants
            .iter()
            .find(|p| p.identity_id.as_str() == f.peer.identity_id)
            .unwrap()
            .id
            .clone();
        let caps = json!({"audio":["g.711-mu"],"video":["vp8"],"screen":["vp8"],"data":["reliable-ordered"]});
        let solo = room
            .invite(
                "sdk-solo".parse().unwrap(),
                vec![],
                sdk::SessionMedium::Voice,
                "sdk-solo".into(),
                caps.clone(),
                mutation("sessions-solo"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        assert_eq!(solo.refresh(options()).await.unwrap().state, "accepted");
        let initial = solo.refresh(options()).await.unwrap();
        let changed = solo
            .update(
                sdk::SessionUpdate::RoleChange {
                    participant_id: own.clone(),
                    role: "observer".into(),
                },
                initial.topology_revision,
                mutation("sessions-role"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        let current = changed.refresh(options()).await.unwrap();
        let changed = solo
            .update(
                sdk::SessionUpdate::RoleChange {
                    participant_id: own,
                    role: "participant".into(),
                },
                current.topology_revision,
                mutation("sessions-role-again"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        let current = changed.refresh(options()).await.unwrap();
        let upgraded = solo
            .update(
                sdk::SessionUpdate::MediumUpgrade(NonEmpty {
                    first: sdk::Medium::Audio,
                    rest: vec![sdk::Medium::Video],
                }),
                current.topology_revision,
                mutation("sessions-upgrade"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        let current = upgraded.refresh(options()).await.unwrap();
        assert!(current.media.contains(&sdk::Medium::Video));
        assert_ne!(current.revision.get(), current.topology_revision.get());
        // Deliberately encode the wrong family's numeric value to prove the
        // server refuses it; ordinary callers cannot pass SessionRevision here.
        let wrong = sdk::TopologyRevision::try_from(current.revision.get()).unwrap();
        assert!(
            matches!(solo.end(1000,"stale".into(),wrong,mutation("sessions-wrong-revision")).await,Err(e) if e.certainty==sdk::Certainty::Refused)
        );
        let current = solo.refresh(options()).await.unwrap();
        assert_eq!(current.state, "accepted");

        let codecs = solo
            .update(
                sdk::SessionUpdate::CodecRenegotiate(NonEmpty {
                    first: "g.711-mu".into(),
                    rest: vec!["vp8".into()],
                }),
                current.topology_revision,
                mutation("sessions-codecs"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        assert_eq!(
            codecs.receipt_snapshot().unwrap().codec_preferences,
            vec!["g.711-mu", "vp8"]
        );
        let current = solo.refresh(options()).await.unwrap();
        solo.invite_participants(
            NonEmpty {
                first: target.clone(),
                rest: vec![],
            },
            sdk::SessionMedium::VoiceVideo,
            "sdk-solo".into(),
            caps.clone(),
            current.topology_revision,
            mutation("sessions-existing-invite"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap();
        let peer_room = peer.conversation_by_id(room.id().clone());
        let existing = peer_room.session_by_id(solo.id().clone());
        let pending = existing.refresh(options()).await.unwrap();
        assert_eq!(pending.state, "accepted"); // A new invitee can join an existing Session.
        existing
            .accept(
                caps.clone(),
                pending.topology_revision,
                mutation("sessions-existing-accept"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        let current = solo.refresh(options()).await.unwrap();
        let ended = solo
            .end(
                1000,
                "complete".into(),
                current.topology_revision,
                mutation("sessions-end"),
            )
            .await
            .unwrap();
        let saved = ended.reference_json();
        assert_eq!(
            ended
                .wait(options())
                .await
                .unwrap()
                .value
                .receipt_snapshot()
                .unwrap()
                .state,
            "ended"
        );
        assert!(saved["scope"].get("connection_id").is_none());
        let invite = room
            .invite(
                "sdk-invited".parse().unwrap(),
                vec![target.clone()],
                sdk::SessionMedium::Voice,
                "sdk-invited".into(),
                caps.clone(),
                mutation("sessions-invite"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        let peer_room = peer.conversation_by_id(room.id().clone());
        let incoming = peer_room.session_by_id(invite.id().clone());
        let current = incoming.refresh(options()).await.unwrap();
        let accepting = incoming
            .accept(
                caps.clone(),
                current.topology_revision,
                mutation("sessions-accept"),
            )
            .await
            .unwrap();
        assert_eq!(
            accepting
                .wait(options())
                .await
                .unwrap()
                .value
                .receipt_snapshot()
                .unwrap()
                .state,
            "accepted"
        );
        let current = invite.refresh(options()).await.unwrap();
        invite
            .update(
                sdk::SessionUpdate::ParticipantLeave(target.clone()),
                current.topology_revision,
                mutation("sessions-leave"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        let mut inspection = CanonicalClient::open(f.client.clone()).await.unwrap();
        let topology = inspection
            .conversation_snapshot(
                room.id().as_str(),
                serde_json::from_value(json!({"limit":100})).unwrap(),
            )
            .await
            .unwrap();
        assert!(topology.items.iter().any(|v|matches!(serde_json::from_value::<connect_protocol::events::SnapshotItem>(v.clone()),Ok(connect_protocol::events::SnapshotItem::SessionParticipant(p)) if p.session_id==invite.id().as_str() && p.participant_id==target.as_str() && p.state=="left")));
        inspection.shutdown().await.unwrap();
        let current = invite.refresh(options()).await.unwrap();

        invite
            .end(
                1000,
                "done".into(),
                current.topology_revision,
                mutation("sessions-group-end"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        let reject = room
            .invite(
                "sdk-reject".parse().unwrap(),
                vec![target.clone()],
                sdk::SessionMedium::Voice,
                "sdk-reject".into(),
                caps.clone(),
                mutation("sessions-reject-invite"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        let incoming = peer_room.session_by_id(reject.id().clone());
        let current = incoming.refresh(options()).await.unwrap();
        let rejected = incoming
            .reject(
                486,
                "Unavailable".into(),
                current.topology_revision,
                mutation("sessions-reject"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        assert_eq!(rejected.value.state, "rejected");
        assert_eq!(rejected.value.reason_code, 486);
        let cancel = room
            .invite(
                "sdk-cancel".parse().unwrap(),
                vec![target],
                sdk::SessionMedium::Voice,
                "sdk-cancel".into(),
                caps,
                mutation("sessions-cancel-invite"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        let current = cancel.refresh(options()).await.unwrap();
        let cancelled = cancel
            .cancel_invitation(
                487,
                "Cancelled".into(),
                current.topology_revision,
                mutation("sessions-cancel"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        assert_eq!(cancelled.value.state, "cancelled");
        peer.close(options()).await.unwrap();
        owner.close(options()).await.unwrap();
        let fresh = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let original = fresh
            .restore(saved, fresh.operation_kinds().session_end)
            .unwrap();
        assert_eq!(
            original.status(options()).await.unwrap().state,
            sdk::OperationState::Completed
        );
        assert_eq!(
            original.wait(options()).await.unwrap().value.id(),
            solo.id()
        );
        fresh.close(options()).await.unwrap();
        f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn public_sdk_durable_decisions_keep_target_identity_and_require_real_evidence() {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_extra_scopes(
            algorithm,
            8,
            true,
            false,
            false,
            &["operation.cancel", "operation.reconcile"],
        )
        .await;
        let client = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let creation = client
            .create_conversation(
                vec![],
                ConversationPolicy::Persistent,
                mutation("decisions-create"),
            )
            .await
            .unwrap();
        let room = creation.wait(options()).await.unwrap().value;
        let unscoped = creation
            .cancel(mutation("decision-create-cancel"))
            .await
            .unwrap();
        let unscoped_saved = unscoped.reference_json();
        assert!(unscoped_saved["scope"].as_object().unwrap().is_empty());
        assert_eq!(
            unscoped
                .wait(options())
                .await
                .unwrap()
                .value
                .target
                .as_str(),
            "decisions-create"
        );
        let solo = room
            .invite(
                "decision-session".parse().unwrap(),
                vec![],
                sdk::SessionMedium::Voice,
                "decision-session".into(),
                json!({"audio":["g.711-mu"]}),
                mutation("decision-session-create"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        let current = solo.refresh(options()).await.unwrap();
        let ending = solo
            .end(
                1000,
                "complete".into(),
                current.topology_revision,
                mutation("decision-session-end"),
            )
            .await
            .unwrap();
        ending.wait(options()).await.unwrap();
        let scoped = ending
            .cancel(mutation("decision-session-cancel"))
            .await
            .unwrap();
        let scoped_saved = scoped.reference_json();
        assert_eq!(scoped_saved["scope"]["session_id"], "decision-session");
        assert!(scoped_saved["scope"].get("connection_id").is_none());
        assert_eq!(
            scoped.wait(options()).await.unwrap().value.disposition,
            "already_terminal"
        );

        let own = room.refresh(options()).await.unwrap().participants[0]
            .id
            .clone();
        let sending = room
            .send(
                "decision-message".parse().unwrap(),
                NonEmpty {
                    first: own.clone(),
                    rest: vec![],
                },
                "Original completed mutation".into(),
                mutation("decision-send"),
            )
            .await
            .unwrap();
        sending.wait(options()).await.unwrap();
        let cancelled = RequestOptions {
            signal: sdk::CancellationToken::new(),
            ..options()
        };
        cancelled.signal.cancel();
        assert!(
            matches!(sending.wait(cancelled).await,Err(e) if e.certainty==sdk::Certainty::ReadOnly)
        );
        let same = sending.cancel(mutation("decision-send")).await;
        assert!(
            matches!(same,Err(e) if e.kind==sdk::ErrorKind::Validation && e.certainty==sdk::Certainty::NotSubmitted)
        );
        let mut extended = mutation("decision-extension");
        extended.extensions = Some(std::collections::BTreeMap::from([(
            "future".into(),
            json!(true),
        )]));
        assert!(
            matches!(sending.cancel(extended).await,Err(e) if e.kind==sdk::ErrorKind::Unsupported && e.certainty==sdk::Certainty::NotSubmitted)
        );
        let decision = sending.cancel(mutation("decision-cancel")).await.unwrap();
        let saved = decision.reference_json();
        assert_eq!(saved["operation_id"], "decision-cancel");
        assert!(saved["scope"].get("connection_id").is_none());
        let receipt = decision.wait(options()).await.unwrap();
        assert_eq!(receipt.value.target.as_str(), "decision-send");
        assert_eq!(receipt.value.observed_state, sdk::OperationState::Completed);
        assert_eq!(receipt.value.disposition, "already_terminal");
        assert_eq!(
            sending.status(options()).await.unwrap().state,
            sdk::OperationState::Completed
        );
        let replay = sending
            .cancel(mutation("decision-cancel"))
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        assert_eq!(
            replay.value.decision.operation_id,
            receipt.value.decision.operation_id
        );
        let other = room
            .send(
                "other-decision-message".parse().unwrap(),
                NonEmpty {
                    first: own,
                    rest: vec![],
                },
                "Second distinct original intent".into(),
                mutation("other-decision-send"),
            )
            .await
            .unwrap();
        other.wait(options()).await.unwrap();
        assert!(
            matches!(other.cancel(mutation("decision-cancel")).await,Err(e) if e.certainty==sdk::Certainty::Refused)
        );
        assert!(
            matches!(decision.cancel(mutation("decision-of-decision")).await,Err(e) if e.certainty==sdk::Certainty::Refused)
        );
        assert!(
            matches!(sending.reconcile(sdk::ReconcileOutcome::Completed,"forged-evidence".into(),mutation("decision-forged")).await,Err(e) if e.certainty==sdk::Certainty::Refused)
        );
        assert!(
            matches!(sending.reconcile(sdk::ReconcileOutcome::Failed,"forged-evidence".into(),mutation("decision-false-failure")).await,Err(e) if e.certainty==sdk::Certainty::Refused)
        );
        client.close(options()).await.unwrap();
        let fresh = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        assert_eq!(
            fresh
                .restore(unscoped_saved, fresh.operation_kinds().operation_cancel)
                .unwrap()
                .wait(options())
                .await
                .unwrap()
                .value
                .target
                .as_str(),
            "decisions-create"
        );
        assert_eq!(
            fresh
                .restore(scoped_saved, fresh.operation_kinds().operation_cancel)
                .unwrap()
                .wait(options())
                .await
                .unwrap()
                .value
                .target
                .as_str(),
            "decision-session-end"
        );
        let original = fresh
            .restore(saved, fresh.operation_kinds().operation_cancel)
            .unwrap();
        let observed = original.status(options()).await.unwrap();
        assert_eq!(observed.state, sdk::OperationState::Completed);
        let recovered = original.wait(options()).await.unwrap();
        assert_eq!(recovered.value.target.as_str(), "decision-send");
        assert_eq!(recovered.value.disposition, "already_terminal");
        fresh.close(options()).await.unwrap();
        f.server.shutdown().await.unwrap();
    }
}
