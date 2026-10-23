#![allow(clippy::unwrap_used)]
use connect_protocol::{BrowserBinding, Limits, Operation, Outcome};
use connect_store::{BrowserRoute, DiscoveryRuntime, Scope, Store};
use std::sync::Arc;
struct Fixture {
    _dir: tempfile::TempDir,
    store: Arc<Store>,
    a: String,
    b: String,
    ak: String,
    bk: String,
    session: String,
    conversation: String,
}
fn fixture(video: bool) -> Fixture {
    fixture_with_acceptance(video, true)
}
fn fixture_with_acceptance(video: bool, accepted: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap());
    let a = store.create_identity("alice").unwrap().id;
    let b = store.create_identity("bob").unwrap().id;
    let expiry = chrono::Utc::now().timestamp() + 600;
    let ak = store
        .issue(&a, Scope::communications(vec![b.clone()]), expiry)
        .unwrap()
        .id;
    let bk = store
        .issue(&b, Scope::communications(vec![a.clone()]), expiry)
        .unwrap()
        .id;
    let Outcome::Conversation(c) = store
        .execute(
            &ak,
            &Operation::Create {
                request_id: "create".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(
                    vec![b.clone()],
                ),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let Outcome::Session(s) = store
        .execute(
            &ak,
            &Operation::SessionInvite {
                request_id: "invite".into(),
                conversation_id: c.id.clone(),
                participant_id: b.clone(),
                media: if video {
                    vec!["video".into(), "audio".into()]
                } else {
                    vec!["audio".into()]
                },
            },
        )
        .unwrap()
    else {
        panic!()
    };
    if accepted {
        store
            .execute(
                &bk,
                &Operation::SessionAccept {
                    request_id: "accept".into(),
                    conversation_id: c.id.clone(),
                    session_id: s.id.clone(),
                },
            )
            .unwrap();
    }
    Fixture {
        _dir: dir,
        store,
        a,
        b,
        ak,
        bk,
        session: s.id,
        conversation: c.id,
    }
}
#[test]
fn video_bindings_are_atomic_unique_idempotent_and_reusable_after_disconnect() {
    let f = fixture(true);
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let tasks: Vec<_> = (0..2)
        .map(|n| {
            let store = f.store.clone();
            let key = f.ak.clone();
            let session = f.session.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.bind_session_connection(&key, &session, &format!("connection-{n}"))
            })
        })
        .collect();
    barrier.wait();
    let results: Vec<_> = tasks.into_iter().map(|task| task.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .next()
            .unwrap()
            .code,
        "video-participant-already-connected"
    );
    let n = results.iter().position(|result| result.is_ok()).unwrap();
    let connection = format!("connection-{n}");
    f.store
        .bind_session_connection(&f.ak, &f.session, &connection)
        .unwrap();
    f.store
        .bind_session_connection(&f.bk, &f.session, "peer")
        .unwrap();
    f.store.unbind_session_connection(&connection).unwrap();
    f.store
        .bind_session_connection(&f.ak, &f.session, "reconnected")
        .unwrap();
}
#[test]
fn browser_video_discovery_authorizes_both_session_participants_and_retains_audio_default() {
    for video in [false, true] {
        let f = fixture(video);
        let runtime = DiscoveryRuntime {
            native_audio: true,
            native_video: true,
            browser: Some(BrowserRoute {
                credential_id: f.ak.clone(),
                binding: BrowserBinding {
                    conversation_id: f.conversation.clone(),
                    participant_id: f.b.clone(),
                    session_id: Some(f.session.clone()),
                    signaling_url: "ws://127.0.0.1:8181".into(),
                },
            }),
            ..Default::default()
        };
        let Outcome::BrowserBinding(left) = f
            .store
            .execute_with_runtime(
                &f.ak,
                &Operation::BrowserBinding {
                    conversation_id: None,
                    session_id: None,
                },
                &runtime,
            )
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(left.participant_id, f.b);
        let right = f.store.execute_with_runtime(
            &f.bk,
            &Operation::BrowserBinding {
                conversation_id: None,
                session_id: None,
            },
            &runtime,
        );
        if video {
            let Outcome::BrowserBinding(right) = right.unwrap() else {
                panic!()
            };
            assert_eq!(right.participant_id, f.a);
            let Outcome::Capabilities(c) = f
                .store
                .execute_with_runtime(
                    &f.bk,
                    &Operation::Capabilities {
                        address: "connect:alice@local".into(),
                        conversation_id: Some(f.conversation.clone()),
                    },
                    &runtime,
                )
                .unwrap()
            else {
                panic!()
            };
            assert!(c
                .routes
                .iter()
                .any(|route| route.id == "browser" && route.media == ["audio", "video"]));
            assert!(c.limits.video_profile.is_some());
        } else {
            assert!(right.is_err());
        }
        let outsider = f.store.create_identity("outsider").unwrap().id;
        let key = f
            .store
            .issue(
                &outsider,
                Scope::operator(),
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap();
        assert!(f
            .store
            .execute_with_runtime(
                &key.id,
                &Operation::BrowserBinding {
                    conversation_id: None,
                    session_id: None
                },
                &runtime
            )
            .is_err());
    }
}

#[test]
fn vapi_routes_reject_video_before_media_attachment_and_keep_audio_supported() {
    for video in [false, true] {
        let f = fixture(video);
        let operator = f
            .store
            .issue(
                &f.a,
                Scope::operator(),
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap();
        f.store
            .configure_voice_route(
                &operator.id,
                &connect_store::VoiceRoute {
                    id: "vapi-audio".into(),
                    conversation_id: f.conversation.clone(),
                    identity_id: f.b.clone(),
                    assistant_id: "12345678-1234-1234-1234-123456789012".into(),
                    phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
                    destinations: vec!["+15555550101".into()],
                    enabled: true,
                },
            )
            .unwrap();
        let route = f.store.vapi_session_route(&f.ak, &f.session);
        if video {
            assert_eq!(route.unwrap_err().code, "unsupported-media-profile");
        } else {
            assert!(route.unwrap().is_some());
        }
        let routed = f
            .store
            .admit_configured_session(&f.ak, &f.conversation, &f.b, &f.session);
        if video {
            assert_eq!(routed.unwrap_err().code, "unsupported-media-profile");
        } else {
            assert!(routed.is_ok());
        }
    }
}

#[test]
fn crashed_video_bindings_reconcile_without_resurrecting_media_or_ending_history() {
    let Fixture {
        _dir,
        store,
        ak,
        bk,
        session,
        conversation,
        ..
    } = fixture(true);
    store
        .bind_session_connection(&ak, &session, "lost-alice")
        .unwrap();
    store
        .bind_session_connection(&bk, &session, "lost-bob")
        .unwrap();
    drop(store); // no graceful unbind: model process loss with durable ready observations.
    let restarted = Store::open(&_dir.path().join("state.sqlite3"), Limits::default()).unwrap();
    assert_eq!(
        restarted.reconcile_interrupted_video_connections().unwrap(),
        1
    );
    assert_eq!(
        restarted.reconcile_interrupted_video_connections().unwrap(),
        0
    );
    let durable = restarted.authorize_session_binding(&ak, &session).unwrap();
    assert_eq!(durable.state, "accepted");
    assert_eq!(durable.conversation_id, conversation);
    assert!(restarted
        .unbind_session_connection("lost-alice")
        .unwrap()
        .is_none());
    restarted
        .bind_session_connection(&ak, &session, "fresh-alice")
        .unwrap();
    restarted
        .bind_session_connection(&bk, &session, "fresh-bob")
        .unwrap();
    assert_eq!(
        restarted
            .authorize_session_binding(&ak, &session)
            .unwrap()
            .state,
        "ready"
    );
    let audio = fixture(false);
    audio
        .store
        .bind_session_connection(&audio.ak, &audio.session, "audio-default")
        .unwrap();
    assert_eq!(
        audio
            .store
            .reconcile_interrupted_video_connections()
            .unwrap(),
        0
    );
    assert_eq!(
        audio
            .store
            .authorize_session_binding(&audio.ak, &audio.session)
            .unwrap()
            .state,
        "ready"
    );
}

#[test]
fn idle_projection_terminal_lookup_is_bounded_and_tracks_explicit_durable_end() {
    let f = fixture(true);
    assert!(f
        .store
        .terminal_media_session_ids(std::slice::from_ref(&f.session))
        .unwrap()
        .is_empty());
    f.store
        .execute(
            &f.ak,
            &Operation::SessionEnd {
                request_id: "end-idle".into(),
                conversation_id: f.conversation.clone(),
                session_id: f.session.clone(),
                reason: None,
            },
        )
        .unwrap();
    let mut projections = vec![f.session.clone()];
    projections.extend((0..300).map(|n| format!("absent-{n}")));
    let terminal = f.store.terminal_media_session_ids(&projections).unwrap();
    assert_eq!(terminal.len(), projections.len());
    assert!(terminal.contains(&f.session));
    assert!(f
        .store
        .terminal_media_session_ids(&vec![f.session; 10_001])
        .is_err());
}

#[test]
fn selected_browser_video_rejoins_a_fresh_invitation_after_explicit_end_in_another_conversation() {
    let f = fixture(true);
    let route = BrowserRoute {
        credential_id: f.ak.clone(),
        binding: BrowserBinding {
            conversation_id: f.conversation.clone(),
            participant_id: f.b.clone(),
            session_id: Some(f.session.clone()),
            signaling_url: "ws://127.0.0.1:8181".into(),
        },
    };
    let runtime = DiscoveryRuntime {
        browser: Some(route.clone()),
        ..Default::default()
    };
    f.store
        .execute(
            &f.ak,
            &Operation::SessionEnd {
                request_id: "end-initial".into(),
                conversation_id: f.conversation.clone(),
                session_id: f.session.clone(),
                reason: None,
            },
        )
        .unwrap();
    let Outcome::Conversation(next) = f
        .store
        .execute(
            &f.bk,
            &Operation::Create {
                request_id: "second-conversation".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![f
                    .a
                    .clone()]),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let Outcome::Session(invite) = f
        .store
        .execute(
            &f.bk,
            &Operation::SessionInvite {
                request_id: "second-invite".into(),
                conversation_id: next.id.clone(),
                participant_id: f.a.clone(),
                media: vec!["audio".into(), "video".into()],
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let selection = Operation::BrowserBinding {
        conversation_id: Some(next.id.clone()),
        session_id: Some(invite.id.clone()),
    };
    assert!(
        f.store
            .execute_with_runtime(&f.ak, &selection, &runtime)
            .is_err(),
        "invitation is not yet accepted"
    );
    f.store
        .execute(
            &f.ak,
            &Operation::SessionAccept {
                request_id: "second-accept".into(),
                conversation_id: next.id.clone(),
                session_id: invite.id.clone(),
            },
        )
        .unwrap();
    for (credential, peer) in [(&f.ak, &f.b), (&f.bk, &f.a)] {
        let Outcome::BrowserBinding(binding) = f
            .store
            .execute_with_runtime(credential, &selection, &runtime)
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(binding.conversation_id, next.id);
        assert_eq!(binding.session_id, Some(invite.id.clone()));
        assert_eq!(&binding.participant_id, peer);
        assert_eq!(
            f.store
                .authorize_selected_browser_video(credential, &route, &next.id, &invite.id)
                .unwrap()
                .session
                .id,
            invite.id
        );
    }
    assert!(f
        .store
        .execute_with_runtime(
            &f.ak,
            &Operation::BrowserBinding {
                conversation_id: Some(f.conversation.clone()),
                session_id: Some(invite.id.clone())
            },
            &runtime
        )
        .is_err());
    assert!(f
        .store
        .execute_with_runtime(
            &f.ak,
            &Operation::BrowserBinding {
                conversation_id: Some(next.id.clone()),
                session_id: None
            },
            &runtime
        )
        .is_err());
    assert!(
        f.store
            .authorize_selected_browser_video(&f.ak, &route, &f.conversation, &f.session)
            .is_err(),
        "terminal old Session cannot revive"
    );
    let operator = f
        .store
        .issue(
            &f.a,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    f.store
        .revoke(&f.store.revalidate(&operator.id).unwrap(), &f.ak)
        .unwrap();
    assert!(
        f.store
            .execute_with_runtime(&f.bk, &selection, &runtime)
            .is_err(),
        "revoked configured route cannot authorize another member"
    );
}
#[test]
fn selected_browser_video_refuses_audio_and_foreign_current_operator() {
    let f = fixture(false);
    let route = BrowserRoute {
        credential_id: f.ak.clone(),
        binding: BrowserBinding {
            conversation_id: f.conversation.clone(),
            participant_id: f.b.clone(),
            session_id: Some(f.session.clone()),
            signaling_url: String::new(),
        },
    };
    assert!(f
        .store
        .authorize_selected_browser_video(&f.ak, &route, &f.conversation, &f.session)
        .is_err());
    let f = fixture(true);
    let outsider = f.store.create_identity("selected-outsider").unwrap().id;
    let key = f
        .store
        .issue(
            &outsider,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let route = BrowserRoute {
        credential_id: f.ak.clone(),
        binding: BrowserBinding {
            conversation_id: f.conversation.clone(),
            participant_id: f.b.clone(),
            session_id: Some(f.session.clone()),
            signaling_url: String::new(),
        },
    };
    assert!(f
        .store
        .authorize_selected_browser_video(&key.id, &route, &f.conversation, &f.session)
        .is_err());
    assert!(f
        .store
        .authorize_selected_browser_video(&f.ak, &route, "bad id", &f.session)
        .is_err());
}

fn session_state(f: &Fixture) -> String {
    let Outcome::Session(session) = f
        .store
        .execute(
            &f.ak,
            &Operation::SessionGet {
                conversation_id: f.conversation.clone(),
                session_id: f.session.clone(),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    session.state
}
fn event_count(f: &Fixture, key: &str, kind: &str) -> usize {
    let Outcome::Inbox { events, .. } = f
        .store
        .execute(
            key,
            &Operation::Inbox {
                cursor: None,
                limit: 100,
                wait_ms: 0,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    events
        .iter()
        .filter(|event| {
            event.kind == kind
                && event
                    .session
                    .as_ref()
                    .is_some_and(|session| session.id == f.session)
        })
        .count()
}
fn end_operation(f: &Fixture, request_id: &str) -> Operation {
    Operation::SessionEnd {
        request_id: request_id.into(),
        conversation_id: f.conversation.clone(),
        session_id: f.session.clone(),
        reason: None,
    }
}
fn assert_terminal(f: &Fixture, state: &str) {
    assert_eq!(session_state(f), state);
    assert!(f
        .store
        .bind_session_connection(&f.ak, &f.session, "stale-attach")
        .is_err());
    assert!(f
        .store
        .bind_session_connection(&f.bk, &f.session, "stale-peer")
        .is_err());
    assert!(f
        .store
        .execute(
            &f.bk,
            &Operation::SessionAccept {
                request_id: "stale-accept".into(),
                conversation_id: f.conversation.clone(),
                session_id: f.session.clone(),
            }
        )
        .is_err());
    assert_eq!(session_state(f), state);
}

#[test]
fn explicit_end_races_attach_without_terminal_resurrection_or_duplicate_events() {
    for _ in 0..16 {
        let f = fixture(true);
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let attach = {
            let store = f.store.clone();
            let key = f.ak.clone();
            let session = f.session.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.bind_session_connection(&key, &session, "racing-attach")
            })
        };
        let end = {
            let store = f.store.clone();
            let key = f.bk.clone();
            let operation = end_operation(&f, "racing-end");
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.execute(&key, &operation)
            })
        };
        barrier.wait();
        let attachment = attach.join().unwrap();
        end.join().unwrap().unwrap();
        assert_terminal(&f, "ended");
        f.store.unbind_session_connection("racing-attach").unwrap();
        assert!(f
            .store
            .unbind_session_connection("racing-attach")
            .unwrap()
            .is_none());
        f.store
            .execute(&f.bk, &end_operation(&f, "racing-end"))
            .unwrap();
        f.store
            .execute(&f.ak, &end_operation(&f, "second-end"))
            .unwrap();
        for key in [&f.ak, &f.bk] {
            assert_eq!(event_count(&f, key, "session_ended"), 1);
            assert_eq!(
                event_count(&f, key, "session_ready"),
                usize::from(attachment.is_ok())
            );
            assert_eq!(event_count(&f, key, "session_interrupted"), 0);
        }
        assert_terminal(&f, "ended");
    }
}

#[test]
fn terminal_end_and_transient_cleanup_preserve_both_orderings() {
    for cleanup_first in [false, true] {
        let f = fixture(true);
        f.store
            .bind_session_connection(&f.ak, &f.session, "left")
            .unwrap();
        f.store
            .bind_session_connection(&f.bk, &f.session, "right")
            .unwrap();
        f.store.unbind_session_connection("left").unwrap();
        assert_eq!(session_state(&f), "ready");
        if cleanup_first {
            f.store.unbind_session_connection("right").unwrap();
            assert_eq!(session_state(&f), "accepted");
        }
        f.store
            .execute(&f.ak, &end_operation(&f, "cleanup-end"))
            .unwrap();
        if !cleanup_first {
            f.store.unbind_session_connection("right").unwrap();
        }
        f.store.reconcile_interrupted_video_connections().unwrap();
        assert_terminal(&f, "ended");
        for key in [&f.ak, &f.bk] {
            assert_eq!(event_count(&f, key, "session_ended"), 1);
            assert_eq!(
                event_count(&f, key, "session_interrupted"),
                usize::from(cleanup_first)
            );
            assert_eq!(event_count(&f, key, "session_ready"), 1);
        }
    }
}

#[test]
fn concurrent_accept_reject_has_one_winner_and_one_journaled_transition() {
    for round in 0..18 {
        let f = fixture_with_acceptance(true, false);
        let accept = Operation::SessionAccept {
            request_id: "race-accept".into(),
            conversation_id: f.conversation.clone(),
            session_id: f.session.clone(),
        };
        let reject = Operation::SessionReject {
            request_id: "race-reject".into(),
            conversation_id: f.conversation.clone(),
            session_id: f.session.clone(),
            reason: Some("declined".into()),
        };
        // The first two rounds explicitly cover each serialization; later rounds use a real barrier race.
        let (accepted, rejected) = match round {
            0 => {
                let a = f.store.execute(&f.bk, &accept);
                (a, f.store.execute(&f.bk, &reject))
            }
            1 => {
                let r = f.store.execute(&f.bk, &reject);
                (f.store.execute(&f.bk, &accept), r)
            }
            _ => {
                let barrier = Arc::new(std::sync::Barrier::new(3));
                let tasks: Vec<_> = [accept.clone(), reject.clone()]
                    .into_iter()
                    .map(|operation| {
                        let store = f.store.clone();
                        let key = f.bk.clone();
                        let barrier = barrier.clone();
                        std::thread::spawn(move || {
                            barrier.wait();
                            store.execute(&key, &operation)
                        })
                    })
                    .collect();
                barrier.wait();
                let mut results = tasks.into_iter().map(|task| task.join().unwrap());
                (results.next().unwrap(), results.next().unwrap())
            }
        };
        assert_ne!(accepted.is_ok(), rejected.is_ok());
        if accepted.is_ok() {
            assert_eq!(session_state(&f), "accepted");
            f.store.execute(&f.bk, &accept).unwrap();
            assert!(f.store.execute(&f.bk, &reject).is_err());
            f.store
                .execute(&f.ak, &end_operation(&f, "accepted-end"))
                .unwrap();
            assert_terminal(&f, "ended");
        } else {
            f.store.execute(&f.bk, &reject).unwrap();
            assert_terminal(&f, "rejected");
        }
        for key in [&f.ak, &f.bk] {
            assert_eq!(
                event_count(&f, key, "session_accepted"),
                usize::from(accepted.is_ok())
            );
            assert_eq!(
                event_count(&f, key, "session_rejected"),
                usize::from(rejected.is_ok())
            );
            assert_eq!(
                event_count(&f, key, "session_ended"),
                usize::from(accepted.is_ok())
            );
        }
    }
}
