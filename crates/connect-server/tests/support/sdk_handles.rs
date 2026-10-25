//! Actual production listener exercises public owned SDK handles and recovery.
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
