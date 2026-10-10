use super::{fixture::Fixture, replies};
use crate::{IssuedCredential, Scope};
use connect_protocol::{tasks::*, Operation, Outcome, Session};

pub(super) struct MediaFixture {
    pub f: Fixture,
    pub peer: IssuedCredential,
    pub peer_id: String,
    pub task: TaskRecord,
}
impl MediaFixture {
    pub fn new() -> Self {
        let (f, _, peer_id) = replies::setup();
        let peer = f
            .store
            .issue(
                &peer_id,
                Scope::communications(vec![f.owner.clone()]),
                f.now + 3600,
            )
            .unwrap();
        let task = Self::create_task(&f, &peer_id, "media-task");
        Self {
            f,
            peer,
            peer_id,
            task,
        }
    }
    pub fn create_task(f: &Fixture, peer: &str, id: &str) -> TaskRecord {
        let mut request = f.create();
        let TaskCommand::Create {
            request_id,
            definition,
        } = &mut request.command
        else {
            panic!("create")
        };
        *request_id = id.into();
        definition.scope.operations = vec![
            TaskPermission::SessionInvite,
            TaskPermission::SessionAccept,
            TaskPermission::SessionGet,
        ];
        definition.scope.recipients = vec![peer.into()];
        Fixture::record(f.run(&request).unwrap())
    }
    pub fn grant(&self, task: &TaskRecord, id: &str, operations: Vec<TaskPermission>) -> String {
        let issued = self
            .f
            .store
            .issue_local_task_grant(
                &self.f.key.id,
                id,
                &GrantDefinition {
                    task_id: task.id.clone(),
                    binding: InvocationBinding {
                        kind: InvocationKind::Native,
                        adapter: "resource-test".into(),
                        external_id: id.into(),
                    },
                    operations,
                    expires_at: self.f.now + 120,
                },
            )
            .unwrap();
        self.f
            .store
            .authenticate_native_task_grant(issued.expose_secret())
            .unwrap()
            .subject()
            .unwrap()
    }
    pub fn invoke(&self, id: &str, operation: Operation) -> Session {
        let result = self
            .f
            .run(&TaskRequest::new(TaskCommand::Invoke {
                request_id: id.into(),
                task_id: self.task.id.clone(),
                step_id: "session".into(),
                operation: Box::new(operation),
                reply_to: None,
                wait_for_reply: false,
            }))
            .unwrap();
        let TaskOutcome::Tool(TaskToolResult::Completed { outcome, .. }) = result else {
            panic!("completed session")
        };
        session(*outcome)
    }
    pub fn accepted_outbound(&self, id: &str, through_task: bool) -> Session {
        let invite = Operation::SessionInvite {
            request_id: id.into(),
            conversation_id: self.f.cid.clone(),
            participant_id: self.peer_id.clone(),
            media: vec!["audio".into()],
        };
        let invited = if through_task {
            self.invoke(id, invite)
        } else {
            session(self.f.store.execute(&self.f.key.id, &invite).unwrap())
        };
        session(
            self.f
                .store
                .execute(
                    &self.peer.id,
                    &Operation::SessionAccept {
                        request_id: format!("accept-{id}"),
                        conversation_id: self.f.cid.clone(),
                        session_id: invited.id,
                    },
                )
                .unwrap(),
        )
    }
    pub fn accepted_inbound(&self) -> Session {
        let invited = session(
            self.f
                .store
                .execute(
                    &self.peer.id,
                    &Operation::SessionInvite {
                        request_id: "inbound".into(),
                        conversation_id: self.f.cid.clone(),
                        participant_id: self.f.owner.clone(),
                        media: vec!["audio".into()],
                    },
                )
                .unwrap(),
        );
        self.invoke(
            "accept-inbound",
            Operation::SessionAccept {
                request_id: "accept-inbound".into(),
                conversation_id: self.f.cid.clone(),
                session_id: invited.id,
            },
        )
    }
}
fn session(outcome: Outcome) -> Session {
    let Outcome::Session(session) = outcome else {
        panic!("session")
    };
    session
}
