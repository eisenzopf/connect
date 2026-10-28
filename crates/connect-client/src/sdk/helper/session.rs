//! Typed Session dispatch; topology and lifecycle behavior stay in the SDK.
use super::*;

#[derive(Deserialize)]
pub enum InviteMedium {
    #[serde(rename = "voice")]
    Voice,
    #[serde(rename = "video")]
    Video,
    #[serde(rename = "voice+video")]
    VoiceVideo,
    #[serde(rename = "screen-share")]
    ScreenShare,
    #[serde(rename = "text-chat")]
    TextChat,
    #[serde(rename = "mixed")]
    Mixed,
}
impl From<InviteMedium> for SessionMedium {
    fn from(value: InviteMedium) -> Self {
        match value {
            InviteMedium::Voice => Self::Voice,
            InviteMedium::Video => Self::Video,
            InviteMedium::VoiceVideo => Self::VoiceVideo,
            InviteMedium::ScreenShare => Self::ScreenShare,
            InviteMedium::TextChat => Self::TextChat,
            InviteMedium::Mixed => Self::Mixed,
        }
    }
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Change {
    ParticipantLeave {
        participant_id: ParticipantId,
    },
    RoleChange {
        participant_id: ParticipantId,
        role: String,
    },
    MediumUpgrade {
        media: Vec<Medium>,
    },
    CodecRenegotiate {
        codec_preferences: Vec<String>,
    },
}
fn nonempty<T>(values: Vec<T>) -> Result<NonEmpty<T>> {
    let mut values = values.into_iter();
    Ok(NonEmpty {
        first: values
            .next()
            .ok_or_else(|| Error::local(ErrorKind::Validation))?,
        rest: values.collect(),
    })
}
impl TryFrom<Change> for SessionUpdate {
    type Error = Error;
    fn try_from(value: Change) -> Result<Self> {
        Ok(match value {
            Change::ParticipantLeave { participant_id } => Self::ParticipantLeave(participant_id),
            Change::RoleChange {
                participant_id,
                role,
            } => Self::RoleChange {
                participant_id,
                role,
            },
            Change::MediumUpgrade { media } => Self::MediumUpgrade(nonempty(media)?),
            Change::CodecRenegotiate { codec_preferences } => {
                Self::CodecRenegotiate(nonempty(codec_preferences)?)
            }
        })
    }
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionAction {
    Invite {
        to: Vec<ParticipantId>,
        medium: InviteMedium,
        intent: String,
        capabilities: Value,
        #[serde(default)]
        expected_revision: Option<TopologyRevision>,
        mutation: Mutation,
    },
    Accept {
        capabilities: Value,
        expected_revision: TopologyRevision,
        mutation: Mutation,
    },
    Reject {
        reason_code: u16,
        reason: String,
        expected_revision: TopologyRevision,
        mutation: Mutation,
    },
    Cancel {
        reason_code: u16,
        reason: String,
        expected_revision: TopologyRevision,
        mutation: Mutation,
    },
    End {
        reason_code: u16,
        reason: String,
        expected_revision: TopologyRevision,
        mutation: Mutation,
    },
    Update {
        update: Change,
        expected_revision: TopologyRevision,
        mutation: Mutation,
    },
    Refresh,
}
pub(super) async fn execute(
    client: &Client,
    cid: ConversationId,
    sid: SessionId,
    action: SessionAction,
    options: RequestOptions,
) -> Result<Value> {
    let room = client.conversation_by_id(cid);
    let session = room.session_by_id(sid.clone());
    Ok(match action {
        SessionAction::Invite {
            to,
            medium,
            intent,
            capabilities,
            expected_revision,
            mutation,
        } => match expected_revision {
            Some(expected) => session
                .invite_participants(
                    nonempty(to)?,
                    medium.into(),
                    intent,
                    capabilities,
                    expected,
                    mutation.options(options),
                )
                .await?
                .reference_json(),
            None => room
                .invite(
                    sid,
                    to,
                    medium.into(),
                    intent,
                    capabilities,
                    mutation.options(options),
                )
                .await?
                .reference_json(),
        },
        SessionAction::Accept {
            capabilities,
            expected_revision,
            mutation,
        } => session
            .accept(capabilities, expected_revision, mutation.options(options))
            .await?
            .reference_json(),
        SessionAction::Reject {
            reason_code,
            reason,
            expected_revision,
            mutation,
        } => session
            .reject(
                reason_code,
                reason,
                expected_revision,
                mutation.options(options),
            )
            .await?
            .reference_json(),
        SessionAction::Cancel {
            reason_code,
            reason,
            expected_revision,
            mutation,
        } => session
            .cancel_invitation(
                reason_code,
                reason,
                expected_revision,
                mutation.options(options),
            )
            .await?
            .reference_json(),
        SessionAction::End {
            reason_code,
            reason,
            expected_revision,
            mutation,
        } => session
            .end(
                reason_code,
                reason,
                expected_revision,
                mutation.options(options),
            )
            .await?
            .reference_json(),
        SessionAction::Update {
            update,
            expected_revision,
            mutation,
        } => session
            .update(
                update.try_into()?,
                expected_revision,
                mutation.options(options),
            )
            .await?
            .reference_json(),
        SessionAction::Refresh => snapshot_json(session.refresh(options).await?),
    })
}
pub(super) fn handle_json(value: Session) -> Value {
    json!({"id":value.id(),"conversationId":value.conversation_id()})
}
pub(super) fn decision_json(value: SessionDecision) -> Value {
    json!({"session":handle_json(value.session),"state":value.state,"topologyRevision":value.topology_revision})
}
fn snapshot_json(value: SessionSnapshot) -> Value {
    json!({"id":value.id,"conversationId":value.conversation_id,"state":value.state,"media":value.media,
        "intent":value.intent,"capabilitiesOffer":value.capabilities_offer,"codecPreferences":value.codec_preferences,
        "isMain":value.is_main,"revision":value.revision,"topologyRevision":value.topology_revision,
        "createdAt":value.created_at,"updatedAt":value.updated_at,"endedReason":value.ended_reason})
}
