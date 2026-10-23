//! Shared canonical read parsing and physically owned bounded subscriptions.
use connect_protocol::{
    events::{EventQuery, SnapshotQuery},
    Error, Result,
};
use connect_store::PeerSubscriptionCommand;
use rvoip_uctp::v2::{payloads::ConversationGet, Envelope, Payload, Scope};
pub(crate) fn assert_read_scopes(e: &Envelope) -> Result<()> {
    if [
        Scope::Session,
        Scope::Connection,
        Scope::Operation,
        Scope::Reply,
    ]
    .iter()
    .any(|scope| e.scope(*scope).is_some())
        || e.expected_revision().is_some()
    {
        return Err(invalid());
    }
    Ok(())
}
pub(crate) fn snapshot_query(input: &ConversationGet) -> Result<SnapshotQuery> {
    Ok(SnapshotQuery {
        page: input.page.clone(),
        limit: input
            .limit
            .map(u32::try_from)
            .transpose()
            .map_err(|_| invalid())?,
    })
}
pub(crate) fn command(e: &Envelope) -> Result<PeerSubscriptionCommand> {
    assert_read_scopes(e)?;
    let Payload::ConversationSubscribe(input) = e.payload() else {
        return Err(invalid());
    };
    let limit = u32::try_from(input.limit.unwrap_or(100)).map_err(|_| invalid())?;
    let lease = u32::try_from(input.lease_seconds.unwrap_or(30)).map_err(|_| invalid())?;
    if limit == 0
        || limit >= rvoip_uctp::v2::auth_dispatch::MAX_HANDLER_REPLIES as u32
        || lease == 0
        || lease > 30
    {
        return Err(invalid());
    }
    Ok(PeerSubscriptionCommand {
        actor_epoch: None,
        ceiling: None,
        conversation_id: e.scope(Scope::Conversation).ok_or_else(invalid)?.into(),
        query: EventQuery {
            after: input.after.clone(),
            limit: Some(
                limit.min((rvoip_uctp::v2::auth_dispatch::MAX_HANDLER_REPLIES as u32 - 1) / 2),
            ),
            lease: None,
            lease_seconds: Some(lease),
            wait_ms: 0,
        },
        live: input.live.unwrap_or(true),
    })
}
fn invalid() -> Error {
    Error::new(
        "invalid-request",
        "Invalid subscription or read resource assertion",
    )
}

use crate::{Service, StoreInvocationAuthority};
use connect_protocol::events::{EventPage, SnapshotPage};
use connect_store::{AuthorityWait, PeerSubscriptionSource};
use rvoip_uctp::v2::auth_dispatch::{
    AuthorityLease, AuthorizedEnvelope, DispatchError, HandlerDelivery, HandlerReplies,
    SubscriptionOrigin,
};
use rvoip_uctp::v2::server::{NativeTransportContext, PhysicalHandlerFactory};
use serde_json::json;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::{oneshot, Notify},
    time::Instant,
};
use tokio_util::sync::CancellationToken;
const MAX_SUBSCRIPTIONS: usize = 8;
struct Pending {
    complete: oneshot::Receiver<std::result::Result<(), DispatchError>>,
    terminal: bool,
}
struct Subscription {
    request: Envelope,
    origin: SubscriptionOrigin,
    source: PeerSubscriptionSource,
    ready: Instant,
    pending: Option<Pending>,
    receipt_observations: bool,
}
#[derive(Default)]
struct State {
    queue: VecDeque<Subscription>,
    origins: BTreeMap<String, SubscriptionOrigin>,
}
pub(crate) struct Subscriptions {
    state: Mutex<State>,
    changed: Notify,
    physical: CancellationToken,
}
impl Drop for Subscriptions {
    fn drop(&mut self) {
        self.physical.cancel();
        if let Ok(state) = self.state.lock() {
            for origin in state.origins.values() {
                origin.retire();
            }
        }
    }
}
pub(crate) struct Factory(pub Arc<Service>);
impl PhysicalHandlerFactory for Factory {
    fn handler(
        &self,
        context: NativeTransportContext,
    ) -> std::result::Result<
        Arc<dyn rvoip_uctp::v2::auth_dispatch::AuthenticatedHandler>,
        DispatchError,
    > {
        if context.lifetime.is_cancelled() {
            return Err(DispatchError::Closed);
        }
        let physical_lifetime = context.lifetime.child_token();
        let physical = self
            .0
            .canonical_runtime
            .get()
            .map(|runtime| runtime.physical(context))
            .transpose()
            .map_err(|_| DispatchError::Capacity)?;
        let handler: Arc<dyn rvoip_uctp::v2::auth_dispatch::AuthenticatedHandler> =
            Arc::new(crate::CanonicalHandler {
                physical: physical.clone(),
                service: self.0.clone(),
                subscriptions: Some(Arc::new(Subscriptions {
                    state: Mutex::new(State::default()),
                    changed: Notify::new(),
                    physical: physical_lifetime,
                })),
            });
        match physical {
            Some(physical) => physical.binding.with_data_handler(handler),
            None => Ok(handler),
        }
    }
}
fn cid(request: &Envelope) -> Result<String> {
    request
        .scope(Scope::Conversation)
        .map(str::to_owned)
        .ok_or_else(invalid)
}
fn snapshot(request: &Envelope, page: &SnapshotPage) -> Result<Envelope> {
    let mut data = json!({"revision":page.revision,"cursor":page.cursor,"items":page.items,"has_more":page.has_more});
    if let Some(next) = &page.next {
        data["next_page"] = json!(next);
    }
    super::canonical::reply(request, "conversation.snapshot", data)
}
pub(crate) fn frames(
    request: &Envelope,
    page: &EventPage,
    live: bool,
    expires: Option<i64>,
    receipt_observations: bool,
) -> Result<Vec<Envelope>> {
    let mut frames = vec![];
    if let Some(page) = &page.snapshot {
        frames.push(snapshot(request, page)?);
    }
    for event in &page.events {
        if matches!(
            &event.data,
            connect_protocol::events::JournalData::SessionParticipantJoined(_)
                | connect_protocol::events::JournalData::SessionParticipantLeft(_)
        ) {
            frames.push(crate::canonical_session_lifecycle::participant_hint(
                &event.data,
                Some(request.id()),
            )?);
        }
        if let connect_protocol::events::JournalData::SessionStarted(fact) = &event.data {
            frames.push(crate::canonical_session_lifecycle::started_hint(
                fact,
                Some(request.id()),
            )?);
        }
        if let connect_protocol::events::JournalData::ConversationClosed(fact) = &event.data {
            if event.kind != "conversation_closed" || fact.conversation_id != event.conversation_id
            {
                return Err(Error::storage());
            }
            let hint = json!({"v":2,"type":"conversation.closed","id":format!("closure_hint_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"in_reply_to":request.id(),"cid":fact.conversation_id,"payload":{"reason_code":fact.reason_code,"reason":fact.reason,"closed_at":chrono::DateTime::from_timestamp(fact.closed_at,0).ok_or_else(Error::storage)?}});
            frames.push(Envelope::from_value(hint).map_err(|_| Error::storage())?);
        }
        if let connect_protocol::events::JournalData::SessionEnded(fact) = &event.data {
            if fact.session.conversation_id != event.conversation_id
                || event.session_id.as_deref() != Some(fact.session.id.as_str())
                || event.connection_id.is_some()
                || fact.session.state != "ended"
            {
                return Err(Error::storage());
            }
            frames.push(crate::canonical_session_lifecycle::ended_hint(
                fact,
                Some(request.id()),
            )?);
        }
        if let connect_protocol::events::JournalData::Record(
            fact @ connect_protocol::events::RecordAvailability::Ready { .. },
        ) = &event.data
        {
            frames.push(crate::canonical_session_lifecycle::record_ready_hint(
                &event.conversation_id,
                fact,
                Some(request.id()),
            )?);
        }

        if receipt_observations {
            if let Some(observation) = crate::canonical_receipts::observation(request, event)? {
                frames.push(observation);
            }
        }
        let mut value = json!({"v":2,"type":"conversation.event","id":format!("event_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"in_reply_to":request.id(),"cid":event.conversation_id,"payload":{"event_id":event.event_id,"cursor":event.cursor,"kind":event.kind,"revision":event.revision,"data":event}});
        for (field, id) in [
            ("sid", &event.session_id),
            ("connid", &event.connection_id),
            ("operation_id", &event.operation_id),
        ] {
            if let Some(id) = id {
                value[field] = json!(id);
            }
        }
        frames.push(Envelope::from_value(value).map_err(|_| Error::storage())?);
    }
    // This typed summary also makes an empty catch-up or explicit resync visible.
    frames.push(super::canonical::reply(request,"ack",json!({"details":{"cursor":page.cursor,"has_more":page.has_more,"resync_required":page.resync_required,"lease":page.lease,"expires_at":expires.or(page.expires_at),"live":live}}))?);
    Ok(frames)
}

impl Service {
    pub(crate) async fn canonical_events_http(
        &self,
        request: &Envelope,
        credential: String,
    ) -> Result<Vec<Envelope>> {
        let command = command(request)?;
        // The bounded HTTP command response is a finite catch-up. A caller must
        // explicitly request it; never silently downgrade a live subscription.
        if command.live {
            return Err(Error::new(
                "unsupported-command",
                "Live subscriptions require the native event stream",
            ));
        }
        let store = self.store.clone();
        let (page, receipts) = tokio::task::spawn_blocking(move || {
            let actor = store.revalidate(&credential)?;
            let receipts = actor.permits("send") || actor.permits("history");
            let page =
                store.conversation_events(&credential, &command.conversation_id, command.query)?;
            Ok::<_, Error>((page, receipts))
        })
        .await
        .map_err(|_| Error::storage())??;
        frames(request, &page, false, None, receipts)
    }
}
impl Subscriptions {
    fn available(&self, cid: &str) -> Result<()> {
        if self.physical.is_cancelled() {
            return Err(Error::forbidden());
        }
        let mut state = self.state.lock().map_err(|_| Error::storage())?;
        state.origins.retain(|_, origin| origin.current());
        if !state.origins.contains_key(cid) && state.origins.len() >= MAX_SUBSCRIPTIONS {
            return Err(Error::new(
                "capacity-exceeded",
                "Physical subscription capacity exceeded",
            ));
        }
        Ok(())
    }
    fn register(
        &self,
        request: Envelope,
        origin: SubscriptionOrigin,
        source: PeerSubscriptionSource,
        receipt_observations: bool,
    ) -> Result<()> {
        let cid = cid(&request)?;
        self.available(&cid)?;
        let mut state = self.state.lock().map_err(|_| Error::storage())?;
        if !state.origins.contains_key(&cid) && state.origins.len() >= MAX_SUBSCRIPTIONS {
            return Err(Error::new(
                "capacity-exceeded",
                "Physical subscription capacity exceeded",
            ));
        }
        if let Some(old) = state.origins.insert(cid.clone(), origin.clone()) {
            old.retire();
        }
        state
            .queue
            .retain(|sub| sub.request.scope(Scope::Conversation) != Some(cid.as_str()));
        state.queue.push_back(Subscription {
            request,
            origin,
            source,
            ready: Instant::now(),
            pending: None,
            receipt_observations,
        });
        drop(state);
        self.changed.notify_one();
        Ok(())
    }
    fn put(&self, sub: Subscription, front: bool) -> std::result::Result<(), DispatchError> {
        if !sub.origin.current() {
            return Ok(());
        }
        let mut state = self.state.lock().map_err(|_| DispatchError::Closed)?;
        if front {
            state.queue.push_front(sub)
        } else {
            state.queue.push_back(sub)
        }
        Ok(())
    }
    fn retire(&self, sub: &Subscription) {
        sub.origin.retire();
        if let Ok(mut state) = self.state.lock() {
            // Replacement already retired this origin; never remove its successor.
            state.origins.retain(|_, origin| origin.current());
        }
    }
    pub(crate) async fn next(
        &self,
        service: &Arc<Service>,
    ) -> std::result::Result<HandlerDelivery, DispatchError> {
        loop {
            let wake = service.changed.notified();
            tokio::pin!(wake);
            wake.as_mut().enable();
            let registered = self.changed.notified();
            tokio::pin!(registered);
            registered.as_mut().enable();
            let (next, ready) = {
                let mut state = self.state.lock().map_err(|_| DispatchError::Closed)?;
                state.origins.retain(|_, origin| origin.current());
                state.queue.retain(|sub| sub.origin.current());
                let ready = state
                    .queue
                    .iter()
                    .map(|sub| sub.ready)
                    .min()
                    .unwrap_or(Instant::now() + Duration::from_secs(30));
                let index = state
                    .queue
                    .iter()
                    .position(|sub| sub.ready <= Instant::now());
                (index.and_then(|index| state.queue.remove(index)), ready)
            };
            let Some(mut sub) = next else {
                tokio::select! {biased;_=self.physical.cancelled()=>return Err(DispatchError::Closed),_=&mut registered=>{},_=&mut wake=>{
                    if let Ok(mut state)=self.state.lock(){for sub in &mut state.queue{sub.ready=Instant::now();}}
                },_=tokio::time::sleep_until(ready)=>{}}
                continue;
            };
            if !sub.origin.is_ready() {
                sub.ready = Instant::now() + Duration::from_millis(5);
                self.put(sub, false)?;
                continue;
            }
            if let Some(pending) = sub.pending.take() {
                let result = tokio::select! {biased;_=self.physical.cancelled()=>return Err(DispatchError::Closed),_=sub.origin.retired()=>Err(DispatchError::Closed),_=tokio::time::sleep_until(sub.origin.terminal_deadline())=>Err(DispatchError::Closed),result=pending.complete=>result.unwrap_or(Err(DispatchError::Closed))};
                if result.is_err() || pending.terminal {
                    self.retire(&sub);
                    continue;
                }
                sub.ready = Instant::now();
                self.put(sub, false)?;
                continue;
            }
            let store = service.store.clone();
            let source = sub.source.clone();
            let wait = AuthorityWait::new(Duration::from_secs(1));
            let _cancel = wait.cancel_on_drop();
            let read =
                tokio::task::spawn_blocking(move || store.read_peer_subscription(&source, &wait));
            let read = tokio::select! {biased;_=self.physical.cancelled()=>return Err(DispatchError::Closed),_=sub.origin.retired()=>None,_=tokio::time::sleep_until(sub.origin.terminal_deadline())=>None,result=read=>Some(result.map_err(|_|DispatchError::StorageUnavailable)?)};
            let Some(read) = read else {
                self.retire(&sub);
                continue;
            };
            let read = match read {
                Ok(read) => read,
                Err(error)
                    if matches!(
                        error.code.as_str(),
                        "authority-busy" | "authority-unavailable"
                    ) =>
                {
                    sub.ready = Instant::now() + Duration::from_millis(50);
                    self.put(sub, false)?;
                    continue;
                }
                Err(error)
                    if matches!(
                        error.code.as_str(),
                        "forbidden" | "unauthorized" | "resync-required"
                    ) =>
                {
                    self.retire(&sub);
                    continue;
                }
                Err(_) => return Err(DispatchError::StorageUnavailable),
            };
            let empty = read.page.events.is_empty()
                && read.page.snapshot.is_none()
                && !read.page.resync_required
                && !read.page.has_more;
            sub.source = read.next;
            if empty {
                // An empty selected page may skip only hidden rows. It owns no
                // delivery grant while waiting; a wake was registered before SQL.
                drop(read.proof);
                sub.ready = (Instant::now() + Duration::from_secs(1))
                    .min(utc_deadline(sub.source.expires_at()));
                self.put(sub, false)?;
                // Preserve a journal wake that happened during the read.
                if tokio::time::timeout(Duration::ZERO, &mut wake)
                    .await
                    .is_ok()
                {
                    if let Ok(mut state) = self.state.lock() {
                        for sub in &mut state.queue {
                            sub.ready = Instant::now();
                        }
                    }
                }
                continue;
            }
            let proof = Arc::new(read.proof);
            let current = proof.clone();
            let origin = sub.origin.clone();
            let physical = self.physical.clone();
            let end = utc_deadline(proof.expires_at()).min(sub.origin.terminal_deadline());
            let lease = AuthorityLease::checking(proof, end, move || {
                current.current() && origin.current() && !physical.is_cancelled()
            });
            let frames = frames(
                &sub.request,
                &read.page,
                !read.terminal,
                Some(sub.source.expires_at()),
                sub.receipt_observations,
            )
            .map_err(|_| DispatchError::Closed)?;
            let (delivery, complete) =
                HandlerDelivery::new(&sub.origin, HandlerReplies::new(frames)?, lease)?;
            sub.pending = Some(Pending {
                complete,
                terminal: read.terminal,
            });
            sub.ready = Instant::now();
            self.put(sub, true)?;
            return Ok(delivery);
        }
    }
}
fn utc_deadline(expires: i64) -> Instant {
    let now = chrono::Utc::now();
    Instant::now()
        + chrono::DateTime::from_timestamp(expires, 0)
            .and_then(|date| date.signed_duration_since(now).to_std().ok())
            .unwrap_or(Duration::ZERO)
}
impl crate::CanonicalHandler {
    /// Every accepted physical peer receives its own bounded source registry.
    pub fn physical_factory(service: Arc<Service>) -> Arc<dyn PhysicalHandlerFactory> {
        Arc::new(Factory(service))
    }
    pub(crate) async fn subscribe(
        &self,
        authorized: &AuthorizedEnvelope,
        credential: String,
        authority: &StoreInvocationAuthority,
    ) -> Result<Vec<Envelope>> {
        let request = authorized.envelope();
        let command = command(request)?;
        let origin = if command.live {
            let subscriptions = self.subscriptions.as_ref().ok_or_else(|| {
                Error::new(
                    "unsupported-command",
                    "Live subscription requires physical composition",
                )
            })?;
            subscriptions.available(&command.conversation_id)?;
            Some(
                authorized
                    .subscription_origin(Duration::from_secs(u64::from(
                        command.query.lease_seconds.unwrap_or(30),
                    )))
                    .map_err(|_| Error::forbidden())?,
            )
        } else {
            None
        };
        let store = self.service.store.clone();
        let held = authority.retained();
        let start = command.clone();
        let (page, source) = tokio::task::spawn_blocking(move || {
            store.start_peer_subscription(
                &credential,
                &start,
                held.peer_guard().ok_or_else(Error::forbidden)?,
            )
        })
        .await
        .map_err(|_| Error::storage())??;
        let expires = source.as_ref().map(PeerSubscriptionSource::expires_at);
        let receipt_observations = authorized
            .principal()
            .has_scope(rvoip_uctp::v2::scope_policy::DATA);
        let output = frames(request, &page, command.live, expires, receipt_observations)?;
        // Bound the initial output before publishing a registration. The actor
        // completes this handler and queues its initial boundary before polling
        // the newly awakened delivery future.
        HandlerReplies::new(output.clone()).map_err(|_| {
            Error::new(
                "capacity-exceeded",
                "Subscription reply exceeds transport budget",
            )
        })?;
        if let (Some(origin), Some(source)) = (origin, source) {
            self.subscriptions
                .as_ref()
                .ok_or_else(Error::forbidden)?
                .register(request.clone(), origin, source, receipt_observations)?;
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn envelope(payload: serde_json::Value) -> serde_json::Value {
        json!({"v":2,"type":"conversation.subscribe","id":"subscription-validation","cid":"room","ts":chrono::Utc::now(),"payload":payload})
    }
    #[test]
    fn subscription_parser_bounds_batch_lease_and_rejects_extra_resource_assertions() {
        let value = envelope(json!({}));
        let parsed = command(&Envelope::from_value(value.clone()).unwrap()).unwrap();
        assert!(parsed.live);
        assert_eq!(parsed.query.limit, Some(63));
        assert_eq!(parsed.query.lease_seconds, Some(30));
        for payload in [
            json!({"limit":0}),
            json!({"limit":128}),
            json!({"limit":-1}),
            json!({"lease_seconds":0}),
            json!({"lease_seconds":31}),
            json!({"live":"true"}),
        ] {
            if let Ok(e) = Envelope::from_value(envelope(payload)) {
                assert!(command(&e).is_err());
            }
        }
        for field in [
            "sid",
            "connid",
            "operation_id",
            "in_reply_to",
            "expected_revision",
        ] {
            let mut bad = value.clone();
            bad[field] = if field == "expected_revision" {
                json!(1)
            } else {
                json!("unrelated")
            };
            if let Ok(e) = Envelope::from_value(bad) {
                assert!(command(&e).is_err(), "{field}");
            }
        }
        let e = Envelope::from_value(envelope(
            json!({"live":false,"limit":127,"lease_seconds":1}),
        ))
        .unwrap();
        let parsed = command(&e).unwrap();
        assert!(!parsed.live);
        assert_eq!(parsed.query.limit, Some(63));
    }
    #[test]
    fn receipt_hints_precede_cursor_facts_within_batch_budget_and_require_data_floor() {
        use connect_protocol::{events::*, receipts::*};
        let request = Envelope::from_value(envelope(json!({"live":false,"limit":127}))).unwrap();
        let events = (0..63)
            .map(|i| JournalEvent {
                event_id: format!("fact-{i}"),
                cursor: format!("cursor-{i}"),
                conversation_id: "room".into(),
                session_id: None,
                connection_id: None,
                participant_id: Some("recipient".into()),
                operation_id: None,
                kind: "message_receipt".into(),
                revision: 1,
                source: EventSource {
                    kind: "message".into(),
                    correlation_id: format!("message-{i}"),
                },
                observed_at: 100,
                audience: vec![connect_protocol::messages::MessageRecipient {
                    participant_id: "recipient".into(),
                    membership_epoch: 1,
                }],
                data: JournalData::Receipt(RecipientReceipt {
                    conversation_id: "room".into(),
                    message_id: format!("message-{i}"),
                    participant_id: "recipient".into(),
                    membership_epoch: 1,
                    channel: "native".into(),
                    delivery_id: "recipient".into(),
                    state: DeliveryState::Read,
                    provenance: "client_reported".into(),
                    observed_at: 100,
                }),
            })
            .collect::<Vec<_>>();
        let page = EventPage {
            conversation_id: "room".into(),
            events,
            cursor: "cursor-62".into(),
            has_more: true,
            lease: Some("lease".into()),
            expires_at: Some(130),
            resync_required: false,
            snapshot: None,
        };
        let output = frames(&request, &page, false, None, true).unwrap();
        assert_eq!(output.len(), 127);
        for pair in output[..126].chunks_exact(2) {
            assert_eq!(pair[0].kind(), "message.read");
            assert_eq!(pair[1].kind(), "conversation.event");
            assert_eq!(
                pair[0].original()["payload"]["evidence"]["event_id"],
                pair[1].original()["payload"]["event_id"]
            );
            assert_eq!(pair[0].scope(Scope::Reply), Some(request.id()));
        }
        assert_eq!(output[126].kind(), "ack");
        assert_eq!(
            output[126].original()["payload"]["details"]["cursor"],
            "cursor-62"
        );
        let generic = frames(&request, &page, false, None, false).unwrap();
        assert_eq!(generic.len(), 64);
        assert!(generic
            .iter()
            .all(|frame| matches!(frame.kind(), "conversation.event" | "ack")));
        let mut event = page.events[0].clone();
        let JournalData::Receipt(receipt) = &mut event.data else {
            panic!()
        };
        receipt.channel = "provider".into();
        receipt.provenance = "provider_observation".into();
        assert!(crate::canonical_receipts::observation(&request, &event)
            .unwrap()
            .is_none());
        let JournalData::Receipt(receipt) = &mut event.data else {
            panic!()
        };
        receipt.state = DeliveryState::Delivered;
        assert_eq!(
            crate::canonical_receipts::observation(&request, &event)
                .unwrap()
                .unwrap()
                .kind(),
            "message.delivered"
        );
    }
    #[test]
    fn record_ready_hints_preserve_session_scope_and_fit_full_subscription_budget() {
        use connect_protocol::events::*;
        let request = Envelope::from_value(envelope(json!({"live":false,"limit":127}))).unwrap();
        let events = (0..63)
            .map(|i| JournalEvent {
                event_id: format!("record-fact-{i}"),
                cursor: format!("cursor-{i}"),
                conversation_id: "room".into(),
                session_id: Some(format!("session-{i}")),
                connection_id: None,
                participant_id: None,
                operation_id: None,
                kind: "record_ready".into(),
                revision: 2,
                source: EventSource {
                    kind: "connect-record-outcome".into(),
                    correlation_id: format!("record-{i}"),
                },
                observed_at: 100,
                audience: vec![],
                data: JournalData::Record(RecordAvailability::Ready {
                    session_id: format!("session-{i}"),
                    uuid: uuid::Uuid::new_v4().to_string(),
                    revision: 1,
                }),
            })
            .collect();
        let page = EventPage {
            conversation_id: "room".into(),
            events,
            cursor: "cursor-62".into(),
            has_more: true,
            lease: None,
            expires_at: None,
            resync_required: false,
            snapshot: None,
        };
        let output = frames(&request, &page, false, None, false).unwrap();
        assert_eq!(output.len(), 127);
        for (i, pair) in output[..126].chunks_exact(2).enumerate() {
            assert_eq!(pair[0].kind(), "recording.vcon-ready");
            assert_eq!(pair[1].kind(), "conversation.event");
            assert_eq!(
                pair[0].scope(Scope::Session),
                Some(format!("session-{i}").as_str())
            );
            assert_eq!(pair[0].scope(Scope::Reply), Some(request.id()));
            let handle = pair[0].original()["payload"]["vcon_handle"]
                .as_object()
                .unwrap();
            assert_eq!(handle.len(), 2);
            assert_eq!(handle["revision"], 1);
            assert_eq!(pair[1].original()["payload"]["revision"], 2);
        }
        assert_eq!(output[126].kind(), "ack");
    }
}
