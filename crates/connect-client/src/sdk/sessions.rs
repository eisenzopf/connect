//! Scoped Session controls use topology assertions, independently of state revision.
use super::*;
use connect_protocol::{
    session_control::{Action, Request, SessionView},
    Outcome,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Medium {
    Audio,
    Video,
    Screen,
    Data,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionMedium {
    Voice,
    Video,
    VoiceVideo,
    ScreenShare,
    TextChat,
    Mixed,
}
impl SessionMedium {
    fn as_str(self) -> &'static str {
        match self {
            Self::Voice => "voice",
            Self::Video => "video",
            Self::VoiceVideo => "voice+video",
            Self::ScreenShare => "screen-share",
            Self::TextChat => "text-chat",
            Self::Mixed => "mixed",
        }
    }
}
pub enum SessionUpdate {
    ParticipantLeave(ParticipantId),
    RoleChange {
        participant_id: ParticipantId,
        role: String,
    },
    MediumUpgrade(NonEmpty<Medium>),
    CodecRenegotiate(NonEmpty<String>),
}
#[derive(Clone)]
pub struct SessionSnapshot {
    pub id: SessionId,
    pub conversation_id: ConversationId,
    pub state: String,
    pub media: Vec<Medium>,
    pub intent: Option<String>,
    pub capabilities_offer: Option<Value>,
    pub codec_preferences: Vec<String>,
    pub is_main: bool,
    pub revision: SessionRevision,
    pub topology_revision: TopologyRevision,
    pub created_at: i64,
    pub updated_at: i64,
    pub ended_reason: Option<String>,
}
impl std::fmt::Debug for SessionSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionSnapshot")
            .field("id", &self.id)
            .field("state", &self.state)
            .field("revision", &self.revision)
            .field("topology_revision", &self.topology_revision)
            .finish_non_exhaustive()
    }
}
impl TryFrom<SessionView> for SessionSnapshot {
    type Error = Error;
    fn try_from(v: SessionView) -> Result<Self> {
        let fail = || Error::request(ErrorKind::Protocol, None, false);
        let media = v
            .media
            .into_iter()
            .map(|v| match v.as_str() {
                "audio" => Ok(Medium::Audio),
                "video" => Ok(Medium::Video),
                "screen" => Ok(Medium::Screen),
                "data" => Ok(Medium::Data),
                _ => Err(fail()),
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            id: v.id.parse().map_err(|_| fail())?,
            conversation_id: v.conversation_id.parse().map_err(|_| fail())?,
            state: v.state,
            media,
            intent: v.intent,
            capabilities_offer: v.capabilities_offer,
            codec_preferences: v.codec_preferences,
            is_main: v.is_main,
            revision: SessionRevision::try_from(v.revision).map_err(|_| fail())?,
            topology_revision: TopologyRevision::try_from(v.topology_revision)
                .map_err(|_| fail())?,
            created_at: v.created_at,
            updated_at: v.updated_at,
            ended_reason: v.ended_reason,
        })
    }
}
#[derive(Clone)]
pub struct Session {
    client: Client,
    id: SessionId,
    conversation_id: ConversationId,
    receipt: Option<Arc<SessionSnapshot>>,
    reconciliation: Option<connect_protocol::session_codecs::ReconciliationReference>,
}
impl Session {
    fn new(client: Client, id: SessionId, conversation_id: ConversationId) -> Self {
        Self {
            client,
            id,
            conversation_id,
            receipt: None,
            reconciliation: None,
        }
    }
    pub fn id(&self) -> &SessionId {
        &self.id
    }
    pub fn conversation_id(&self) -> &ConversationId {
        &self.conversation_id
    }
    /// Original local receipt observation, never proof of media or current authority.
    pub fn receipt_snapshot(&self) -> Option<&SessionSnapshot> {
        self.receipt.as_deref()
    }
    /// Separate reconciliation work, when supplied; local policy receipt is not completion of it.
    pub fn codec_reconciliation(
        &self,
    ) -> Option<&connect_protocol::session_codecs::ReconciliationReference> {
        self.reconciliation.as_ref()
    }
    pub async fn refresh(&self, options: RequestOptions) -> Result<SessionSnapshot> {
        snapshot::read(&self.client, &self.conversation_id, options)
            .await?
            .sessions
            .remove(&self.id)
            .ok_or_else(|| Error::request(ErrorKind::ReceiptUnavailable, None, false))
    }
    pub async fn accept(
        &self,
        capabilities: Value,
        expected_topology_revision: TopologyRevision,
        options: MutationOptions,
    ) -> Result<Operation<Session>> {
        self.control(
            Action::Accept { capabilities },
            Some(expected_topology_revision),
            self.client.operation_kinds().session_accept,
            options,
        )
        .await
    }
    /// State revision cannot authorize this mutation.
    ///
    /// ```compile_fail
    /// use connect_client::sdk::{Session,SessionRevision,MutationOptions};
    /// fn wrong(session:&Session,state:SessionRevision,options:MutationOptions) {
    ///     let _=session.end(1000,"done".into(),state,options);
    /// }
    /// ```
    pub async fn end(
        &self,
        reason_code: u16,
        reason: String,
        expected_topology_revision: TopologyRevision,
        options: MutationOptions,
    ) -> Result<Operation<Session>> {
        self.control(
            Action::End {
                reason_code: u64::from(reason_code),
                reason,
            },
            Some(expected_topology_revision),
            self.client.operation_kinds().session_end,
            options,
        )
        .await
    }
    pub async fn reject(
        &self,
        reason_code: u16,
        reason: String,
        expected_topology_revision: TopologyRevision,
        options: MutationOptions,
    ) -> Result<Operation<SessionDecision>> {
        self.control(
            Action::Reject {
                reason_code: u64::from(reason_code),
                reason,
            },
            Some(expected_topology_revision),
            self.client.operation_kinds().session_reject,
            options,
        )
        .await
    }
    pub async fn cancel_invitation(
        &self,
        reason_code: u16,
        reason: String,
        expected_topology_revision: TopologyRevision,
        options: MutationOptions,
    ) -> Result<Operation<SessionDecision>> {
        self.control(
            Action::Cancel {
                reason_code: u64::from(reason_code),
                reason,
            },
            Some(expected_topology_revision),
            self.client.operation_kinds().session_cancel,
            options,
        )
        .await
    }
    pub async fn update(
        &self,
        input: SessionUpdate,
        expected_topology_revision: TopologyRevision,
        options: MutationOptions,
    ) -> Result<Operation<Session>> {
        let action = match input {
            SessionUpdate::ParticipantLeave(participant_id) => Action::Leave {
                participant_id: participant_id.as_str().into(),
            },
            SessionUpdate::RoleChange {
                participant_id,
                role,
            } => Action::Role {
                participant_id: participant_id.as_str().into(),
                role,
            },
            SessionUpdate::MediumUpgrade(media) => Action::Upgrade {
                media: media
                    .into_vec()
                    .into_iter()
                    .map(|m| {
                        match m {
                            Medium::Audio => "audio",
                            Medium::Video => "video",
                            Medium::Screen => "screen",
                            Medium::Data => "data",
                        }
                        .into()
                    })
                    .collect(),
            },
            SessionUpdate::CodecRenegotiate(codecs) => Action::Renegotiate {
                codec_preferences: codecs.into_vec(),
            },
        };
        self.control(
            action,
            Some(expected_topology_revision),
            self.client.operation_kinds().session_update,
            options,
        )
        .await
    }
    /// Invite another current Conversation Participant into this existing Session.
    /// Use current media and preserve the original intent/capability offer.
    pub async fn invite_participants(
        &self,
        to: NonEmpty<ParticipantId>,
        medium: SessionMedium,
        intent: String,
        capabilities: Value,
        expected_topology_revision: TopologyRevision,
        options: MutationOptions,
    ) -> Result<Operation<Session>> {
        self.control(
            Action::Invite {
                participants: to.into_vec().into_iter().map(String::from).collect(),
                medium: medium.as_str().into(),
                intent,
                capabilities,
            },
            Some(expected_topology_revision),
            self.client.operation_kinds().session_invite,
            options,
        )
        .await
    }
    async fn control<T>(
        &self,
        action: Action,
        expected: Option<TopologyRevision>,
        kind: OperationKind<T>,
        options: MutationOptions,
    ) -> Result<Operation<T>> {
        let mut request = Request {
            operation_id: options.operation_id.as_str().into(),
            conversation_id: self.conversation_id.as_str().into(),
            session_id: self.id.as_str().into(),
            by_participant: None,
            expected_revision: expected.map(TopologyRevision::get),
            action,
        };
        request
            .normalize()
            .map_err(|_| Error::local(ErrorKind::Validation))?;
        let body = match request.action {
            Action::Invite {
                participants,
                medium,
                intent,
                capabilities,
            } => {
                serde_json::json!({"to":participants,"medium":medium,"intent":intent,"capabilities_offer":capabilities})
            }
            Action::Accept { capabilities } => {
                serde_json::json!({"capabilities_answer":capabilities})
            }
            Action::Reject {
                reason_code,
                reason,
            }
            | Action::Cancel {
                reason_code,
                reason,
            }
            | Action::End {
                reason_code,
                reason,
            } => serde_json::json!({"reason_code":reason_code,"reason":reason}),
            Action::Leave { participant_id } => {
                serde_json::json!({"kind":"participant-leave","details":{"participant_id":participant_id}})
            }
            Action::Role {
                participant_id,
                role,
            } => {
                serde_json::json!({"kind":"role-change","details":{"participant_id":participant_id,"role":role}})
            }
            Action::Upgrade { media } => {
                serde_json::json!({"kind":"medium-upgrade","details":{"media":media}})
            }
            Action::Renegotiate { codec_preferences } => {
                serde_json::json!({"kind":"codec-renegotiate","details":{"codec_preferences":codec_preferences}})
            }
        };
        self.client
            .submit_local(
                kind,
                OperationScope {
                    conversation_id: Some(self.conversation_id.clone()),
                    session_id: Some(self.id.clone()),
                    connection_id: None,
                },
                expected.map(TopologyRevision::get),
                body,
                options,
            )
            .await
    }
}
impl Conversation {
    pub fn session_by_id(&self, id: SessionId) -> Session {
        Session::new(self.client().clone(), id, self.id().clone())
    }
    /// New Session creation permits an empty target list for a solo Session.
    pub async fn invite(
        &self,
        session_id: SessionId,
        to: Vec<ParticipantId>,
        medium: SessionMedium,
        intent: String,
        capabilities: Value,
        options: MutationOptions,
    ) -> Result<Operation<Session>> {
        self.session_by_id(session_id)
            .control(
                Action::Invite {
                    participants: to.into_iter().map(String::from).collect(),
                    medium: medium.as_str().into(),
                    intent,
                    capabilities,
                },
                None,
                self.client().operation_kinds().session_invite,
                options,
            )
            .await
    }
}
pub struct SessionDecision {
    pub session: Session,
    pub state: String,
    pub topology_revision: TopologyRevision,
    pub reason_code: u16,
    pub reason: String,
}
pub(super) fn decode(value: Value, client: &Client, key: &OperationKey) -> Result<Session> {
    let fail = || Error::request(ErrorKind::Protocol, Some(key), true);
    let Outcome::SessionControl(control) = serde_json::from_value(value).map_err(|_| fail())?
    else {
        return Err(fail());
    };
    if control.operation_id != key.operation_id.as_str()
        || key.scope.conversation_id.as_ref().map(|i| i.as_str())
            != Some(control.session.conversation_id.as_str())
        || key.scope.session_id.as_ref().map(|i| i.as_str()) != Some(control.session.id.as_str())
    {
        return Err(fail());
    }
    if key.command == "session.update" {
        if ![
            "participant-leave",
            "role-change",
            "medium-upgrade",
            "codec-renegotiate",
        ]
        .contains(&control.action.as_str())
        {
            return Err(fail());
        }
    } else if control.action != key.command {
        return Err(fail());
    }
    let snapshot = SessionSnapshot::try_from(control.session).map_err(|_| fail())?;
    let mut result = Session::new(
        client.clone(),
        snapshot.id.clone(),
        snapshot.conversation_id.clone(),
    );
    result.receipt = Some(Arc::new(snapshot));
    result.reconciliation = control.codec_reconciliation;
    Ok(result)
}
pub(super) fn decode_decision(
    value: Value,
    client: &Client,
    key: &OperationKey,
) -> Result<SessionDecision> {
    let fail = || Error::request(ErrorKind::Protocol, Some(key), true);
    let Outcome::SessionDecision(value) = serde_json::from_value(value).map_err(|_| fail())? else {
        return Err(fail());
    };
    let expected = match key.command.as_str() {
        "session.reject" => "rejected",
        "session.cancel" => "cancelled",
        _ => return Err(fail()),
    };
    if value.operation_id != key.operation_id.as_str()
        || key.scope.conversation_id.as_ref().map(|i| i.as_str())
            != Some(value.conversation_id.as_str())
        || key.scope.session_id.as_ref().map(|i| i.as_str()) != Some(value.session_id.as_str())
        || value.state != expected
        || value.reason.len() > 256
        || value.reason.chars().any(char::is_control)
    {
        return Err(fail());
    }
    Ok(SessionDecision {
        session: Session::new(
            client.clone(),
            value.session_id.parse().map_err(|_| fail())?,
            value.conversation_id.parse().map_err(|_| fail())?,
        ),
        state: value.state,
        topology_revision: TopologyRevision::try_from(value.revision).map_err(|_| fail())?,
        reason_code: u16::try_from(value.reason_code).map_err(|_| fail())?,
        reason: value.reason,
    })
}
