//! Actual worker dispatch must preserve task denial without disabling owner control.
#![allow(clippy::unwrap_used)]
use async_trait::async_trait;
use connect_connector_api::{VoiceQuery, VoiceStart};
use connect_connectors::{
    voice::{RemoteCall, VoiceConnector},
    Failure,
};
use connect_protocol::{tasks::*, Limits, Operation, Outcome};
use connect_server::voice::Worker;
use connect_service::Service;
use connect_store::{Scope, Store, VoiceRoute};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

#[derive(Default)]
struct Driver {
    starts: AtomicUsize,
    ends: AtomicUsize,
}
impl Driver {
    fn remote(&self) -> RemoteCall {
        RemoteCall {
            id: "12345678-1234-1234-1234-123456789015".into(),
            state: if self.ends.load(Ordering::SeqCst) == 0 {
                "in-progress"
            } else {
                "ended"
            }
            .into(),
            control_url: None,
            ended_reason: None,
        }
    }
}
#[async_trait]
impl VoiceConnector for Driver {
    fn voice_manifest(&self) -> connect_connector_api::manifest::Manifest {
        let mut m = connect_connectors::provider_mapping::vapi_telephone_manifest();
        m.operations
            .remove(&connect_connector_api::manifest::ConnectorOperation::VoiceReconcile);
        m
    }
    fn voice_configuration(&self) -> std::collections::BTreeMap<String, String> {
        connect_connectors::provider_mapping::references("private-key", "OFFLINE_FIXTURE")
    }
    fn normalize(
        &self,
        id: &str,
        call: &RemoteCall,
    ) -> Result<connect_connector_api::lifecycle::Observation, Failure> {
        use connect_connector_api::lifecycle::*;
        Ok(Observation {
            mutation_id: id.into(),
            external_id: Some(call.id.clone()),
            event_id: format!("{}:{}", call.id, call.state),
            state: State::Voice(connect_connectors::provider_mapping::vapi_voice_state(
                &call.state,
            )),
            verification: Verification::AuthenticatedResponse,
        })
    }

    async fn inbound(&self, _: &VoiceQuery) -> Result<Vec<connect_store::InboundCall>, Failure> {
        Ok(Vec::new())
    }
    async fn start(&self, _: &VoiceStart) -> Result<RemoteCall, Failure> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        Ok(self.remote())
    }
    async fn status(&self, _: &str) -> Result<RemoteCall, Failure> {
        Ok(self.remote())
    }
    async fn end(&self, _: &str, _: Option<&str>) -> Result<RemoteCall, Failure> {
        self.ends.fetch_add(1, Ordering::SeqCst);
        Ok(self.remote())
    }
}

fn task(store: &Store, key: &str, cid: &str, owner: &str, request: &str) -> TaskRecord {
    let TaskOutcome::Record(task) = store
        .execute_task(
            key,
            &TaskRequest::new(TaskCommand::Create {
                request_id: request.into(),
                definition: TaskDefinition {
                    conversation_id: cid.into(),
                    scope: TaskScope {
                        recipients: vec![owner.into()],
                        operations: vec![TaskPermission::CallStart, TaskPermission::CallEnd],
                        sms_enrollments: Vec::new(),
                    },
                    expires_at: chrono::Utc::now().timestamp() + 600,
                    initiating_event_id: None,
                },
            }),
        )
        .unwrap()
    else {
        panic!("task record")
    };
    *task
}
fn invoke(store: &Store, key: &str, task: &TaskRecord, operation: Operation) -> Outcome {
    let outcome = store
        .execute_task(
            key,
            &TaskRequest::new(TaskCommand::Invoke {
                request_id: operation.request_id().unwrap().into(),
                task_id: task.id.clone(),
                step_id: "voice".into(),
                operation: Box::new(operation),
                reply_to: None,
                wait_for_reply: false,
            }),
        )
        .unwrap();
    let TaskOutcome::Tool(TaskToolResult::Pending {
        effect_id: Some(effect_id),
        ..
    }) = outcome
    else {
        panic!("pending physical effect")
    };
    let TaskOutcome::Effect(effect) = store
        .execute_task(
            key,
            &TaskRequest::new(TaskCommand::EffectGet {
                task_id: task.id.clone(),
                effect_id,
            }),
        )
        .unwrap()
    else {
        panic!("effect record")
    };
    let outcome = *effect.outcome.expect("accepted local operation");
    store
        .execute_task(
            key,
            &TaskRequest::new(TaskCommand::Cancel {
                request_id: format!("cancel-{}", task.id),
                task_id: task.id.clone(),
                reason: None,
            }),
        )
        .unwrap();
    outcome
}
async fn wait_state(store: &Store, key: &str, cid: &str, call: &str, state: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let Outcome::Call(value) = store
                .execute(
                    key,
                    &Operation::CallStatus {
                        conversation_id: cid.into(),
                        call_id: call.into(),
                    },
                )
                .unwrap()
            else {
                panic!("call")
            };
            if value.state == state {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelled_task_has_no_provider_effect_and_fresh_owner_hangup_still_works() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap());
    let owner = store.create_identity("owner").unwrap().id;
    let key = store
        .issue(
            &owner,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 3600,
        )
        .unwrap();
    let Outcome::Conversation(cid) = store
        .execute(
            &key.id,
            &Operation::Create {
                request_id: "conversation".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![]),
            },
        )
        .unwrap()
    else {
        panic!("conversation")
    };
    store
        .configure_voice_route(
            &key.id,
            &VoiceRoute {
                id: "voice".into(),
                conversation_id: cid.id.clone(),
                identity_id: owner.clone(),
                assistant_id: "12345678-1234-1234-1234-123456789012".into(),
                phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
                destinations: vec!["+15555550101".into()],
                enabled: true,
            },
        )
        .unwrap();
    let start_task = task(&store, &key.id, &cid.id, &owner, "start-task");
    let start = |id: &str| Operation::CallStart {
        request_id: id.into(),
        conversation_id: cid.id.clone(),
        route_id: "voice".into(),
        destination: "+15555550101".into(),
    };
    let denied = invoke(&store, &key.id, &start_task, start("task-start"));
    let service = Service::new(store.clone());
    let driver = Arc::new(Driver::default());
    let worker = Worker::with_driver(service.clone(), driver.clone()).unwrap();
    let Outcome::Call(denied) = denied else {
        panic!("call")
    };
    // Observe the worker's durable denial, not merely a timer with no network activity.
    wait_state(&store, &key.id, &cid.id, &denied.id, "forbidden").await;
    assert_eq!(driver.starts.load(Ordering::SeqCst), 0);
    let Outcome::Call(call) = store.execute(&key.id, &start("owner-start")).unwrap() else {
        panic!("call")
    };
    wait_state(&store, &key.id, &cid.id, &call.id, "in-progress").await;
    assert_eq!(driver.starts.load(Ordering::SeqCst), 1);
    worker.shutdown().await.unwrap();
    let end_task = task(&store, &key.id, &cid.id, &owner, "end-task");
    let end = |id: &str| Operation::CallEnd {
        request_id: id.into(),
        conversation_id: cid.id.clone(),
        call_id: call.id.clone(),
    };
    invoke(&store, &key.id, &end_task, end("task-end"));
    let worker = Worker::with_driver(service, driver.clone()).unwrap();
    // The cancelled request is resolved by the worker before another request is issued.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let Outcome::Call(value) = store
                .execute(
                    &key.id,
                    &Operation::CallStatus {
                        conversation_id: cid.id.clone(),
                        call_id: call.id.clone(),
                    },
                )
                .unwrap()
            else {
                panic!("call")
            };
            if !value.end_requested {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(driver.ends.load(Ordering::SeqCst), 0);
    store.execute(&key.id, &end("owner-end")).unwrap();
    wait_state(&store, &key.id, &cid.id, &call.id, "ended").await;
    assert_eq!(driver.starts.load(Ordering::SeqCst), 1);
    assert_eq!(driver.ends.load(Ordering::SeqCst), 1);
    worker.shutdown().await.unwrap();
}
