//! Bounded server callback jobs share the owned reader, media loop and slots.
use super::*;
use crate::canonical::{Envelope, Scope};
use serde_json::json;
use std::collections::{BTreeMap, VecDeque};
use tokio::time::Instant;
const ORIGIN_CAPACITY: usize = 64;
const RETIRED_QUERIES: usize = 64;
const POLL: Duration = Duration::from_millis(100);
#[derive(Clone)]
pub(super) struct Origin {
    key: OperationKey,
    record: Option<OperationRecordId>,
    completed: bool,
}
fn bad(key: &OperationKey) -> Error {
    Error::request(ErrorKind::Protocol, Some(key), true)
}
fn exact<'a>(
    cache: &'a mut NativeCache,
    key: &OperationKey,
) -> Result<Option<&'a mut NativeConnection>> {
    let Some(entry) = key
        .scope
        .connection_id
        .as_ref()
        .and_then(|id| cache.get_mut(id))
    else {
        return Ok(None);
    };
    if key.scope.session_id.as_ref() != Some(&entry.session)
        || key.scope.conversation_id.as_ref() != Some(&entry.conversation)
    {
        return Err(bad(key));
    }
    Ok((!entry.ended).then_some(entry))
}
pub(super) fn issued(cache: &std::sync::Mutex<NativeCache>, key: &OperationKey) -> Result<()> {
    if !super::super::selections::is_command(&key.command) {
        return Ok(());
    }
    let mut cache = cache.lock().map_err(|_| bad(key))?;
    if let Some(entry) = key
        .scope
        .connection_id
        .as_ref()
        .and_then(|id| cache.get_mut(id))
        .filter(|e| {
            !e.ended
                && Some(&e.session) == key.scope.session_id.as_ref()
                && Some(&e.conversation) == key.scope.conversation_id.as_ref()
        })
    {
        if !entry.selectors.iter().any(|o| o.key == *key) {
            if entry.selectors.len() == ORIGIN_CAPACITY {
                entry.selectors.pop_front();
            }
            entry.selectors.push_back(Origin {
                key: key.clone(),
                record: None,
                completed: false,
            });
        }
    }
    Ok(())
}
pub(super) fn remember(
    cache: &std::sync::Mutex<NativeCache>,
    key: &OperationKey,
    p: &Value,
) -> Result<()> {
    if p["operation_id"].as_str() != Some(key.operation_id.as_str()) {
        return Err(bad(key));
    }
    if p.get("command_type")
        .is_some_and(|value| value.as_str() != Some(key.command.as_str()))
    {
        return Err(bad(key));
    }
    let mut cache = cache.lock().map_err(|_| bad(key))?;
    let Some(entry) = exact(&mut cache, key)? else {
        return Ok(());
    };
    let Some(index) = entry.selectors.iter().position(|o| o.key == *key) else {
        return Ok(());
    };
    let Some(value) = p.get("record_id") else {
        return Ok(());
    };
    let record: OperationRecordId = serde_json::from_value(value.clone()).map_err(|_| bad(key))?;
    if entry
        .selectors
        .iter()
        .any(|o| o.key != *key && o.record.as_ref() == Some(&record))
    {
        return Err(bad(key));
    }
    let origin = &mut entry.selectors[index];
    if origin.record.as_ref().is_some_and(|old| old != &record) {
        return Err(bad(key));
    }
    origin.record = Some(record);
    origin.completed |= p["state"] == "completed";
    Ok(())
}
struct Journal {
    record: OperationRecordId,
    parent: OperationRecordId,
    expected: u64,
    state: OperationState,
}
fn journal(key: &OperationKey, p: &Value) -> Result<Journal> {
    let fail = || bad(key);
    if p["operation_id"].as_str() != Some(key.operation_id.as_str())
        || p["command_type"] != "stream.reconcile"
        || p["revision"]
            .as_u64()
            .is_none_or(|n| n > 9_007_199_254_740_991)
    {
        return Err(fail());
    }
    Ok(Journal {
        record: serde_json::from_value(p["record_id"].clone()).map_err(|_| fail())?,
        parent: serde_json::from_value(p["subscription_record_id"].clone()).map_err(|_| fail())?,
        expected: p["expected_revision"]
            .as_u64()
            .filter(|n| *n < 9_007_199_254_740_991)
            .ok_or_else(fail)?,
        state: serde_json::from_value(p["state"].clone()).map_err(|_| fail())?,
    })
}
fn identity(entry: &NativeConnection, key: &OperationKey, j: &Journal) -> Result<()> {
    if let Some(snapshot) = entry.reconciliation.as_ref().filter(|s| s.key == *key) {
        if snapshot.expected_revision != j.expected
            || snapshot.record_id.as_ref().is_some_and(|r| r != &j.record)
            || snapshot
                .subscription_record_id
                .as_ref()
                .is_some_and(|r| r != &j.parent)
        {
            return Err(bad(key));
        }
    }
    Ok(())
}
fn progress(
    cache: &std::sync::Mutex<NativeCache>,
    key: &OperationKey,
    phase: NativeReconciliationPhase,
    error: Option<ErrorKind>,
) {
    if let Ok(mut cache) = cache.lock() {
        if let Ok(Some(entry)) = exact(&mut cache, key) {
            if let Some(snapshot) = entry.reconciliation.as_mut().filter(|s| s.key == *key) {
                snapshot.phase = phase;
                snapshot.error = error;
            }
        }
    }
}
/// A restored read can settle a retained original preparation, but cannot
/// create preparation or acquire a different peer's native ownership.
pub(super) fn observe(
    cache: &std::sync::Mutex<NativeCache>,
    media: &media::Inbox,
    key: &OperationKey,
    p: &Value,
) -> Result<()> {
    let j = journal(key, p)?;
    let completed = if j.state == OperationState::Completed {
        if p["result"]["subscription_record_id"] != json!(j.parent) {
            return Err(bad(key));
        }
        let out = super::super::selections::parse(&p["result"], key)?;
        if out.expected_revision != j.expected {
            return Err(bad(key));
        }
        Some(out)
    } else {
        None
    };
    {
        let mut cache = cache.lock().map_err(|_| bad(key))?;
        let Some(entry) = exact(&mut cache, key)? else {
            return Ok(());
        };
        let Some(snapshot) = entry.reconciliation.as_ref().filter(|s| s.key == *key) else {
            return Ok(());
        };
        identity(entry, key, &j)?;
        if snapshot.record_id.is_none() || snapshot.subscription_record_id.is_none() {
            if !entry
                .selectors
                .iter()
                .any(|o| o.completed && o.record.as_ref() == Some(&j.parent))
            {
                return Err(bad(key));
            }
            let snapshot = entry.reconciliation.as_mut().expect("owned snapshot");
            snapshot.record_id = Some(j.record.clone());
            snapshot.subscription_record_id = Some(j.parent.clone());
        }
    }
    match j.state {
        OperationState::Completed => {
            let out = completed.as_ref().expect("completed result");
            selections::record(cache, media, key, out)?;
            let mut cache = cache.lock().map_err(|_| bad(key))?;
            if let Some(entry) = exact(&mut cache, key)? {
                let adopted = out.stream_bindings.as_ref().is_some_and(|bindings| {
                    bindings.len() == entry.selected.streams.len()
                        && bindings.iter().all(|b| {
                            codecs::matches(&entry.selected.streams, &entry.wire_handles, b)
                                && b.stream_id
                                    .parse::<StreamId>()
                                    .ok()
                                    .and_then(|id| entry.generations.get(&id))
                                    .copied()
                                    == Some(b.stream_generation)
                        })
                });
                if let Some(snapshot) = entry.reconciliation.as_mut().filter(|s| s.key == *key) {
                    snapshot.phase = NativeReconciliationPhase::Recorded;
                    snapshot.bindings_adopted = adopted;
                    snapshot.error = None;
                }
            }
        }
        OperationState::Failed | OperationState::Cancelled => {
            codecs::failed(cache, key)?;
            progress(cache, key, NativeReconciliationPhase::Refused, None);
        }
        _ => (),
    }
    Ok(())
}
struct Job {
    key: OperationKey,
    offer: Option<Envelope>,
    announced: Option<connect_protocol::native_streams::ReconciliationHint>,
    expected: u64,
    deadline: Instant,
    due: Instant,
    query: Option<crate::canonical::exchange::Exchange>,
    confirmed: bool,
    _slot: OwnedSemaphorePermit,
}
struct Hint {
    key: OperationKey,
    value: connect_protocol::native_streams::ReconciliationHint,
    at: Instant,
}
#[derive(Default)]
pub(super) struct Driver {
    jobs: Vec<Job>,
    retired: VecDeque<String>,
    hints: BTreeMap<ConnectionId, Hint>,
}
impl Driver {
    pub(super) fn busy(&self, key: &OperationKey) -> bool {
        self.jobs.iter().any(|j| j.key.scope == key.scope)
    }
    pub(super) fn wakeup(&self) -> Option<Instant> {
        self.jobs
            .iter()
            .map(|j| {
                j.deadline
                    .min(j.query.as_ref().map_or(j.due, |q| q.deadline))
            })
            .chain((!self.hints.is_empty()).then(|| Instant::now() + POLL))
            .min()
    }
    fn retire(
        &mut self,
        index: usize,
        cache: &std::sync::Mutex<NativeCache>,
        error: Option<ErrorKind>,
    ) {
        let job = self.jobs.swap_remove(index);
        if let Some(q) = job.query {
            if self.retired.len() == RETIRED_QUERIES {
                self.retired.pop_front();
            }
            self.retired.push_back(q.id);
        }
        if let Some(error) = error {
            progress(
                cache,
                &job.key,
                NativeReconciliationPhase::Unresolved,
                Some(error),
            );
        }
    }
    pub(super) async fn tick(
        &mut self,
        peer: &mut CanonicalClient,
        cache: &std::sync::Mutex<NativeCache>,
        slots: &Arc<Semaphore>,
    ) -> Result<()> {
        let now = Instant::now();
        for index in (0..self.jobs.len()).rev() {
            if now >= self.jobs[index].deadline
                || self.jobs[index]
                    .query
                    .as_ref()
                    .is_some_and(|q| now >= q.deadline)
            {
                self.retire(index, cache, Some(ErrorKind::Deadline));
            }
        }
        self.hints
            .retain(|_, hint| now.saturating_duration_since(hint.at) < Duration::from_secs(30));
        let ready = {
            let mut cache = cache
                .lock()
                .map_err(|_| Error::local(ErrorKind::Protocol))?;
            self.hints.iter().find_map(|(id, hint)| {
                let entry = exact(&mut cache, &hint.key).ok().flatten()?;
                (entry.prepared.is_none()
                    && !self.busy(&hint.key)
                    && entry.selectors.iter().any(|o| {
                        o.completed
                            && o.record.as_ref().map(OperationRecordId::as_str)
                                == Some(hint.value.subscription_record_id.as_str())
                    }))
                .then(|| id.clone())
            })
        };
        if let Some(id) = ready {
            if let Ok(slot) = slots.clone().try_acquire_owned() {
                let hint = self.hints.remove(&id).expect("pending hint");
                let mut cache = cache.lock().map_err(|_| bad(&hint.key))?;
                if let Some(entry) = exact(&mut cache, &hint.key)? {
                    if hint.value.stream_revision > entry.selected.stream_revision.get() {
                        entry.reconciliation = Some(NativeReconciliation {
                            key: hint.key.clone(),
                            expected_revision: hint.value.expected_revision,
                            record_id: None,
                            subscription_record_id: None,
                            phase: NativeReconciliationPhase::Authorizing,
                            bindings_adopted: false,
                            error: None,
                        });
                        self.jobs.push(Job {
                            key: hint.key,
                            expected: hint.value.expected_revision,
                            offer: None,
                            announced: Some(hint.value),
                            deadline: hint.at + Duration::from_secs(30),
                            due: now,
                            query: None,
                            confirmed: true,
                            _slot: slot,
                        });
                    }
                }
            }
        }
        if let Some(job) = self
            .jobs
            .iter_mut()
            .find(|j| j.query.is_none() && j.due <= now)
        {
            let mut query = tokio::time::timeout_at(
                job.deadline,
                peer.begin_exchange(
                    "operation.get",
                    json!({"operation_id":job.key.operation_id}),
                    job.key.scope.command(None),
                ),
            )
            .await
            .map_err(|_| Error::request(ErrorKind::Deadline, Some(&job.key), false))?
            .map_err(|e| Error::transport(e, Some(&job.key)))?;
            query.deadline = query.deadline.min(job.deadline);
            job.query = Some(query);
        }
        Ok(())
    }
    pub(super) fn hint(
        &mut self,
        cache: &std::sync::Mutex<NativeCache>,
        frame: &Envelope,
    ) -> Result<bool> {
        if frame.kind() != "operation.updated"
            || frame.scope(Scope::Reply).is_some()
            || frame.original()["payload"]["journal_hint"] != true
        {
            return Ok(false);
        }
        let value = match serde_json::from_value::<
            connect_protocol::native_streams::ReconciliationHint,
        >(frame.original()["payload"].clone())
        {
            Ok(value) if value.validate().is_ok() => value,
            _ => return Ok(true),
        };
        if frame.scope(Scope::Operation) != Some(value.operation_id.as_str())
            || frame.expected_revision().is_some()
        {
            return Ok(true);
        }
        let parse = || {
            Some(OperationKey {
                operation_id: value.operation_id.parse().ok()?,
                command: "stream.reconcile".into(),
                scope: OperationScope {
                    conversation_id: Some(frame.scope(Scope::Conversation)?.parse().ok()?),
                    session_id: Some(frame.scope(Scope::Session)?.parse().ok()?),
                    connection_id: Some(frame.scope(Scope::Connection)?.parse().ok()?),
                },
            })
        };
        let Some(key) = parse() else { return Ok(true) };
        let mut cache = cache.lock().map_err(|_| bad(&key))?;
        let Some(entry) = key
            .scope
            .connection_id
            .as_ref()
            .and_then(|id| cache.get_mut(id))
            .filter(|e| {
                !e.ended
                    && Some(&e.session) == key.scope.session_id.as_ref()
                    && Some(&e.conversation) == key.scope.conversation_id.as_ref()
            })
        else {
            return Ok(false);
        };
        if value.stream_revision <= entry.selected.stream_revision.get()
            || self.jobs.iter().any(|job| job.key == key)
        {
            return Ok(true);
        }
        if !entry.selectors.iter().any(|o| {
            o.completed
                && o.record.as_ref().map(OperationRecordId::as_str)
                    == Some(value.subscription_record_id.as_str())
        }) && !entry.selectors.iter().any(|o| !o.completed)
        {
            return Ok(true);
        }
        let id = key
            .scope
            .connection_id
            .as_ref()
            .expect("hint connection")
            .clone();
        if self
            .hints
            .get(&id)
            .is_some_and(|h| h.value.stream_revision >= value.stream_revision)
        {
            return Ok(true);
        }
        if self.hints.len() >= NATIVE_CONNECTION_CAPACITY && !self.hints.contains_key(&id) {
            return Ok(true);
        }
        self.hints.insert(
            id,
            Hint {
                key,
                value,
                at: Instant::now(),
            },
        );
        Ok(true)
    }
    pub(super) async fn offer(
        &mut self,
        peer: &mut CanonicalClient,
        cache: &std::sync::Mutex<NativeCache>,
        slots: &Arc<Semaphore>,
        frame: &Envelope,
    ) -> Result<bool> {
        if frame.kind() != "connection.offer"
            || frame.scope(Scope::Reply).is_some()
            || frame.expected_revision().is_none()
        {
            return Ok(false);
        }
        let parse = || {
            Some(OperationKey {
                operation_id: frame.scope(Scope::Operation)?.parse().ok()?,
                command: "stream.reconcile".into(),
                scope: OperationScope {
                    conversation_id: Some(frame.scope(Scope::Conversation)?.parse().ok()?),
                    session_id: Some(frame.scope(Scope::Session)?.parse().ok()?),
                    connection_id: Some(frame.scope(Scope::Connection)?.parse().ok()?),
                },
            })
        };
        let Some(key) = parse() else { return Ok(false) };
        let pending_preparation = {
            let mut cache = cache.lock().map_err(|_| bad(&key))?;
            // A callback for another scope is never an ownership lookup.
            let Some(entry) = key
                .scope
                .connection_id
                .as_ref()
                .and_then(|id| cache.get_mut(id))
                .filter(|e| {
                    !e.ended
                        && Some(&e.session) == key.scope.session_id.as_ref()
                        && Some(&e.conversation) == key.scope.conversation_id.as_ref()
                })
            else {
                return Ok(false);
            };
            if self.jobs.iter().any(|j| j.key == key)
                || entry.reconciliation.as_ref().is_some_and(|s| s.key == key)
            {
                return Ok(true);
            }
            entry
                .prepared
                .as_ref()
                .is_some_and(|prepared| !prepared.belongs_to(&key))
        };
        let permit = slots.clone().try_acquire_owned();
        if pending_preparation || self.busy(&key) || permit.is_err() {
            tokio::time::timeout(
                Duration::from_secs(1),
                peer.decline_connection_offer(
                    frame,
                    &key.scope.command(Some(&key.operation_id)),
                    429,
                ),
            )
            .await
            .map_err(|_| bad(&key))?
            .map_err(|e| Error::transport(e, Some(&key)))?;
            return Ok(true);
        }
        let now = Instant::now();
        let expected = frame.expected_revision().ok_or_else(|| bad(&key))?;
        {
            let mut cache = cache.lock().map_err(|_| bad(&key))?;
            let entry = exact(&mut cache, &key)?.ok_or_else(|| bad(&key))?;
            entry.reconciliation = Some(NativeReconciliation {
                key: key.clone(),
                expected_revision: expected,
                record_id: None,
                subscription_record_id: None,
                phase: NativeReconciliationPhase::Authorizing,
                bindings_adopted: false,
                error: None,
            });
        }
        self.jobs.push(Job {
            key,
            offer: Some(frame.clone()),
            announced: None,
            expected,
            deadline: now + Duration::from_secs(30),
            due: now,
            query: None,
            confirmed: false,
            _slot: permit.expect("checked slot"),
        });
        Ok(true)
    }
    pub(super) async fn frame(
        &mut self,
        peer: &mut CanonicalClient,
        cache: &std::sync::Mutex<NativeCache>,
        media: &media::Inbox,
        frame: &Envelope,
    ) -> Result<bool> {
        let Some(reply) = frame.scope(Scope::Reply) else {
            return Ok(false);
        };
        if self.retired.iter().any(|id| id == reply) {
            return Ok(true);
        }
        let Some(index) = self
            .jobs
            .iter()
            .position(|j| j.query.as_ref().is_some_and(|q| q.id == reply))
        else {
            return Ok(false);
        };
        if frame.kind() == "operation.updated"
            && journal(&self.jobs[index].key, &frame.original()["payload"]).is_err()
        {
            self.retire(index, cache, Some(ErrorKind::Protocol));
            return Ok(true);
        }
        let accepted = peer.accept_exchange_frame(
            self.jobs[index].query.as_mut().expect("query"),
            frame.clone(),
        );
        match accepted {
            Ok(false) => return Ok(true),
            Err(_) => {
                self.retire(index, cache, Some(ErrorKind::Protocol));
                return Ok(true);
            }
            Ok(true) => (),
        }
        let batch = self.jobs[index].query.take().expect("query").finish();
        let payload = if batch.ack().is_ok()
            && batch.frames.len() == 2
            && batch.frames[0].kind() == "operation.updated"
        {
            batch.frames[0].original()["payload"].clone()
        } else {
            self.retire(index, cache, Some(ErrorKind::Application));
            return Ok(true);
        };
        let result = self.complete(index, peer, cache, media, &payload).await;
        if let Err(error) = result {
            self.retire(index, cache, Some(error.kind));
        }
        Ok(true)
    }
    async fn complete(
        &mut self,
        index: usize,
        peer: &mut CanonicalClient,
        cache: &std::sync::Mutex<NativeCache>,
        media: &media::Inbox,
        p: &Value,
    ) -> Result<()> {
        let job = &mut self.jobs[index];
        let j = journal(&job.key, p)?;
        if j.expected != job.expected {
            return Err(bad(&job.key));
        }
        if let Some(hint) = &job.announced {
            if j.record.as_str() != hint.record_id
                || j.parent.as_str() != hint.subscription_record_id
                || j.state != OperationState::Completed
                || p["revision"].as_u64().is_none_or(|n| n < hint.revision)
                || p["result"]["stream_revision"].as_u64() != Some(hint.stream_revision)
                || p["result"]["resources_changed"] != false
            {
                return Err(bad(&job.key));
            }
        }

        let known = {
            let mut cache = cache.lock().map_err(|_| bad(&job.key))?;
            let entry = exact(&mut cache, &job.key)?.ok_or_else(|| bad(&job.key))?;
            identity(entry, &job.key, &j)?;
            let known = entry
                .selectors
                .iter()
                .any(|o| o.completed && o.record.as_ref() == Some(&j.parent));
            if !known && !entry.selectors.iter().any(|o| !o.completed) {
                return Err(bad(&job.key));
            }
            if let Some(snapshot) = entry.reconciliation.as_mut().filter(|s| s.key == job.key) {
                snapshot.record_id = Some(j.record.clone());
                snapshot.subscription_record_id = Some(j.parent.clone());
            }
            known
        };
        if known
            && matches!(
                j.state,
                OperationState::Completed | OperationState::Failed | OperationState::Cancelled
            )
        {
            observe(cache, media, &job.key, p)?;
            self.retire(index, cache, None);
            return Ok(());
        }
        if known && !job.confirmed && j.state == OperationState::Preparing {
            let offer = job.offer.as_ref().ok_or_else(|| bad(&job.key))?;
            let request_id = offer.original()["payload"]["substrate_setup"]["offer_id"]
                .as_str()
                .filter(|s| s.parse::<OperationId>().is_ok())
                .ok_or_else(|| bad(&job.key))?;
            let answer = selections::prepare(
                cache,
                &job.key,
                &Value::Null,
                job.expected,
                request_id,
                offer,
            );
            let root = job.key.scope.command(Some(&job.key.operation_id));
            match answer {
                Ok(answer) => {
                    tokio::time::timeout_at(
                        job.deadline,
                        peer.answer_connection_offer(offer, answer, &root),
                    )
                    .await
                    .map_err(|_| Error::request(ErrorKind::Deadline, Some(&job.key), true))?
                    .map_err(|e| Error::transport(e, Some(&job.key)))?;
                    job.confirmed = true;
                    progress(
                        cache,
                        &job.key,
                        NativeReconciliationPhase::AwaitingCompletion,
                        None,
                    );
                }
                Err(error) => {
                    tokio::time::timeout_at(
                        job.deadline,
                        peer.decline_connection_offer(offer, &root, 488),
                    )
                    .await
                    .map_err(|_| bad(&job.key))?
                    .map_err(|e| Error::transport(e, Some(&job.key)))?;
                    progress(
                        cache,
                        &job.key,
                        NativeReconciliationPhase::Refused,
                        Some(error.kind),
                    );
                    self.retire(index, cache, None);
                    return Ok(());
                }
            }
        }
        self.jobs[index].due = Instant::now() + POLL;
        Ok(())
    }
}
