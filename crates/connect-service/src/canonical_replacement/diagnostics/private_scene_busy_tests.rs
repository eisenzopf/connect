//! Real production emitter/filter and file-custody controls; no media IO grant.
use super::*;
use connect_store::{AuthorityMode, AuthorityScope, Store};
use rvoip_core::staged_streams::StreamControlError;
use std::{
    collections::BTreeMap,
    sync::{atomic::AtomicUsize, Arc, Mutex},
};
use tracing::{
    field::{Field, Visit},
    Event, Subscriber,
};
use tracing_subscriber::{
    layer::{Context, SubscriberExt},
    Layer,
};

type Events = Arc<Mutex<Vec<BTreeMap<String, String>>>>;
struct Fields(BTreeMap<String, String>);
impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().into(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().into(), value.into());
    }
}
struct Capture {
    events: Events,
    store_writer: Option<(Arc<Store>, AuthorityScope, Arc<AtomicUsize>)>,
}
impl<S: Subscriber> Layer<S> for Capture {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if event.metadata().target() != PRIVATE_RECEIVE_TARGET {
            return;
        }
        if let Some((store, scope, acquired)) = &self.store_writer {
            // This is an actual Store/kernel file fence. A retained original
            // read guard would refuse this write, not merely change a counter.
            let writer = store
                .try_acquire_authority(&[scope.clone()])
                .expect("refusal emitter ran before original Store guard dropped");
            acquired.fetch_add(1, Ordering::Relaxed);
            drop(writer);
        }
        let mut fields = Fields(BTreeMap::new());
        event.record(&mut fields);
        self.events.lock().unwrap().push(fields.0);
    }
}
fn fields() -> PrivateReceiveFields<'static> {
    PrivateReceiveFields {
        connection: "target",
        target_generation: 2,
        operation_id: None,
        source_connection: "source",
        source_generation: 1,
        source_logical: "video",
        source_transport: "transport",
        source_stream_generation: 1,
        medium: Some(rvoip_core::stream_lifecycle::Medium::Video),
        target_logical: Some("video"),
        preroll_current: true,
        core_committed: false,
        root_cancelled: false,
    }
}
fn emit(
    diagnostic: &PrivateReceive,
    leaf: u32,
    error: &StreamControlError,
    busy: Option<PrivateSceneBusy>,
) {
    diagnostic.observe(leaf, |leaf| {
        emit_private_receive(leaf, &fields(), error, busy)
    });
}

#[test]
fn private_scene_busy_known_store_messages_keep_closed_subtypes_and_original_mapping() {
    let production = concat!(
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../connect-store/src/packet_observation.rs"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../connect-store/src/authority_fences.rs"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../connect-store/src/peer_authority/connection_media/snapshot.rs"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../connect-store/src/peer_authority/connection_media.rs"
        )),
    );
    let mut names = std::collections::BTreeSet::new();
    for &(message, expected) in PRIVATE_SCENE_BUSY_DETAILS {
        assert!(
            production.contains(message),
            "allowlisted subtype is not an existing Store error: {message}"
        );
        assert!(
            names.insert(expected.name()),
            "two distinct Store subtypes collapsed"
        );
        let mut detail = None;
        let result: rvoip_core::staged_streams::ControlResult<()> = private_scene_result(
            Err(connect_protocol::Error::new("authority-busy", message)),
            &mut detail,
        );
        assert!(matches!(result, Err(StreamControlError::NotReady)));
        assert_eq!(detail, Some(expected));
    }
    assert_eq!(names.len(), 17);
    let mut detail = None;
    assert_eq!(private_scene_result(Ok(41), &mut detail).unwrap(), 41);
    assert_eq!(
        detail, None,
        "success must not create a refusal observation"
    );
}

#[test]
fn actual_private_scene_emitter_uses_runner_filters_once_per_leaf_and_same_fourteen_cap() {
    for directives in super::private_receive_filter_tests::required_runner_filters() {
        let diagnostic = PrivateReceive::default();
        tracing::subscriber::with_default(tracing::subscriber::NoSubscriber::default(), || {
            emit(
                &diagnostic,
                5,
                &StreamControlError::NotReady,
                Some(PrivateSceneBusy::ObserverSlotsOccupied),
            );
        });
        assert_eq!(
            diagnostic.seen.load(Ordering::Relaxed),
            0,
            "disabled emitter consumed original budget"
        );
        let events = Events::default();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::try_new(&directives).unwrap())
            .with(Capture {
                events: events.clone(),
                store_writer: None,
            });
        tracing::subscriber::with_default(subscriber, || {
            for _ in 0..2 {
                for leaf in 0..PRIVATE_RECEIVE_LEAVES.len() as u32 {
                    emit(
                        &diagnostic,
                        leaf,
                        &StreamControlError::NotReady,
                        (leaf == 5).then_some(PrivateSceneBusy::ReaderRegistry),
                    );
                }
            }
            emit(
                &diagnostic,
                PRIVATE_RECEIVE_LEAVES.len() as u32,
                &StreamControlError::Authority,
                None,
            );
        });
        let actual = events.lock().unwrap();
        assert_eq!(
            actual.len(),
            14,
            "actual production emitter flooded or actual runner omitted it: {directives}"
        );
        assert_eq!(diagnostic.seen.load(Ordering::Relaxed).count_ones(), 14);
        for event in actual.iter() {
            assert!(event["message"].contains("Original private receive authority leaf refused"));
            assert_eq!(
                event["store_busy"],
                if event["leaf"] == "current-scene-read" {
                    "authority-reader-registry"
                } else {
                    "not-observed"
                }
            );
        }
    }
}

#[test]
fn actual_store_busy_guard_is_released_before_private_refusal_event() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Default::default()).unwrap());
    let read_scope =
        AuthorityScope::identity("diagnostic-original-file-custody", AuthorityMode::Read).unwrap();
    let write_scope =
        AuthorityScope::identity("diagnostic-original-file-custody", AuthorityMode::Write).unwrap();
    let mut detail = None;
    let result = private_scene_result(
        {
            let original = store.try_acquire_authority(&[read_scope]).unwrap();
            let refused = store
                .try_acquire_authority(&[write_scope.clone()])
                .map(|_| ());
            assert_eq!(refused.as_ref().unwrap_err().code, "authority-busy");
            // Same boundary as Store returning Err: release original file custody
            // before the error classifier and later existing leaf event execute.
            drop(original);
            refused
        },
        &mut detail,
    );
    assert!(matches!(result, Err(StreamControlError::NotReady)));
    assert_eq!(detail, Some(PrivateSceneBusy::AuthorityCurrentBusy));
    for directives in super::private_receive_filter_tests::required_runner_filters() {
        let events = Events::default();
        let acquired = Arc::new(AtomicUsize::new(0));
        let subscriber = tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::try_new(&directives).unwrap())
            .with(Capture {
                events: events.clone(),
                store_writer: Some((store.clone(), write_scope.clone(), acquired.clone())),
            });
        tracing::subscriber::with_default(subscriber, || {
            emit(
                &PrivateReceive::default(),
                5,
                result.as_ref().unwrap_err(),
                detail,
            );
        });
        assert_eq!(acquired.load(Ordering::Relaxed), 1);
        assert_eq!(events.lock().unwrap().len(), 1);
    }
}

#[test]
fn unknown_busy_and_nonbusy_refusals_emit_no_raw_error_text_or_reclassification() {
    const SECRET: &str = "private credential/path/identity must never become diagnostic detail";
    for (code, expected_kind, busy) in [
        (
            "authority-busy",
            PrivateSceneBusy::UnknownAuthorityBusy,
            true,
        ),
        ("forbidden", PrivateSceneBusy::NotAuthorityBusy, false),
        (
            "storage-unavailable",
            PrivateSceneBusy::NotAuthorityBusy,
            false,
        ),
    ] {
        let mut detail = None;
        let result: rvoip_core::staged_streams::ControlResult<()> =
            private_scene_result(Err(connect_protocol::Error::new(code, SECRET)), &mut detail);
        assert_eq!(detail, Some(expected_kind));
        assert!(if busy {
            matches!(&result, Err(StreamControlError::NotReady))
        } else {
            matches!(&result, Err(StreamControlError::Authority))
        });
        for directives in super::private_receive_filter_tests::required_runner_filters() {
            let events = Events::default();
            let subscriber = tracing_subscriber::registry()
                .with(tracing_subscriber::EnvFilter::try_new(&directives).unwrap())
                .with(Capture {
                    events: events.clone(),
                    store_writer: None,
                });
            tracing::subscriber::with_default(subscriber, || {
                emit(
                    &PrivateReceive::default(),
                    5,
                    result.as_ref().unwrap_err(),
                    detail,
                );
            });
            let actual = events.lock().unwrap();
            assert_eq!(actual.len(), 1);
            assert_eq!(actual[0]["store_busy"], expected_kind.name());
            assert!(actual[0].values().all(|value| !value.contains(SECRET)));
            assert!(!actual[0].contains_key("store_message"));
        }
    }
    let spoof =
        connect_protocol::Error::new("forbidden", "Current authority reader registry is busy");
    assert_eq!(
        PrivateSceneBusy::from_error(&spoof),
        PrivateSceneBusy::NotAuthorityBusy
    );
    let prefix = connect_protocol::Error::new(
        "authority-busy",
        "Current authority reader registry is busy /private/path",
    );
    assert_eq!(
        PrivateSceneBusy::from_error(&prefix),
        PrivateSceneBusy::UnknownAuthorityBusy
    );
}
