use super::*;

fn participant(c: &Connection, cid: &str, id: &str) -> Result<MemberView> {
    let identity: String = c
        .query_row(
            "SELECT identity_id FROM participants WHERE participant_id=?1 AND conversation_id=?2",
            params![id, cid],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?
        .ok_or_else(Error::forbidden)?;
    audience::member(c, &identity, cid, false)
}
pub(crate) fn stop_presence(c: &Connection, member: &MemberView) -> Result<()> {
    c.execute("UPDATE connections SET state='retired',generation=generation+1,revision=revision+1,observed_runtime_id=NULL,observed_at=NULL,readiness_json=NULL WHERE participant_id=?1 AND membership_epoch=?2 AND state NOT IN ('retired','failed')",params![member.participant_id,member.membership_epoch]).map_err(|_|Error::storage())?;
    c.execute("UPDATE session_participants SET state='removed' WHERE participant_id=?1 AND membership_epoch=?2 AND state NOT IN ('left','removed')",params![member.participant_id,member.membership_epoch]).map_err(|_|Error::storage())?;
    c.execute("UPDATE session_bindings SET state='ended',updated_at=?3 WHERE participant_id=?1 AND session_id IN (SELECT id FROM sessions WHERE conversation_id=?2)",params![member.identity_id,member.conversation_id,now()]).map_err(|_|Error::storage())?;
    c.execute("UPDATE membership_invitations SET state='cancelled' WHERE conversation_id=?1 AND inviter_id=?2 AND state='pending'",params![member.conversation_id,member.identity_id]).map_err(|_|Error::storage())?;
    Ok(())
}
fn shared_sources(
    c: &Connection,
    cid: &str,
    resources: &[connect_protocol::membership::ContentRef],
) -> Result<Vec<connect_protocol::membership::ContentRef>> {
    let mut expanded = resources.to_vec();
    for source in resources.iter().filter(|s| s.kind == ContentKind::Session) {
        let mut q=c.prepare("SELECT uuid FROM session_record_intents WHERE conversation_id=?1 AND session_id=?2 AND state='ready' ORDER BY uuid LIMIT 65").map_err(|_|Error::storage())?;
        let records = q
            .query_map(params![cid, source.id], |r| r.get::<_, String>(0))
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| Error::storage())?;
        if records.len() > 64 || expanded.len() + records.len() > 1024 {
            return Err(Error::new(
                "capacity-exceeded",
                "Shared record source budget exceeded",
            ));
        }
        expanded.extend(
            records
                .into_iter()
                .map(|id| connect_protocol::membership::ContentRef {
                    kind: ContentKind::Record,
                    id,
                }),
        );
    }
    let mut seen = std::collections::BTreeSet::new();
    expanded.retain(|source| seen.insert((source.kind.as_str(), source.id.clone())));
    Ok(expanded)
}

pub(super) fn apply(
    c: &Connection,
    actor: &Actor,
    request: &MembershipRequest,
    store: &Store,
) -> Result<MembershipOutcome> {
    let cid = request.conversation_id.as_str();
    match &request.change {
        MembershipAction::Add {
            identity_id,
            kind,
            role,
        } => {
            let count:u64=c.query_row("SELECT COUNT(*) FROM participants p JOIN identities i ON i.id=p.identity_id WHERE p.conversation_id=?1 AND p.active=1 AND i.revoked=0",[cid],|r|r.get(0)).map_err(|_|Error::storage())?;
            if count >= store.limits.participants as u64 {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Conversation participant capacity exceeded",
                ));
            }
            let existing:Option<(String,u64,bool)>=c.query_row("SELECT participant_id,membership_epoch,active FROM participants WHERE conversation_id=?1 AND identity_id=?2",params![cid,identity_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
            if let Some((id, epoch, active)) = existing {
                if active || epoch >= 9_007_199_254_740_991 {
                    return Err(conflict());
                }
                c.execute("UPDATE participants SET active=1,membership_epoch=membership_epoch+1,kind=?2,role=?3,joined_at=?4,removed_at=NULL,removal_reason=NULL WHERE participant_id=?1",params![id,kind.as_str(),role,now()]).map_err(|_|Error::storage())?;
            } else {
                c.execute("INSERT INTO participants(conversation_id,identity_id,participant_id,tenant_id,kind,role) VALUES(?1,?2,?3,?4,?5,?6)",params![cid,identity_id,format!("part_{}",uuid::Uuid::new_v4()),tenant(c,actor,cid)?,kind.as_str(),role]).map_err(|_|Error::storage())?;
            }
            c.execute("UPDATE membership_invitations SET state='cancelled' WHERE conversation_id=?1 AND identity_id=?2 AND state='pending'",params![cid,identity_id]).map_err(|_|Error::storage())?;
            Ok(MembershipOutcome::Participant {
                participant: audience::member(c, identity_id, cid, true)?,
                revision: 0,
                receipt: None,
            })
        }
        MembershipAction::Invite {
            identity_id,
            kind,
            role,
            expires_at,
        } => {
            if !actor.permits_recipient(identity_id) {
                return Err(Error::forbidden());
            }
            let tenant = tenant(c, actor, cid)?;
            let actual: Option<String> = c
                .query_row(
                    "SELECT kind FROM identities WHERE id=?1 AND tenant_id=?2 AND revoked=0",
                    params![identity_id, tenant],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|_| Error::storage())?;
            if actual.as_deref() != Some(kind.as_str()) {
                return Err(Error::forbidden());
            }
            if *expires_at <= now() || *expires_at > now().saturating_add(604800) {
                return Err(Error::new(
                    "invalid-request",
                    "Invitation lifetime is outside configured bounds",
                ));
            }
            let active:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM participants WHERE conversation_id=?1 AND identity_id=?2 AND active=1)",params![cid,identity_id],|r|r.get(0)).map_err(|_|Error::storage())?;
            if active {
                return Err(conflict());
            }
            c.execute("UPDATE membership_invitations SET state='expired' WHERE conversation_id=?1 AND identity_id=?2 AND state='pending' AND expires_at<=?3",params![cid,identity_id,now()]).map_err(|_|Error::storage())?;
            let id = format!("inv_{}", uuid::Uuid::new_v4());
            c.execute("INSERT INTO membership_invitations(id,conversation_id,tenant_id,identity_id,inviter_id,kind,role,state,expires_at,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'pending',?8,?9)",params![id,cid,tenant,identity_id,actor.identity_id,kind.as_str(),role,expires_at,now()]).map_err(|e|if matches!(e,rusqlite::Error::SqliteFailure(ref x,_) if x.code==rusqlite::ErrorCode::ConstraintViolation){conflict()}else{Error::storage()})?;
            Ok(MembershipOutcome::Invitation {
                invitation: invitation(c, &id, cid)?,
                receipt: None,
            })
        }
        MembershipAction::Join { invitation_id } => {
            let invite = invitation(c, invitation_id, cid)?;
            if invite.state != "pending" || invite.expires_at <= now() {
                return Err(Error::forbidden());
            }
            let count:u64=c.query_row("SELECT COUNT(*) FROM participants p JOIN identities i ON i.id=p.identity_id WHERE p.conversation_id=?1 AND p.active=1 AND i.revoked=0",[cid],|r|r.get(0)).map_err(|_|Error::storage())?;
            if count >= store.limits.participants as u64 {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Conversation participant capacity exceeded",
                ));
            }
            let (kind, role): (String, String) = c
                .query_row(
                    "SELECT kind,role FROM membership_invitations WHERE id=?1",
                    [invitation_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(|_| Error::storage())?;
            let current:Option<(String,u64,bool)>=c.query_row("SELECT participant_id,membership_epoch,active FROM participants WHERE conversation_id=?1 AND identity_id=?2",params![cid,actor.identity_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
            if let Some((id, epoch, active)) = current {
                if active || epoch >= 9_007_199_254_740_991 {
                    return Err(conflict());
                }
                c.execute("UPDATE participants SET active=1,membership_epoch=membership_epoch+1,kind=?2,role=?3,joined_at=?4,removed_at=NULL,removal_reason=NULL WHERE participant_id=?1",params![id,kind,role,now()]).map_err(|_|Error::storage())?;
            } else {
                c.execute("INSERT INTO participants(conversation_id,identity_id,participant_id,tenant_id,kind,role) VALUES(?1,?2,?3,?4,?5,?6)",params![cid,actor.identity_id,format!("part_{}",uuid::Uuid::new_v4()),tenant(c,actor,cid)?,kind,role]).map_err(|_|Error::storage())?;
            }
            let member = audience::member(c, &actor.identity_id, cid, true)?;
            c.execute(
                "UPDATE membership_invitations SET state='joined',joined_epoch=?2 WHERE id=?1",
                params![invitation_id, member.membership_epoch],
            )
            .map_err(|_| Error::storage())?;

            Ok(MembershipOutcome::Participant {
                participant: member,
                revision: 0,
                receipt: None,
            })
        }
        MembershipAction::Decline { invitation_id }
        | MembershipAction::CancelInvitation { invitation_id } => {
            let value = invitation(c, invitation_id, cid)?;
            if !actor.permits_recipient(&value.identity_id) {
                return Err(Error::forbidden());
            }
            if value.state != "pending" {
                return Err(conflict());
            }
            let state = if matches!(request.change, MembershipAction::Decline { .. }) {
                "declined"
            } else {
                "cancelled"
            };
            c.execute(
                "UPDATE membership_invitations SET state=?2 WHERE id=?1",
                params![invitation_id, state],
            )
            .map_err(|_| Error::storage())?;
            Ok(MembershipOutcome::Invitation {
                invitation: invitation(c, invitation_id, cid)?,
                receipt: None,
            })
        }
        MembershipAction::Leave {} | MembershipAction::Remove { .. } => {
            let member = match &request.change {
                MembershipAction::Remove { participant_id, .. } => {
                    participant(c, cid, participant_id)?
                }
                _ => audience::member(c, &actor.identity_id, cid, true)?,
            };
            if !member.active {
                return Err(conflict());
            }
            if !actor.permits_recipient(&member.identity_id) {
                return Err(Error::forbidden());
            }
            c.execute(
                "UPDATE participants SET active=0,removed_at=?2,removal_reason=?3 WHERE participant_id=?1",
                params![member.participant_id, now(),match &request.change {MembershipAction::Remove{reason,..}=>reason.as_deref(),_=>None}],
            )
            .map_err(|_| Error::storage())?;
            stop_presence(c, &member)?;
            let mut sessions=c.prepare("SELECT session_id FROM session_connected_presence WHERE participant_id=?1 AND membership_epoch=?2 AND state='joined'").map_err(|_|Error::storage())?;
            let ids = sessions
                .query_map(
                    params![member.participant_id, member.membership_epoch],
                    |r| r.get::<_, String>(0),
                )
                .map_err(|_| Error::storage())?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|_| Error::storage())?;
            let why = if matches!(request.change, MembershipAction::Leave {}) {
                "explicit-leave"
            } else {
                "kicked"
            };
            for sid in ids {
                crate::session_participant_lifecycle::reconcile(
                    c,
                    store,
                    cid,
                    &sid,
                    Some((&member.participant_id, why)),
                )?;
            }

            let mut removed = member;
            removed.active = false;
            removed.removal_reason = match &request.change {
                MembershipAction::Remove { reason, .. } => reason.clone(),
                _ => None,
            };
            Ok(MembershipOutcome::Participant {
                participant: removed,
                revision: 0,
                receipt: None,
            })
        }
        MembershipAction::UpdateRole {
            participant_id,
            role,
        } => {
            let member = participant(c, cid, participant_id)?;
            if !member.active || !actor.permits_recipient(&member.identity_id) {
                return Err(Error::forbidden());
            }
            // This changes only the Conversation default, never Session snapshots
            // or credential scopes. No label is promoted into a permission.
            c.execute(
                "UPDATE participants SET role=?2 WHERE participant_id=?1",
                params![participant_id, role],
            )
            .map_err(|_| Error::storage())?;
            Ok(MembershipOutcome::Participant {
                participant: audience::member(c, &member.identity_id, cid, true)?,
                revision: 0,
                receipt: None,
            })
        }
        MembershipAction::ShareHistory {
            resources,
            participants,
            provenance,
        } => {
            let id = format!("share_{}", uuid::Uuid::new_v4());
            c.execute("INSERT INTO history_shares(id,conversation_id,owner_id,provenance,active,created_at) VALUES(?1,?2,?3,?4,1,?5)",params![id,cid,actor.identity_id,provenance,now()]).map_err(|_|Error::storage())?;
            let sources = shared_sources(c, cid, resources)?;
            for target in participants {
                let member = participant(c, cid, target)?;
                for source in &sources {
                    c.execute("INSERT INTO history_share_items(sharing_id,conversation_id,kind,source_id,participant_id,membership_epoch) VALUES(?1,?2,?3,?4,?5,?6)",params![id,cid,source.kind.as_str(),source.id,target,member.membership_epoch]).map_err(|_|Error::storage())?;
                }
            }
            Ok(MembershipOutcome::Sharing {
                sharing_id: id,
                conversation_id: cid.into(),
                revision: 0,
                receipt: None,
                active: true,
            })
        }
        MembershipAction::RevokeHistoryShare { sharing_id } => {
            let owner: Option<String> = c
                .query_row(
                    "SELECT owner_id FROM history_shares WHERE id=?1 AND conversation_id=?2",
                    params![sharing_id, cid],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|_| Error::storage())?;
            if owner.as_deref() != Some(&actor.identity_id) && !actor.permits("admin") {
                return Err(Error::forbidden());
            }
            if owner.is_none() {
                return Err(Error::forbidden());
            }
            c.execute(
                "UPDATE history_shares SET active=0,revoked_by=?2 WHERE id=?1",
                params![sharing_id, actor.identity_id],
            )
            .map_err(|_| Error::storage())?;
            Ok(MembershipOutcome::Sharing {
                sharing_id: sharing_id.clone(),
                conversation_id: cid.into(),
                revision: 0,
                receipt: None,
                active: false,
            })
        }
    }
}

pub(super) fn emit(
    c: &Connection,
    actor: &Actor,
    request: &MembershipRequest,
    outcome: &MembershipOutcome,
    record: Option<&str>,
    revision: u64,
) -> Result<()> {
    let (kind, data) = match outcome {
        MembershipOutcome::Participant { participant, .. } => {
            let kind = match request.change {
                MembershipAction::Add { .. } | MembershipAction::Join { .. } => {
                    "conversation_participant_added"
                }
                MembershipAction::UpdateRole { .. } => "conversation_participant_updated",
                _ => "conversation_participant_removed",
            };
            (
                kind,
                serde_json::json!({"identity_id":participant.identity_id,"participant_id":participant.participant_id,"membership_epoch":participant.membership_epoch,"active":participant.active,"role":participant.role,"removal_reason":participant.removal_reason}),
            )
        }
        MembershipOutcome::Invitation { invitation, .. } => (
            "membership_invitation_changed",
            serde_json::json!({"identity_id":invitation.identity_id,"invitation_id":invitation.invitation_id,"state":invitation.state}),
        ),
        MembershipOutcome::Sharing {
            sharing_id, active, ..
        } => (
            "history_sharing_changed",
            serde_json::json!({"identity_id":actor.identity_id,"sharing_id":sharing_id,"active":active}),
        ),
    };
    let mut st=c.prepare("SELECT identity_id FROM participants p JOIN identities i ON i.id=p.identity_id WHERE p.conversation_id=?1 AND p.active=1 AND i.revoked=0 ORDER BY p.participant_id").map_err(|_|Error::storage())?;
    let recipients = st
        .query_map([&request.conversation_id], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    for recipient in recipients {
        c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,data_json,resource_revision,operation_record_id) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![uuid::Uuid::new_v4().to_string(),recipient,request.conversation_id,kind,serde_json::to_string(&data).map_err(|_|Error::storage())?,revision,record]).map_err(|_|Error::storage())?;
    }
    if let MembershipAction::ShareHistory {
        resources,
        participants,
        ..
    } = &request.change
    {
        let sources = shared_sources(c, &request.conversation_id, resources)?;
        for target in participants {
            let member = participant(c, &request.conversation_id, target)?;
            for source in &sources {
                let (message, session, artifact, call): (
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                ) = match source.kind {
                    ContentKind::Message => (Some(source.id.clone()), None, None, None),
                    ContentKind::Session => (
                        None,
                        Some(
                            serde_json::to_string(&crate::sessions::get(
                                c,
                                &request.conversation_id,
                                &source.id,
                            )?)
                            .map_err(|_| Error::storage())?,
                        ),
                        None,
                        None,
                    ),
                    ContentKind::Artifact => (
                        None,
                        None,
                        Some(
                            serde_json::to_string(&crate::artifacts::metadata(
                                c,
                                &request.conversation_id,
                                &source.id,
                            )?)
                            .map_err(|_| Error::storage())?,
                        ),
                        None,
                    ),
                    _ => (None, None, None, None),
                };
                let kind = match source.kind {
                    ContentKind::Artifact => "artifact_history_shared",
                    ContentKind::Session => "session_history_shared",
                    ContentKind::Record => "record_history_shared",
                    _ => "history_shared",
                };
                let data = serde_json::json!({"source_kind":source.kind,"source_id":source.id});
                c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,message_id,session_json,artifact_json,call_json,data_json,resource_revision,operation_record_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![uuid::Uuid::new_v4().to_string(),member.identity_id,request.conversation_id,kind,message,session,artifact,call,data.to_string(),revision,record]).map_err(|_|Error::storage())?;
            }
        }
    }
    Ok(())
}
