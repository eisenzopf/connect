//! Controlled Store origins only; no physical provider/media claim.
use super::*;
use std::sync::Arc;
mod observation_sources;
mod phases;
fn setup(with_context: bool) -> (Fixture, Arc<crate::NativeMediaOrigin>, String, usize) {
    let mut f = Fixture::with_identity_kinds(
        vec!["audio".into()],
        [IdentityKind::Ai, IdentityKind::Ai, IdentityKind::System],
    );
    let first: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT participant_id FROM sessions WHERE id='session'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let target_actor = if f.store.revalidate(&f.keys[1]).unwrap().identity_id != first {
        1
    } else {
        2
    };
    let identity = f
        .store
        .revalidate(&f.keys[target_actor])
        .unwrap()
        .identity_id;
    if !with_context {
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope.operations.retain(|op| op != "context_view");
        f.keys[target_actor] = f
            .store
            .issue(&identity, scope, crate::now() + 600)
            .unwrap()
            .id;
    }
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::Endpoint {
                operation_id: "assistant-context-endpoint".into(),
                identity_id: identity,
                device: DeviceSpec {
                    id: "assistant-context-provider".into(),
                    kind: "service".into(),
                    platform: "controlled-store".into(),
                },
                transport: EndpointTransport::VapiVoice,
                address: "assistant-context-fixture".into(),
                capabilities: vec!["audio".into()],
                secret_ref: Some("fixture_context_secret".into()),
            },
        )
        .unwrap()
    else {
        panic!("provider endpoint")
    };
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: "verify-assistant-context-endpoint".into(),
                endpoint_id: endpoint_id.clone(),
                expected_revision: 0,
                evidence_ref: "controlled-store".into(),
            },
        )
        .unwrap();
    let origin = consultation_reads::ready(&f, target_actor);
    (f, origin, endpoint_id, target_actor)
}
#[test]
fn native_assistant_context_uses_exact_ai_incarnation_and_every_current_listener() {
    use connect_protocol::messages::*;
    let (f, target, endpoint, target_actor) = setup(true);
    f.store
        .execute(
            &f.keys[0],
            &Operation::Send {
                request_id: "public-assistant-context".into(),
                conversation_id: f.cid.clone(),
                message: MessageDraft::text_all("PUBLIC-CONTEXT-FACT"),
            },
        )
        .unwrap();
    f.store
        .execute(
            &f.keys[0],
            &Operation::Send {
                request_id: "narrow-assistant-context".into(),
                conversation_id: f.cid.clone(),
                message: MessageDraft::text(
                    MessageAudience::Participants(vec![
                        f.pids[0].clone(),
                        f.pids[target_actor].clone(),
                    ]),
                    "PAIR-ONLY-SECRET",
                ),
            },
        )
        .unwrap();
    let read = f
        .store
        .try_native_assistant_context_read(&target, &f.keys[0], &endpoint)
        .unwrap();
    assert!(read.current());
    assert_eq!(
        read.target().connection.participant_id,
        f.pids[target_actor]
    );
    assert_eq!(read.audience().len(), 4);
    let first: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT participant_id FROM sessions WHERE id='session'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let ai = f
        .store
        .revalidate(&f.keys[target_actor])
        .unwrap()
        .identity_id;
    assert_ne!(first, ai, "the AI is deliberately not the first invitee");
    let view: connect_protocol::ContextView =
        serde_json::from_str(read.context_json().unwrap()).unwrap();
    assert!(view.items.iter().any(|v| v.body == "PUBLIC-CONTEXT-FACT"));
    assert!(!read.context_json().unwrap().contains("PAIR-ONLY-SECRET"));
    assert_eq!(
        view.omitted, 0,
        "hidden pair history must not become a public omission count"
    );
    assert_eq!(read.provider_endpoint().unwrap()["id"], endpoint);
    assert_eq!(format!("{read:?}"), "NativeAssistantContextRead(redacted)");
}
#[test]
fn native_assistant_context_never_borrows_the_context_owners_target_permission() {
    let (f, target, endpoint, _target_actor) = setup(false);
    assert_eq!(
        f.store
            .try_native_assistant_context_read(&target, &f.keys[0], &endpoint)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let (f, target, endpoint, target_actor) = setup(true);
    let narrow = f
        .store
        .issue(
            &f.store.revalidate(&f.keys[0]).unwrap().identity_id,
            Scope::collaboration(vec![
                f.store
                    .revalidate(&f.keys[target_actor])
                    .unwrap()
                    .identity_id,
            ]),
            crate::now() + 600,
        )
        .unwrap();
    assert!(f
        .store
        .try_native_assistant_context_read(&target, &narrow.id, &endpoint)
        .is_err());
    assert!(
        f.store
            .try_native_assistant_context_read(&target, &f.keys[0], &f.endpoints[target_actor])
            .is_err(),
        "native endpoint is not a provider destination"
    );
    let human = consultation_reads::ready(&f, 0);
    assert_eq!(
        f.store
            .try_native_assistant_context_read(&human, &f.keys[0], &endpoint)
            .unwrap_err()
            .code,
        "unsupported-media-profile"
    );
}
#[test]
fn native_assistant_context_holds_one_complete_interval_behind_queued_revocation() {
    let (f, target, endpoint, target_actor) = setup(true);
    let read = f
        .store
        .try_native_assistant_context_read(&target, &f.keys[0], &endpoint)
        .unwrap();
    let body = read.context_json().unwrap().to_owned();
    let ai = f
        .store
        .revalidate(&f.keys[target_actor])
        .unwrap()
        .identity_id;
    let write = crate::AuthorityScope::identity(&ai, crate::AuthorityMode::Write).unwrap();
    let admin = f.store.revalidate(&f.admin).unwrap();
    let key = f.keys[target_actor].clone();
    std::thread::scope(|threads| {
        let writer = threads.spawn(|| f.store.revoke(&admin, &key));
        crate::authority_fences::test_wait_for_writer(&f.store, &write);
        assert_eq!(read.context_json().unwrap(), body);
        assert!(read.current());
        assert_eq!(read.provider_endpoint().unwrap()["id"], endpoint);
        drop(read);
        writer.join().unwrap().unwrap();
    });
    assert!(f
        .store
        .try_native_assistant_context_read(&target, &f.keys[0], &endpoint)
        .is_err());
}

fn reserve(
    f: &Fixture,
    target: &crate::NativeMediaOrigin,
    endpoint: &str,
    id: &str,
) -> crate::AssistantProviderContext {
    let read = f
        .store
        .try_native_assistant_context_read(target, &f.keys[0], endpoint)
        .unwrap();
    f.store.prepare_native_assistant_provider(id, read).unwrap()
}
#[test]
fn assistant_provider_preserves_one_creation_original_selection_and_exact_ack() {
    let (f, target, endpoint, _) = setup(true);
    f.store
        .execute(
            &f.keys[0],
            &Operation::Send {
                request_id: "public-before-provider".into(),
                conversation_id: f.cid.clone(),
                message: connect_protocol::messages::MessageDraft::text_all(
                    "ORIGINAL-PUBLIC-HISTORY",
                ),
            },
        )
        .unwrap();
    let context = reserve(&f, &target, &endpoint, "provider-lifecycle");
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let replay = reserve(&f, &target, &endpoint, "provider-lifecycle");
    assert_eq!(replay.delivery_id(), context.delivery_id());
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let read = f
        .store
        .try_native_assistant_provider_read(&context)
        .unwrap();
    assert_eq!(read.context_json().unwrap_err().code, "not-ready");
    let (attempt, read) = f
        .store
        .begin_native_assistant_provider(context.clone(), read)
        .unwrap();
    drop(read);
    assert!(f
        .store
        .try_native_assistant_provider_read(&context)
        .is_err());
    assert_eq!(
        attempt
            .created("https://provider.invalid/private?token=fixture".into())
            .unwrap_err()
            .code,
        "invalid-request"
    );
    assert_eq!(
        attempt.created("bad\nreference".into()).unwrap_err().code,
        "invalid-request"
    );
    let created = attempt.created("public-provider-one".into()).unwrap();
    f.store
        .record_native_assistant_provider_created(&created)
        .unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    f.store
        .record_native_assistant_provider_created(&created)
        .unwrap();
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    f.store
        .execute(
            &f.keys[0],
            &Operation::Send {
                request_id: "public-after-provider".into(),
                conversation_id: f.cid.clone(),
                message: connect_protocol::messages::MessageDraft::text_all("NEWER-PUBLIC-HISTORY"),
            },
        )
        .unwrap();
    let read = f
        .store
        .try_native_assistant_provider_read(&context)
        .unwrap();
    let json = read.context_json().unwrap().to_owned();
    assert!(json.contains("ORIGINAL-PUBLIC-HISTORY"));
    assert!(!json.contains("NEWER-PUBLIC-HISTORY"));
    assert_eq!(read.provider_ref().unwrap(), "public-provider-one");
    let (export, read) = f
        .store
        .begin_native_assistant_export(context.clone(), read)
        .unwrap();
    assert_eq!(
        export.acknowledged("truncated").unwrap_err().code,
        "context-ack-mismatch"
    );
    let applied = export.acknowledged(&json).unwrap();
    drop(read);
    assert!(f
        .store
        .finish_context_delivery(
            context.delivery_id(),
            "delivered",
            Some("borrowed-instance")
        )
        .is_err());
    assert!(f
        .store
        .finish_context_delivery(context.delivery_id(), "failed", None)
        .is_err());
    f.store.record_native_assistant_delivered(&applied).unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    f.store.record_native_assistant_delivered(&applied).unwrap();
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    assert!(f
        .store
        .try_native_assistant_provider_read(&context)
        .is_err());
    let memory = f
        .store
        .try_native_assistant_memory_read(&context, &applied)
        .unwrap();
    assert_eq!(memory.context_json().unwrap(), json);
    drop(memory);
    let end = f.store.begin_native_assistant_end(&created).unwrap();
    assert!(f.store.begin_native_assistant_end(&created).is_err());
    assert!(f
        .store
        .try_native_assistant_memory_read(&context, &applied)
        .is_err());
    f.store.native_assistant_end_uncertain(&end).unwrap();
    assert!(f.store.begin_native_assistant_end(&created).is_err());
    let closed = end.closed();
    f.store.record_native_assistant_closed(&closed).unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    f.store.record_native_assistant_closed(&closed).unwrap();
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let original_generation = target.generation();
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE connections SET generation=generation+1,state='retired' WHERE id=?1",
            [target.connection_id()],
        )
        .unwrap();
    let Outcome::ContextDeliveries(page) = f
        .store
        .execute(
            &f.keys[0],
            &Operation::ContextDeliveries {
                conversation_id: f.cid.clone(),
                before: None,
                limit: 100,
            },
        )
        .unwrap()
    else {
        panic!("receipts")
    };
    let receipt = page
        .deliveries
        .iter()
        .find(|d| d.id == context.delivery_id())
        .unwrap();
    let destination = receipt.destination.as_ref().unwrap();
    assert_eq!(destination.connection_id, target.connection_id());
    assert_eq!(destination.generation, original_generation);
    assert_eq!(
        destination.purpose,
        connect_protocol::ContextDeliveryPurpose::PublicSession
    );
    let lifecycle = receipt.lifecycle.as_ref().unwrap();
    assert!(lifecycle.creation_confirmed && lifecycle.delivery_confirmed && lifecycle.closed);
    assert_eq!(lifecycle.cleanup_admitted, Some(true));
    assert_eq!(lifecycle.cleanup_uncertain, Some(true));
}
#[test]
fn assistant_provider_captured_observation_survives_revocation_but_never_authorizes_more_io() {
    let (f, target, endpoint, _) = setup(true);
    let context = reserve(&f, &target, &endpoint, "revoked-provider");
    let read = f
        .store
        .try_native_assistant_provider_read(&context)
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_native_assistant_provider(context.clone(), read)
        .unwrap();
    drop(read);
    let created = attempt.created("created-before-revoke".into()).unwrap();
    let actor = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&actor, &f.keys[0]).unwrap();
    f.store
        .record_native_assistant_provider_created(&created)
        .unwrap();
    assert!(f
        .store
        .try_native_assistant_provider_read(&context)
        .is_err());
    let end = f.store.begin_native_assistant_end(&created).unwrap();
    f.store
        .record_native_assistant_closed(&end.closed())
        .unwrap();
}
#[test]
fn assistant_provider_failure_observations_are_visible_and_cannot_be_overwritten_by_an_ack() {
    use crate::AssistantContextFailure::*;
    for (i, problem) in [Rejected, Truncated, Incomplete].into_iter().enumerate() {
        let (f, target, endpoint, _) = setup(true);
        let context = reserve(&f, &target, &endpoint, "rejected-provider");
        let read = f
            .store
            .try_native_assistant_provider_read(&context)
            .unwrap();
        let (attempt, read) = f
            .store
            .begin_native_assistant_provider(context.clone(), read)
            .unwrap();
        drop(read);
        let created = attempt.created(format!("rejected-call-{i}")).unwrap();
        f.store
            .record_native_assistant_provider_created(&created)
            .unwrap();
        let read = f
            .store
            .try_native_assistant_provider_read(&context)
            .unwrap();
        let json = read.context_json().unwrap().to_owned();
        let (export, read) = f
            .store
            .begin_native_assistant_export(context.clone(), read)
            .unwrap();
        drop(read);
        let ack = export.acknowledged(&json).unwrap();
        let rejected = export.rejected(problem);
        f.store.record_native_assistant_rejected(&rejected).unwrap();
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        f.store.record_native_assistant_rejected(&rejected).unwrap();
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
        assert!(f.store.record_native_assistant_delivered(&ack).is_err());
        assert!(f
            .store
            .try_native_assistant_provider_read(&context)
            .is_err());
        let c = f.store.connection().unwrap();
        let (state,reason,events):(String,String,u64)=c.query_row("SELECT d.state,p.problem,(SELECT COUNT(*) FROM visible_journal j WHERE j.recipient=d.owner_id AND j.kind='context_delivery' AND j.source_id=d.id AND json_extract(j.data_json,'$.problem')=p.problem) FROM native_assistant_providers p JOIN context_deliveries d ON d.id=p.delivery_id WHERE p.delivery_id=?1",[context.delivery_id()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(state, "failed");
        assert!(["rejected", "truncated", "incomplete"].contains(&reason.as_str()));
        assert_eq!(events, 1);
    }
}
#[test]
fn assistant_provider_recovery_never_repeats_possible_effects_or_changes_logical_identity() {
    for started in [false, true] {
        let (f, target, endpoint, _) = setup(true);
        let context = reserve(&f, &target, &endpoint, "recovered-provider");
        if started {
            let read = f
                .store
                .try_native_assistant_provider_read(&context)
                .unwrap();
            let (_attempt, read) = f
                .store
                .begin_native_assistant_provider(context.clone(), read)
                .unwrap();
            drop(read);
        }
        f.store.connection().unwrap().execute("UPDATE native_assistant_providers SET creation_expires_at=CASE WHEN creation_started=1 THEN 1 ELSE NULL END WHERE delivery_id=?1",[context.delivery_id()]).unwrap();
        if !started {
            // Controlled clock boundary; original request identity remains immutable.
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
        assert_eq!(f.store.recover_native_assistant_providers(10).unwrap(), 1);
        assert_eq!(f.store.recover_native_assistant_providers(10).unwrap(), 0);
        let read = f
            .store
            .try_native_assistant_context_read(&target, &f.keys[0], &endpoint)
            .unwrap();
        let replay = f
            .store
            .prepare_native_assistant_provider("recovered-provider", read)
            .unwrap();
        assert_eq!(replay.delivery_id(), context.delivery_id());
        assert!(f.store.try_native_assistant_provider_read(&replay).is_err());
        let key = f
            .store
            .issue(
                &f.store.revalidate(&f.keys[0]).unwrap().identity_id,
                Scope::collaboration(vec!["*".into()]),
                crate::now() + 600,
            )
            .unwrap();
        let read = f
            .store
            .try_native_assistant_context_read(&target, &key.id, &endpoint)
            .unwrap();
        assert_eq!(
            f.store
                .prepare_native_assistant_provider("recovered-provider", read)
                .unwrap_err()
                .code,
            "conflict"
        );
        let state: String = f
            .store
            .connection()
            .unwrap()
            .query_row(
                "SELECT state FROM context_deliveries WHERE id=?1",
                [context.delivery_id()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, if started { "uncertain" } else { "failed" });
    }
}

#[test]
fn assistant_provider_cannot_adopt_old_pair_memory_even_when_its_new_selection_is_public() {
    let (f, target, endpoint, _) = setup(true);
    let first: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT participant_id FROM sessions WHERE id='session'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let first_actor = f
        .keys
        .iter()
        .position(|key| f.store.revalidate(key).unwrap().identity_id == first)
        .unwrap();
    f.store
        .execute(
            &f.keys[0],
            &Operation::Send {
                request_id: "old-pair-history".into(),
                conversation_id: f.cid.clone(),
                message: connect_protocol::messages::MessageDraft::text(
                    connect_protocol::messages::MessageAudience::Participants(vec![f.pids
                        [first_actor]
                        .clone()]),
                    "OLD-PAIR-ONLY-MODEL-MEMORY",
                ),
            },
        )
        .unwrap();
    let old = f.store.context_for_session(&f.keys[0], "session").unwrap();
    assert!(serde_json::to_string(&old)
        .unwrap()
        .contains("OLD-PAIR-ONLY-MODEL-MEMORY"));
    let historical = f
        .store
        .begin_context_delivery(&f.keys[0], "session", "vapi", &old)
        .unwrap();
    f.store
        .finish_context_delivery(&historical, "delivered", Some("old-pair-provider-instance"))
        .unwrap();
    let selected = f
        .store
        .try_native_assistant_context_read(&target, &f.keys[0], &endpoint)
        .unwrap();
    assert!(!selected
        .context_json()
        .unwrap()
        .contains("OLD-PAIR-ONLY-MODEL-MEMORY"));
    let context = f
        .store
        .prepare_native_assistant_provider("fresh-public-provider", selected)
        .unwrap();
    let read = f
        .store
        .try_native_assistant_provider_read(&context)
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_native_assistant_provider(context.clone(), read)
        .unwrap();
    drop(read);
    let reused = attempt
        .created("old-pair-provider-instance".into())
        .unwrap();
    assert_eq!(
        f.store
            .record_native_assistant_provider_created(&reused)
            .unwrap_err()
            .code,
        "conflict"
    );
    assert!(f
        .store
        .try_native_assistant_provider_read(&context)
        .is_err());
    assert!(
        f.store.begin_native_assistant_end(&reused).is_err(),
        "a refused foreign provider reference authorized cleanup"
    );
    f.store
        .native_assistant_creation_uncertain(&attempt)
        .unwrap();
}

fn routed_setup() -> (
    Fixture,
    Arc<crate::NativeMediaOrigin>,
    String,
    usize,
    crate::VoiceRoute,
    crate::DiscoveryRuntime,
) {
    use connect_protocol::routing::RouteMedium;
    let (f, target, _old_endpoint, actor) = setup(true);
    let assistant = "12345678-1234-1234-1234-123456789012";
    let identity = f.store.revalidate(&f.keys[actor]).unwrap().identity_id;
    let ProvisioningOutcome::Endpoint {
        endpoint_id: endpoint,
        ..
    } = f
        .store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::Endpoint {
                operation_id: "routed-assistant-endpoint".into(),
                identity_id: identity,
                device: DeviceSpec {
                    id: "routed-assistant-provider".into(),
                    kind: "service".into(),
                    platform: "controlled-store".into(),
                },
                transport: EndpointTransport::VapiVoice,
                address: assistant.into(),
                capabilities: vec!["audio".into()],
                secret_ref: Some("fixture_context_secret".into()),
            },
        )
        .unwrap()
    else {
        panic!("routed provider endpoint")
    };
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: "verify-routed-assistant-endpoint".into(),
                endpoint_id: endpoint.clone(),
                expected_revision: 0,
                evidence_ref: "controlled-store".into(),
            },
        )
        .unwrap();
    let route = crate::VoiceRoute {
        id: "routed-assistant".into(),
        conversation_id: f.cid.clone(),
        identity_id: f.store.revalidate(&f.keys[actor]).unwrap().identity_id,
        assistant_id: assistant.into(),
        phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
        destinations: vec!["+15555550100".into()],
        enabled: true,
    };
    f.store.configure_voice_route(&f.admin, &route).unwrap();
    let runtime = crate::DiscoveryRuntime {
        endpoints: crate::EndpointRuntime::new(vec![crate::EndpointProfile {
            transport: EndpointTransport::VapiVoice,
            media: vec![RouteMedium::Audio],
        }])
        .unwrap()
        .with_secret_refs(vec![(
            EndpointTransport::VapiVoice,
            "fixture_context_secret".into(),
        )])
        .unwrap(),
        ..Default::default()
    };
    (f, target, endpoint, actor, route, runtime)
}
#[test]
fn routed_assistant_requires_own_identity_installed_profile_and_explicit_unknown_policy() {
    use connect_protocol::routing::ReachabilityPolicy::*;
    let (f, target, endpoint, actor, _, runtime) = routed_setup();
    assert_eq!(
        f.store
            .try_routed_native_assistant_context_read(
                &target,
                &f.keys[actor],
                &endpoint,
                &runtime,
                RequireFresh
            )
            .unwrap_err()
            .code,
        "endpoint-unavailable"
    );
    assert_eq!(
        f.store
            .try_routed_native_assistant_context_read(
                &target,
                &f.keys[actor],
                &endpoint,
                &crate::DiscoveryRuntime::default(),
                AllowUnknown
            )
            .unwrap_err()
            .code,
        "endpoint-unavailable"
    );
    assert_eq!(
        f.store
            .try_routed_native_assistant_context_read(
                &target,
                &f.keys[0],
                &endpoint,
                &runtime,
                AllowUnknown
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    let read = f
        .store
        .try_routed_native_assistant_context_read(
            &target,
            &f.keys[actor],
            &endpoint,
            &runtime,
            AllowUnknown,
        )
        .unwrap();
    assert!(read.current());
}
#[test]
fn routed_assistant_creation_rechecks_route_after_preparation_before_possible_provider_io() {
    use connect_protocol::routing::ReachabilityPolicy::AllowUnknown;
    let (f, target, endpoint, actor, mut route, runtime) = routed_setup();
    let read = f
        .store
        .try_routed_native_assistant_context_read(
            &target,
            &f.keys[actor],
            &endpoint,
            &runtime,
            AllowUnknown,
        )
        .unwrap();
    let context = f
        .store
        .prepare_native_assistant_provider("routed-provider", read)
        .unwrap();
    route.enabled = false;
    f.store.configure_voice_route(&f.admin, &route).unwrap();
    assert_eq!(
        f.store
            .try_native_assistant_provider_read(&context)
            .err()
            .unwrap()
            .code,
        "endpoint-unavailable"
    );
    let started: bool = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT creation_started FROM native_assistant_providers WHERE delivery_id=?1",
            [context.delivery_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!started, "a stale route cannot become a provider attempt");
    route.enabled = true;
    f.store.configure_voice_route(&f.admin, &route).unwrap();
    let read = f
        .store
        .try_native_assistant_provider_read(&context)
        .unwrap();
    assert!(read.current());
}
