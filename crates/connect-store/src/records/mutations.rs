//! Public mutation admission and durable replay constraints. Private backend
//! mutation facts alone confer no current audience or administrative authority.
use super::*;
use crate::{identity::actor_from_connection, Actor};
use connect_protocol::record_mutations::{Action, Mode, Outcome, RemoteDeletion, Request, Target};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplayAuthority {
    administrative: bool,
    correlation: String,
    subjects: BTreeSet<String>,
    groups: BTreeSet<String>,
}
fn query(request: &Request) -> read::RecordFetchIntent {
    read::RecordFetchIntent {
        conversation_id: request.conversation_id.clone(),
        session_id: request.session_id.clone(),
        uuid: Some(request.uuid.clone()),
        url: None,
    }
}
fn administrative(actor: &Actor) -> bool {
    actor.permits("admin") && actor.permits("record_admin")
}
fn admission(c: &Connection, actor: &Actor, request: &Request) -> Result<RecordScope> {
    actor.require(request.permission())?;
    if !administrative(actor) {
        return Ok(read::participant_mutation_origin(c, actor, &query(request))?.scope);
    }
    let tenant:Option<String>=c.query_row("SELECT c.tenant_id FROM conversations c JOIN sessions s ON s.conversation_id=c.id WHERE c.id=?1 AND s.id=?2",params![request.conversation_id,request.session_id],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
    Ok(RecordScope {
        tenant_id: tenant.ok_or_else(|| Error::new("not-found", "Record scope unavailable"))?,
        conversation_id: request.conversation_id.clone(),
        session_id: request.session_id.clone(),
    })
}
fn scopes(c: &Connection, actor: &Actor, request: &Request) -> Result<Vec<AuthorityScope>> {
    admission(c, actor, request)?;
    let mut scopes = crate::authority_admission::conversation_scopes(
        c,
        &request.conversation_id,
        AuthorityMode::Write,
    )?;
    scopes.push(AuthorityScope::identity(
        &actor.identity_id,
        AuthorityMode::Read,
    )?);
    Ok(scopes)
}
fn rules(bytes: &RecordBytes) -> Result<BTreeMap<String, Value>> {
    let manifest: Value = serde_json::from_slice(&bytes.manifest).map_err(|_| Error::storage())?;
    let rows = manifest["sources"]
        .as_array()
        .filter(|v| v.len() <= 4096)
        .ok_or_else(Error::storage)?;
    let mut result = BTreeMap::new();
    for row in rows {
        let id = row["id"]
            .as_str()
            .filter(|id| valid_id(id))
            .ok_or_else(Error::storage)?;
        if result.insert(id.into(), row.clone()).is_some() {
            return Err(Error::storage());
        }
    }
    Ok(result)
}
fn source_value<'a>(body: &'a Value, rule: &Value) -> Result<&'a Value> {
    let target = &rule["target"];
    if target["kind"] == "document" {
        return Ok(body);
    }
    let key = match target["kind"].as_str() {
        Some("party") => "parties",
        Some("analysis") => "analysis",
        Some("dialog") => "dialog",
        Some("attachment") => "attachments",
        _ => return Err(Error::storage()),
    };
    let index = target["index"]
        .as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .ok_or_else(Error::storage)?;
    body[key].get(index).ok_or_else(Error::storage)
}
fn affected(before: &RecordBytes, after: Option<&RecordBytes>) -> Result<BTreeSet<String>> {
    let original = rules(before)?;
    let Some(after) = after else {
        return Ok(original
            .into_iter()
            .filter(|(_, v)| v["state"] != "metadata-unavailable")
            .map(|(id, _)| id)
            .collect());
    };
    let next = rules(after)?;
    if original.keys().ne(next.keys()) {
        return Err(Error::storage());
    }
    let old_body: Value = serde_json::from_slice(&before.body).map_err(|_| Error::storage())?;
    let new_body: Value = serde_json::from_slice(&after.body).map_err(|_| Error::storage())?;
    let mut changed = BTreeSet::new();
    for (id, rule) in &original {
        let new = &next[id];
        if rule != new || source_value(&old_body, rule)? != source_value(&new_body, new)? {
            changed.insert(id.clone());
        }
    }
    Ok(changed)
}
/// Private adapter input, resolved from current semantic selectors under the
/// original writer/source guards. Never serialize internal manifest indexes.
pub struct SelectedSource {
    pub source_id: String,
    pub mode: Mode,
}
fn select(
    bytes: &RecordBytes,
    request: &Request,
    allowed: Option<&BTreeSet<String>>,
) -> Result<Vec<SelectedSource>> {
    let Action::Redact { sources } = &request.action else {
        return Ok(vec![]);
    };
    let all = rules(bytes)?;
    let body: Value = serde_json::from_slice(&bytes.body).map_err(|_| Error::storage())?;
    let mut result = vec![];
    for selected in sources {
        let mut matched = false;
        for (id, rule) in &all {
            if allowed.is_some_and(|allowed| !allowed.contains(id)) {
                continue;
            }
            let matches = match &selected.target {
                Target::Document => id == "document",
                Target::SessionOutcome => id == "session.outcome",
                Target::CaptureDiagnostics => id == "session.capture",
                Target::Participant {
                    participant_id,
                    membership_epoch,
                } => {
                    id.starts_with("appearance.") && {
                        let value = source_value(&body, rule)?;
                        value["connect_participant_id"].as_str() == Some(participant_id)
                            && value["membership_epoch"].as_u64() == Some(*membership_epoch)
                    }
                }
                Target::Connection {
                    connection_id,
                    generation,
                } => {
                    id.starts_with("connection.") && {
                        let value = source_value(&body, rule)?;
                        value["body"]["connection_id"].as_str() == Some(connection_id)
                            && value["body"]["generation_at_terminal_intent"].as_u64()
                                == Some(*generation)
                    }
                }
            };
            if matches {
                matched = true;
                result.push(SelectedSource {
                    source_id: id.clone(),
                    mode: selected.mode,
                });
            }
        }
        // Unknown and unauthorized selectors have the same refusal. A caller
        // cannot enumerate private sources through canonical-transform errors.
        if !matched {
            return Err(Error::forbidden());
        }
    }
    Ok(result)
}
fn remember(
    c: &Connection,
    request: &Request,
    bytes: &RecordBytes,
    selected: &BTreeSet<String>,
) -> Result<ReplayAuthority> {
    let all = rules(bytes)?;
    let mut selected = selected.clone();
    // Capture diagnostics were authorized only with the complete source set.
    // Preserve that footprint when the original bytes are later unavailable.
    if selected.contains("session.capture") {
        selected.extend(all.keys().cloned());
        selected.remove("session.capture");
    }
    let body: Value = serde_json::from_slice(&bytes.body).map_err(|_| Error::storage())?;
    let mut facts = ReplayAuthority::default();
    for id in selected {
        if matches!(id.as_str(), "document" | "session.outcome") {
            continue;
        }
        if id.starts_with("connection.") {
            facts.groups.insert(id);
            continue;
        }
        let index = id
            .strip_prefix("appearance.")
            .and_then(|s| s.parse::<usize>().ok())
            .ok_or_else(Error::forbidden)?;
        let participant = body["parties"]
            .get(index)
            .and_then(|v| v["connect_participant_id"].as_str())
            .ok_or_else(Error::forbidden)?;
        let identity:Option<String>=c.query_row("SELECT identity_id FROM participants WHERE conversation_id=?1 AND participant_id=?2",params![request.conversation_id,participant],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
        facts
            .subjects
            .insert(identity.ok_or_else(Error::forbidden)?);
    }
    Ok(facts)
}
fn replay_current(
    c: &Connection,
    actor: &Actor,
    request: &Request,
    facts: &ReplayAuthority,
) -> Result<()> {
    if administrative(actor) {
        return Ok(());
    }
    if facts.administrative || facts.subjects.iter().any(|id| !actor.permits_recipient(id)) {
        return Err(Error::forbidden());
    }
    let origin = read::participant_mutation_origin(c, actor, &query(request))?;
    let (participant, epoch) = origin.viewer.participant().ok_or_else(Error::forbidden)?;
    let mut q=c.prepare("SELECT source_group FROM session_record_audiences WHERE uuid=?1 AND participant_id=?2 AND membership_epoch=?3").map_err(|_|Error::storage())?;
    let groups = q
        .query_map(params![request.uuid, participant, epoch], |r| {
            r.get::<_, String>(0)
        })
        .map_err(|_| Error::storage())?
        .collect::<std::result::Result<BTreeSet<_>, _>>()
        .map_err(|_| Error::storage())?;
    let shared:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM history_share_items i JOIN history_shares h ON h.id=i.sharing_id AND h.active=1 WHERE i.kind='session' AND i.source_id=?1 AND i.conversation_id=?2 AND i.participant_id=?3 AND i.membership_epoch=?4)",params![request.session_id,request.conversation_id,participant,epoch],|r|r.get(0)).map_err(|_|Error::storage())?;
    if (!groups.contains("session") && !shared) || !facts.groups.is_subset(&groups) {
        return Err(Error::forbidden());
    }
    Ok(())
}
impl Store {
    /// Trusted Service entry, including current replay admission. The transform
    /// must validate canonical vCon/manifest/dependency semantics; Store checks
    /// the actual changed source footprint and commits authority facts atomically.
    pub fn mutate_authorized_record(
        &self,
        credential: &str,
        request: &Request,
        transform: impl FnOnce(
            &RecordBytes,
            &RecordHandle,
            &[SelectedSource],
        ) -> Result<Option<RecordBytes>>,
    ) -> Result<Outcome> {
        request.validate()?;
        let held = self.stable_authority(&AuthorityWait::new(Duration::from_secs(2)), || {
            let c = self.connection()?;
            let actor = actor_from_connection(&c, credential)?;
            scopes(&c, &actor, request)
        })?;
        let _record = self.record_reservation(&request.uuid, AuthorityMode::Write)?;
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential)?;
        let scope = admission(&tx, &actor, request)?;
        if !self.authority_covers(&held, &scopes(&tx, &actor, request)?) {
            return Err(Error::forbidden());
        }
        let digest = hash(
            &serde_json::to_vec(&("record-public-mutation-v1", &actor.identity_id, request))
                .map_err(|_| Error::storage())?,
        );
        let prior = row(&tx, &scope, &request.uuid)?
            .ok_or_else(|| Error::new("not-found", "Record unavailable"))?;
        verify(&prior)?;
        let supported:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM session_record_intents WHERE uuid=?1 AND state='ready' AND format_version=2 AND policy='connect-session-metadata-v1')",[&request.uuid],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !supported {
            return Err(Error::new(
                "unsupported-command",
                "Record mutation policy unavailable",
            ));
        }
        let mut applied = false;
        let saved:Option<(String,String,String)>=tx.query_row("SELECT actor_id,intent_hash,authority_json FROM session_record_mutation_authority WHERE uuid=?1 AND operation_id=?2",params![request.uuid,request.operation_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
        let kind = match request.action {
            Action::Redact { .. } => RecordMutationKind::Redact,
            Action::Delete => RecordMutationKind::Delete,
        };
        let fact=if let Some((owner,original,authorization))=saved {
            if owner!=actor.identity_id || original!=digest {return Err(conflict());}
            let facts:ReplayAuthority=serde_json::from_str(&authorization).map_err(|_|Error::storage())?;
            replay_current(&tx,&actor,request,&facts)?;
            let expected:String=tx.query_row("SELECT expected_json FROM session_record_mutations WHERE uuid=?1 AND operation_id=?2",params![request.uuid,request.operation_id],|r|r.get(0)).map_err(|_|Error::storage())?;
            let expected:RecordHandle=serde_json::from_str(&expected).map_err(|_|Error::storage())?;
            mutate_locked(&tx,&scope,&request.operation_id,kind,&digest,&expected,|_|Err(Error::storage()))?
        } else {
            if prior.handle.revision!=request.expected_revision {return Err(conflict());}
            let expected=prior.handle;
            let allowed=if administrative(&actor) {None} else {
                Some(read::participant_policy(&tx,&actor,&query(request))?.ok_or_else(Error::forbidden)?.sources)
            };
            let mut remembered=None;
            let result=mutate_locked(&tx,&scope,&request.operation_id,kind,&digest,&expected,|bytes| {
                let selected=select(bytes,request,allowed.as_ref())?;
                let next=transform(bytes,&expected,&selected)?;
                let mut changed=affected(bytes,next.as_ref())?;
                changed.extend(selected.iter().map(|s|s.source_id.clone()));
                let mut facts=if let Some(allowed)=&allowed {
                    if !changed.is_subset(allowed) {return Err(Error::forbidden());}
                    remember(&tx,request,bytes,&changed)?
                } else {ReplayAuthority {administrative:true,..Default::default()}};
                facts.correlation=format!("record_evt_{}",uuid::Uuid::new_v4().simple());
                remembered=Some(serde_json::to_string(&facts).map_err(|_|Error::storage())?);
                Ok(next)
            })?;
            // A private backend operation cannot be adopted as a public receipt.
            applied=true;
            let facts=remembered.ok_or_else(conflict)?;
            if facts.len()>1024*1024 {return Err(Error::storage());}
            tx.execute("INSERT INTO session_record_mutation_authority VALUES(?1,?2,?3,?4,?5)",params![request.uuid,request.operation_id,actor.identity_id,digest,facts]).map_err(|_|Error::storage())?;
            result
        }.ok_or_else(||Error::new("not-found","Record unavailable"))?;
        // File fences exclude authorization writers; time still advances while
        // canonical transformation runs. Expiry before commit rolls all SQL back.
        if applied {
            let saved:String=tx.query_row("SELECT authority_json FROM session_record_mutation_authority WHERE uuid=?1 AND operation_id=?2",params![request.uuid,request.operation_id],|r|r.get(0)).map_err(|_|Error::storage())?;
            let facts: ReplayAuthority =
                serde_json::from_str(&saved).map_err(|_| Error::storage())?;
            publish(
                self,
                &tx,
                &scope,
                &request.uuid,
                kind,
                &fact,
                &facts.correlation,
            )?;
        }
        actor_from_connection(&tx, credential)?.require(request.permission())?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Outcome {
            operation_id: fact.operation_id,
            uuid: request.uuid.clone(),
            revision: fact.result.revision,
            action: kind.name().into(),
            at: fact.at,
            remote_deletion: RemoteDeletion::NotAttempted,
        })
    }
}

pub(super) fn publish(
    store: &Store,
    tx: &rusqlite::Transaction<'_>,
    scope: &RecordScope,
    uuid: &str,
    mutation: RecordMutationKind,
    fact: &RecordMutationFact,
    correlation: &str,
) -> Result<()> {
    use connect_protocol::{events::RecordAvailability, membership::ContentKind};
    let recipients = crate::audience::source_members(
        tx,
        &scope.conversation_id,
        ContentKind::Session,
        &scope.session_id,
    )?;
    if !crate::conversations::capacity(tx, recipients.len() as u64, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Record mutation journal capacity reached",
        ));
    }
    let (kind, availability) = match mutation {
        RecordMutationKind::Delete => (
            "record_unavailable",
            RecordAvailability::Unavailable {
                session_id: scope.session_id.clone(),
            },
        ),
        RecordMutationKind::Redact => (
            "record_ready",
            RecordAvailability::Ready {
                session_id: scope.session_id.clone(),
                uuid: uuid.into(),
                revision: fact.result.revision,
            },
        ),
    };
    // Ready hints and context use the stable Record source. Unavailable hints
    // retain only Session metadata and must not reveal an erased record UUID.
    let (source_kind, source_id) = match &availability {
        RecordAvailability::Ready { uuid, .. } => ("record", uuid.as_str()),
        _ => ("session", scope.session_id.as_str()),
    };
    let data = serde_json::to_string(
        &serde_json::json!({"source_kind":source_kind,"source_id":source_id,"record":availability}),
    )
    .map_err(|_| Error::storage())?;
    let source = serde_json::to_string(
        &serde_json::json!({"kind":"connect-record-mutation","correlation_id":correlation}),
    )
    .map_err(|_| Error::storage())?;
    for recipient in recipients {
        tx.execute("INSERT INTO journal(id,recipient,conversation_id,kind,data_json,resource_revision,source_json) VALUES(?1,?2,?3,?4,?5,(SELECT state_revision FROM sessions WHERE id=?6),?7)",params![format!("evt_{}",uuid::Uuid::new_v4().simple()),recipient,scope.conversation_id,kind,data,scope.session_id,source]).map_err(|_|Error::storage())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use connect_protocol::{
        record_mutations::{Mode, Source},
        Limits,
    };
    struct Fixture {
        _dir: tempfile::TempDir,
        store: Store,
        owner: String,
        admin: String,
        uuid: String,
    }
    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let store =
                Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap();
            let owner = store.create_identity("owner").unwrap().id;
            let peer = store.create_identity("peer").unwrap().id;
            let admin = store
                .issue(&owner, crate::Scope::operator(), crate::now() + 3600)
                .unwrap()
                .id;
            let c = store.connection().unwrap();
            c.execute("INSERT INTO conversations(id) VALUES('conv_record')", [])
                .unwrap();
            c.execute("INSERT INTO participants(conversation_id,identity_id,participant_id,role) VALUES('conv_record',?1,'part_a','participant'),('conv_record',?2,'part_b','participant')",params![owner,peer]).unwrap();
            c.execute("INSERT INTO sessions(id,conversation_id,initiator_id,participant_id,state,media,created_at,updated_at) VALUES('sess_record','conv_record',?1,?2,'invited','[]',100,100)",params![owner,peer]).unwrap();
            c.execute(
                "UPDATE sessions SET state='cancelled' WHERE id='sess_record'",
                [],
            )
            .unwrap();
            drop(c);
            let uuid = store.pending_record_intents(1).unwrap().pop().unwrap();
            store.finalize_record_intent(&uuid,|intent|{
                let sources=serde_json::json!([
                    {"id":"document","target":{"kind":"document"}},
                    {"id":"session.outcome","target":{"kind":"analysis","index":0}},
                    {"id":"session.capture","target":{"kind":"analysis","index":1}},
                    {"id":"appearance.0","target":{"kind":"party","index":0}},
                    {"id":"appearance.1","target":{"kind":"party","index":1}},
                    {"id":"connection.0","target":{"kind":"analysis","index":2}}]);
                let body=serde_json::json!({"uuid":intent.uuid,"parties":[{"connect_participant_id":"part_a","membership_epoch":1},{"connect_participant_id":"part_b","membership_epoch":1}],"analysis":[{"body":{"state":"cancelled"}},{"body":{"appearance_omissions":0,"connection_omissions":0,"audience_omissions":0}},{"body":{"private":"lineage","connection_id":"private-connection","generation_at_terminal_intent":1}}]});
                Ok((RecordBytes {uuid:intent.uuid.clone(),body:serde_json::to_vec(&body).unwrap(),manifest:serde_json::to_vec(&serde_json::json!({"policy":"connect-session-metadata-v1","sources":sources})).unwrap()},vec![
                    super::super::RecordSourceEpoch{source_group:"session".into(),participant_id:"part_a".into(),membership_epoch:1},
                    super::super::RecordSourceEpoch{source_group:"session".into(),participant_id:"part_b".into(),membership_epoch:1},
                    super::super::RecordSourceEpoch{source_group:"connection.0".into(),participant_id:"part_b".into(),membership_epoch:1},
                ]))
            }).unwrap();
            Self {
                _dir: dir,
                store,
                owner,
                admin,
                uuid,
            }
        }
        fn request(&self, operation: &str, revision: u64, action: Action) -> Request {
            Request {
                operation_id: operation.into(),
                conversation_id: "conv_record".into(),
                session_id: "sess_record".into(),
                uuid: self.uuid.clone(),
                expected_revision: revision,
                action,
            }
        }
        fn key(&self, recipients: Vec<String>) -> String {
            self.store
                .issue(
                    &self.owner,
                    crate::Scope {
                        operations: vec![
                            "record_get".into(),
                            "record_redact".into(),
                            "record_delete".into(),
                        ],
                        recipients,
                    },
                    crate::now() + 300,
                )
                .unwrap()
                .id
        }
    }
    fn erase_appearance(bytes: &RecordBytes, index: usize) -> Result<Option<RecordBytes>> {
        let mut next = bytes.clone();
        let mut body: Value = serde_json::from_slice(&next.body).unwrap();
        body["parties"][index] = serde_json::json!({});
        next.body = serde_json::to_vec(&body).unwrap();
        Ok(Some(next))
    }
    #[test]
    fn source_erasure_checks_actual_changes_and_exact_replay_after_later_deletion() {
        let f = Fixture::new();
        let key = f.key(vec![]);
        let own = f.request(
            "erase-own",
            1,
            Action::Redact {
                sources: vec![Source {
                    target: Target::Participant {
                        participant_id: "part_a".into(),
                        membership_epoch: 1,
                    },
                    mode: Mode::Metadata,
                }],
            },
        );
        let journal_count = || {
            f.store
                .connection()
                .unwrap()
                .query_row("SELECT count(*) FROM journal", [], |r| r.get::<_, u64>(0))
                .unwrap()
        };
        let before = journal_count();
        let first = f
            .store
            .mutate_authorized_record(&key, &own, |b, _, _| erase_appearance(b, 0))
            .unwrap();
        assert_eq!(first.revision, 2);
        assert_eq!(journal_count(), before + 2);
        assert!(
            f.store
                .mutate_authorized_record(&key, &own, |_, _, _| panic!("replay transformed bytes"))
                .unwrap()
                == first
        );
        assert_eq!(
            journal_count(),
            before + 2,
            "receipt replay appended a journal event"
        );
        let denied = f.request(
            "erase-private",
            2,
            Action::Redact {
                sources: vec![Source {
                    target: Target::Connection {
                        connection_id: "private-connection".into(),
                        generation: 1,
                    },
                    mode: Mode::Metadata,
                }],
            },
        );
        assert!(
            matches!(f.store.mutate_authorized_record(&key,&denied,|b,_,_|{let mut next=b.clone();let mut body:Value=serde_json::from_slice(&next.body).unwrap();body["analysis"][2]=serde_json::json!({});next.body=serde_json::to_vec(&body).unwrap();Ok(Some(next))}),Err(e) if e.code=="forbidden")
        );
        // A declared shared source cannot conceal an actual change to a peer source.
        let deceit = f.request(
            "wrong-footprint",
            2,
            Action::Redact {
                sources: vec![Source {
                    target: Target::SessionOutcome,
                    mode: Mode::Metadata,
                }],
            },
        );
        assert!(
            matches!(f.store.mutate_authorized_record(&key,&deceit,|b,_,_|erase_appearance(b,1)),Err(e) if e.code=="forbidden")
        );
        let delete = f.request("delete-all", 2, Action::Delete);
        let deleted = f
            .store
            .mutate_authorized_record(&f.admin, &delete, |_, _, _| Ok(None))
            .unwrap();
        assert_eq!(deleted.revision, 3);
        assert_eq!(journal_count(), before + 4);
        let count: u64 = f
            .store
            .connection()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM journal WHERE kind='record_unavailable'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
        assert!(
            f.store
                .mutate_authorized_record(&key, &own, |_, _, _| panic!(
                    "redaction resurrected deletion"
                ))
                .unwrap()
                == first
        );
        assert!(
            f.store
                .mutate_authorized_record(&f.admin, &delete, |_, _, _| panic!(
                    "delete replay mutated"
                ))
                .unwrap()
                == deleted
        );
        assert_eq!(
            journal_count(),
            before + 4,
            "historical replay appended another event"
        );
        let exposed:u64=f.store.connection().unwrap().query_row("SELECT count(*) FROM journal WHERE source_json LIKE '%erase-own%' OR source_json LIKE '%delete-all%'",[],|r|r.get(0)).unwrap();
        assert_eq!(exposed, 0);
        let exposed:u64=f.store.connection().unwrap().query_row("SELECT count(*) FROM journal WHERE kind='record_unavailable' AND source_json LIKE '%'||?1||'%'",[&f.uuid],|r|r.get(0)).unwrap();
        assert_eq!(
            exposed, 0,
            "unavailable provenance disclosed a record handle"
        );
        let mut changed = own.clone();
        changed.expected_revision = 2;
        assert!(
            matches!(f.store.mutate_authorized_record(&key,&changed,|_,_,_|panic!()),Err(e) if e.code=="conflict")
        );
    }
    #[test]
    fn replay_rechecks_current_scope_recipient_ceiling_owner_and_revocation() {
        let f = Fixture::new();
        let broad = f.key(vec!["*".into()]);
        let request = f.request(
            "erase-peer",
            1,
            Action::Redact {
                sources: vec![Source {
                    target: Target::Participant {
                        participant_id: "part_b".into(),
                        membership_epoch: 1,
                    },
                    mode: Mode::Metadata,
                }],
            },
        );
        f.store
            .mutate_authorized_record(&broad, &request, |b, _, _| erase_appearance(b, 1))
            .unwrap();
        let narrow = f.key(vec![]);
        assert!(
            matches!(f.store.mutate_authorized_record(&narrow,&request,|_,_,_|panic!()),Err(e) if e.code=="forbidden")
        );
        let other = f.store.create_identity("other-admin").unwrap();
        let other = f
            .store
            .issue(&other.id, crate::Scope::operator(), crate::now() + 300)
            .unwrap();
        assert!(
            matches!(f.store.mutate_authorized_record(&other.id,&request,|_,_,_|panic!()),Err(e) if e.code=="conflict")
        );
        let actor = f.store.revalidate(&f.admin).unwrap();
        f.store.revoke(&actor, &broad).unwrap();
        assert!(f
            .store
            .mutate_authorized_record(&broad, &request, |_, _, _| panic!())
            .is_err());
    }
    fn change_outcome(bytes: &RecordBytes) -> Result<Option<RecordBytes>> {
        let mut next = bytes.clone();
        let mut body: Value = serde_json::from_slice(&next.body).unwrap();
        body["analysis"][0]["body"] = serde_json::json!({"unavailable":"redacted"});
        next.body = serde_json::to_vec(&body).unwrap();
        Ok(Some(next))
    }
    #[test]
    fn journal_capacity_failure_rolls_back_bytes_receipt_authority_and_original_retry() {
        let mut f = Fixture::new();
        let key = f.key(vec![]);
        let request = f.request(
            "journal-refused",
            1,
            Action::Redact {
                sources: vec![Source {
                    target: Target::SessionOutcome,
                    mode: Mode::Content,
                }],
            },
        );
        let before: u64 = f
            .store
            .connection()
            .unwrap()
            .query_row("SELECT count(*) FROM journal", [], |r| r.get(0))
            .unwrap();
        let limit = f.store.limits.journal_events;
        f.store.limits.journal_events = before;
        assert!(
            matches!(f.store.mutate_authorized_record(&key,&request,|b,_,_|change_outcome(b)),Err(e) if e.code=="capacity-exceeded")
        );
        let c = f.store.connection().unwrap();
        for table in [
            "session_record_mutations",
            "session_record_mutation_authority",
        ] {
            assert_eq!(
                c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, u64>(0))
                    .unwrap(),
                0
            );
        }
        assert_eq!(
            c.query_row(
                "SELECT revision FROM session_records WHERE uuid=?1",
                [&f.uuid],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            c.query_row("SELECT count(*) FROM journal", [], |r| r.get::<_, u64>(0))
                .unwrap(),
            before
        );
        drop(c);
        f.store.limits.journal_events = limit;
        assert_eq!(
            f.store
                .mutate_authorized_record(&key, &request, |b, _, _| change_outcome(b))
                .unwrap()
                .revision,
            2
        );
    }
    #[test]
    fn expiry_during_transform_rolls_back_every_fact_and_new_credential_can_retry() {
        let f = Fixture::new();
        let expires = crate::now() + 2;
        let key = f
            .store
            .issue(
                &f.owner,
                crate::Scope {
                    operations: vec!["record_get".into(), "record_redact".into()],
                    recipients: vec![],
                },
                expires,
            )
            .unwrap();
        let request = f.request(
            "expiry-before-commit",
            1,
            Action::Redact {
                sources: vec![Source {
                    target: Target::SessionOutcome,
                    mode: Mode::Content,
                }],
            },
        );
        let mut transformed = false;
        let result = f
            .store
            .mutate_authorized_record(&key.id, &request, |b, _, _| {
                transformed = true;
                let end = chrono::DateTime::from_timestamp(expires, 0).unwrap()
                    + chrono::Duration::milliseconds(100);
                if let Ok(wait) = (end - chrono::Utc::now()).to_std() {
                    std::thread::sleep(wait);
                }
                change_outcome(b)
            });
        assert!(transformed);
        assert!(result.is_err());
        let c = f.store.connection().unwrap();
        for table in [
            "session_record_mutations",
            "session_record_mutation_authority",
        ] {
            assert_eq!(
                c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, u64>(0))
                    .unwrap(),
                0
            );
        }
        assert_eq!(
            c.query_row(
                "SELECT revision FROM session_records WHERE uuid=?1",
                [&f.uuid],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            1
        );
        drop(c);
        let fresh = f.key(vec![]);
        assert_eq!(
            f.store
                .mutate_authorized_record(&fresh, &request, |b, _, _| change_outcome(b))
                .unwrap()
                .revision,
            2
        );
    }
}
