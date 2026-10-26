//! One bounded owner multiplexes signed exchanges despite caller cancellation.
use super::*;
mod codecs;
mod controls;
use crate::canonical::{CanonicalClient, CommandScope, ReplyBatch};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::{mpsc, oneshot, Mutex, OwnedSemaphorePermit, Semaphore};
const REQUEST_CAPACITY: usize = 16;
const NATIVE_CONNECTION_CAPACITY: usize = 64;
struct NativeConnection {
    session: SessionId,
    conversation: ConversationId,
    selected: NativeNegotiation,
    ended: bool,
    known_revision: StreamRevision,
    participant: ParticipantId,
    wire_handles: std::collections::BTreeMap<StreamId, u16>,
    local_capabilities: Value,
    server_capabilities: Value,
    reserved_handles: std::collections::BTreeSet<u16>,
    generations: std::collections::BTreeMap<StreamId, u64>,
    prepared: Option<codecs::Prepared>,
    gates: std::collections::BTreeMap<StreamId, controls::LocalGate>,
}
// Keep bounded retired metadata so a later offer cannot reuse a peer hop handle.
type NativeCache = std::collections::BTreeMap<ConnectionId, NativeConnection>;
pub(super) struct Shared {
    requests: mpsc::Sender<Command>,
    stop: CancellationToken,
    pub(super) closed: Arc<AtomicBool>,
    slots: Arc<Semaphore>,
    native: Arc<std::sync::Mutex<NativeCache>>,
    media: Arc<media::Inbox>,
    media_requests: mpsc::Sender<MediaCommand>,
    task: Mutex<Option<tokio::task::JoinHandle<crate::Result<()>>>>,
}
struct MediaCommand {
    key: streams::StreamKey,
    packet: MediaPacket,
    expires: tokio::time::Instant,
    signal: CancellationToken,
    reply: oneshot::Sender<Result<LocalMediaSubmission>>,
}
struct InFlight {
    exchange: crate::canonical::exchange::Exchange,
    key: Option<OperationKey>,
    reply: Option<oneshot::Sender<Result<ReplyBatch>>>,
    native: Option<native_negotiation::Pending>,
    control: Option<(Value, u64)>,
    _slot: OwnedSemaphorePermit,
}
struct Command {
    slot: OwnedSemaphorePermit,
    native: bool,
    kind: String,
    payload: Value,
    scope: CommandScope,
    key: Option<OperationKey>,
    expires: tokio::time::Instant,
    signal: CancellationToken,
    reply: oneshot::Sender<Result<ReplyBatch>>,
}
impl Drop for Shared {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
impl Shared {
    pub(super) fn start(mut peer: CanonicalClient) -> Result<Arc<Self>> {
        peer.start_signaling_reader()
            .map_err(|e| Error::transport(e, None))?;
        let (media_requests, mut media_receiver) =
            mpsc::channel::<MediaCommand>(media::SEND_CAPACITY);
        let media = Arc::new(media::Inbox::default());
        let owned_media = media.clone();
        let (requests, mut receiver) = mpsc::channel::<Command>(REQUEST_CAPACITY);
        let stop = CancellationToken::new();
        let stopping = stop.clone();
        let slots = Arc::new(Semaphore::new(REQUEST_CAPACITY));
        let owned_slots = slots.clone();
        let closed = Arc::new(AtomicBool::new(false));
        let owned_closed = closed.clone();
        let native = Arc::new(std::sync::Mutex::new(NativeCache::new()));
        let owned_native = native.clone();
        let task = tokio::spawn(async move {
            let mut active: Vec<InFlight> = Vec::new();
            let failure = loop {
                let next_deadline = active
                    .iter()
                    .map(|pending| pending.exchange.deadline)
                    .min()
                    .unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(60));
                if stopping.is_cancelled() {
                    break ErrorKind::Cancelled;
                }
                if !active.is_empty() && tokio::time::Instant::now() >= next_deadline {
                    break ErrorKind::Deadline;
                }
                // Fair selection keeps both packet directions and control live.
                tokio::select! {
                    _=stopping.cancelled()=>break ErrorKind::Cancelled,
                    _=tokio::time::sleep_until(next_deadline), if !active.is_empty()=>break ErrorKind::Deadline,
                    frame=peer.receive_owned_frame()=> {
                        let frame = match frame { Ok(frame)=>frame, Err(error)=>break Error::transport(error,None).kind };
                        if matches!(frame.kind(), "auth.bye" | "identity.assurance-changed") {
                            break ErrorKind::Authentication;
                        }
                        if frame.kind()=="connection.offer" {
                            if let Some(index)=active.iter().position(|p|p.native.is_some() && p.key.as_ref().and_then(|k|k.scope.connection_id.as_ref()).map(|v|v.as_str())==frame.scope(crate::canonical::Scope::Connection)) {
                                let until=active[index].exchange.deadline;
                                let result=tokio::select! { biased;
                                    _=stopping.cancelled()=>break ErrorKind::Cancelled,
                                    result=tokio::time::timeout_at(until,confirm_native(&mut peer,&active[index],&frame,&owned_native))=>result
                                };
                                let (selected,(participant,local_capabilities,server_capabilities,wire_handles))=match result {Ok(Ok(selected))=>selected,Ok(Err(error))=>break error.kind,Err(_)=>break ErrorKind::Deadline};
                                let pending=active.swap_remove(index);
                                let key=pending.key.expect("native key");
                                let mut cache=match owned_native.lock(){Ok(cache)=>cache,Err(_)=>break ErrorKind::Protocol};
                                let cid=key.scope.conversation_id.expect("native conversation");let sid=key.scope.session_id.expect("native session");let conn=key.scope.connection_id.expect("native connection");
                                if let Err(error)=owned_media.install(&cid,&sid,&conn,&selected){break error.kind;}
                                let reserved_handles=wire_handles.values().copied().collect();
                                let generations=selected.streams.iter().map(|s|(s.id.clone(),1)).collect();
                                let known_revision=selected.stream_revision;
                                cache.insert(conn,NativeConnection{session:sid,conversation:cid,selected,ended:false,known_revision,participant,wire_handles,local_capabilities,server_capabilities,reserved_handles,generations,prepared:None,gates:Default::default()});
                                continue;
                            }
                        }
                        if frame.kind()=="connection.offer" {
                            let is_codec=|p:&InFlight|p.control.as_ref().is_some_and(|(intent,_)|intent["action"]=="codec-renegotiate") && p.key.as_ref().is_some_and(|k|frame.scope(crate::canonical::Scope::Operation)==Some(k.operation_id.as_str()));
                            let exact=active.iter().position(|p|is_codec(p) && p.key.as_ref().and_then(|k|k.scope.connection_id.as_ref()).map(ConnectionId::as_str)==frame.scope(crate::canonical::Scope::Connection));
                            let index=exact.or_else(||{let mut ids=active.iter().enumerate().filter(|(_,p)|is_codec(p));let first=ids.next().map(|(i,_)|i);if ids.next().is_none(){first}else{None}});
                            if let Some(index)=index {
                                let pending=&active[index];let key=pending.key.as_ref().expect("codec key");let (intent,expected)=pending.control.as_ref().expect("codec intent");
                                let answer=match codecs::prepare(&owned_native,key,intent,*expected,&pending.exchange.id,&frame){Ok(v)=>v,Err(e)=>break e.kind};
                                let root=key.scope.command(Some(&key.operation_id));let until=pending.exchange.deadline;
                                let result=tokio::select! {biased;_=stopping.cancelled()=>break ErrorKind::Cancelled,r=tokio::time::timeout_at(until,peer.answer_connection_offer(&frame,answer,&root))=>r};
                                match result {Ok(Ok(_))=>(),Ok(Err(error))=>break Error::transport(error,Some(key)).kind,Err(_)=>break ErrorKind::Deadline};
                                continue;
                            }
                        }
                        let index = active.iter().position(|pending| frame.scope(crate::canonical::Scope::Reply) == Some(pending.exchange.id.as_str()));
                        if let Some(index) = index {
                            let native_answer=active[index].native.is_some() && frame.kind()=="connection.answer";
                            if native_answer {
                                let pending=&mut active[index];
                                if let Err(error)=pending.native.as_mut().expect("native pending").answer(&frame,&pending.exchange.id){break error.kind;}
                            }
                            let native_frame=native_answer.then(||frame.clone());
                            let ending = active[index].key.as_ref()
                                .filter(|k| k.command=="connection.end" && frame.kind()=="operation.updated")
                                .map(|k| (k.clone(),frame.clone()));
                            let updating=active[index].control.as_ref().zip(active[index].key.as_ref())
                                .filter(|_|frame.kind()=="operation.updated")
                                .map(|((intent,expected),key)|(key.clone(),intent.clone(),*expected,frame.clone()));
                            let accepted=peer.accept_exchange_frame(&mut active[index].exchange,frame);
                            if accepted.is_ok() {
                                if let Some((key,frame))=ending {
                                    if let Err(error)=observe_end(&owned_native,&owned_media,&key,&frame){break error.kind;}
                                }
                                if let Some((key,intent,expected,frame))=updating {
                                    if let Err(error)=controls::observe(&owned_native,&owned_media,&key,&intent,expected,&frame){break error.kind;}
                                }
                            }
                            match accepted {
                                Ok(true)=> {
                                    let pending=active.swap_remove(index);
                                    if let Some(reply)=pending.reply {let _=reply.send(Ok(pending.exchange.finish()));}
                                }
                                Ok(false)=>{
                                    if let Some(frame)=native_frame {
                                        let pending=&mut active[index];
                                        if let Some(reply)=pending.reply.take(){let _=reply.send(Ok(ReplyBatch{request_id:pending.exchange.id.clone(),frames:vec![frame]}));}
                                    }
                                },
                                Err(error)=>break Error::transport(error,None).kind,
                            }
                        } else if let Err(error)=peer.retain_unmatched_frame(frame) {
                            break Error::transport(error,None).kind;
                        }
                    }
                    datagram=peer.receive_owned_native_datagram()=> {
                        match datagram {
                            Ok(bytes)=>if let Err(error)=owned_media.push(bytes){break error.kind;},
                            Err(crate::ClientError::Protocol)=>if let Err(error)=owned_media.malformed(){break error.kind;},
                            Err(error)=>break Error::transport(peer.with_owned_signaling_cause(error),None).kind,
                        }
                    }
                    command=media_receiver.recv()=> {
                        let Some(command)=command else{break ErrorKind::Cancelled;};
                        if command.reply.is_closed() || command.signal.is_cancelled() || stopping.is_cancelled() || tokio::time::Instant::now()>=command.expires {
                            let kind=if tokio::time::Instant::now()>=command.expires{ErrorKind::Deadline}else{ErrorKind::Cancelled};
                            let _=command.reply.send(Err(Error::media(kind,false)));continue;
                        }
                        let result=(|| {
                            let binding=native_binding(&owned_native,&command.key)?;
                            let (bytes,datagram_sequence)=media::encode(&binding,&command.packet)?;
                            peer.send_native_media_datagram(&bytes).map_err(|e| {
                                let kind=Error::transport(e,None).kind;
                                Error::media(kind,!matches!(kind,ErrorKind::Protocol|ErrorKind::Validation|ErrorKind::Configuration))
                            })?;
                            Ok(LocalMediaSubmission{datagram_sequence})
                        })();
                        let _=command.reply.send(result);
                    }
                    command=receiver.recv()=> {
                        let command=match command {Some(command)=>command,None=>break ErrorKind::Cancelled};
                        // Cancellation/deadline while queued never becomes a write.
                        if command.reply.is_closed() || command.signal.is_cancelled()
                            || tokio::time::Instant::now() >= command.expires {
                            let kind=if tokio::time::Instant::now() >= command.expires {ErrorKind::Deadline}else{ErrorKind::Cancelled};
                            let _=command.reply.send(Err(Error::request(kind,command.key.as_ref(),false)));
                            continue;
                        }
                        if command.kind=="connection.update" && command.payload["action"]=="codec-renegotiate" {
                            let busy=match owned_native.lock(){Ok(cache)=>command.key.as_ref().is_some_and(|key|key.scope.connection_id.as_ref().and_then(|id|cache.get(id)).and_then(|entry|entry.prepared.as_ref()).is_some_and(|p|!p.belongs_to(key))),Err(_)=>break ErrorKind::Protocol};
                            if busy {let _=command.reply.send(Err(Error::request(ErrorKind::Conflict,command.key.as_ref(),false)));continue;}
                        }
                        if command.native {
                            let count=match owned_native.lock(){Ok(cache)=>cache.len(),Err(_)=>break ErrorKind::Protocol};
                            if count+active.iter().filter(|p|p.native.is_some()).count()>=NATIVE_CONNECTION_CAPACITY {
                                let _=command.reply.send(Err(Error::request(ErrorKind::Configuration,command.key.as_ref(),false)));
                                continue;
                            }
                        }
                        let negotiation=command.native.then(||native_negotiation::Pending::new(command.payload.clone(),command.scope.clone()));
                        let control=if command.kind=="connection.update" && (connect_protocol::native_gates::is_gate(&command.payload) || command.payload["action"]=="quality" || command.payload["action"]=="codec-renegotiate") {
                            command.scope.expected_revision.map(|revision|(command.payload.clone(),revision))
                        } else {None};
                        // Only explicit close may interrupt a started write. Local caller
                        // cancellation leaves its correlation/limits/deadline owned here.
                        let result=tokio::select! { biased;
                            _=stopping.cancelled()=> {
                                let _=command.reply.send(Err(Error::request(ErrorKind::Cancelled,command.key.as_ref(),true)));
                                break ErrorKind::Cancelled;
                            },
                            result=peer.begin_exchange(&command.kind,command.payload,command.scope)=>result
                        };
                        match result {
                            Ok(exchange)=>active.push(InFlight {exchange,key:command.key,reply:Some(command.reply),native:negotiation,control,_slot:command.slot}),
                            Err(error)=> {
                                let error=Error::transport(peer.with_owned_signaling_cause(error),command.key.as_ref());
                                let kind=error.kind;
                                let _=command.reply.send(Err(error));
                                break kind;
                            }
                        }
                    }
                }
                tokio::task::consume_budget().await;
            };
            owned_closed.store(true, Ordering::Release);
            owned_media.end();
            media_receiver.close();
            while let Some(command) = media_receiver.recv().await {
                let _ = command.reply.send(Err(Error::media(failure, false)));
            }
            owned_slots.close();
            // Retire the physical peer on an incomplete exchange, preserving all
            // submitted logical IDs. Unwritten queued commands remain unsubmitted.
            peer.close();
            for pending in active {
                if let Some(reply) = pending.reply {
                    let _ = reply.send(Err(Error::request(failure, pending.key.as_ref(), true)));
                }
            }
            receiver.close();
            while let Some(command) = receiver.recv().await {
                let _ =
                    command
                        .reply
                        .send(Err(Error::request(failure, command.key.as_ref(), false)));
            }
            peer.shutdown().await
        });
        Ok(Arc::new(Self {
            requests,
            stop,
            closed,
            slots,
            native,
            media,
            media_requests,
            task: Mutex::new(Some(task)),
        }))
    }
    pub(super) fn media_stats(&self) -> Result<MediaStats> {
        self.media.stats()
    }
    pub(super) async fn receive_media(
        &self,
        key: &streams::StreamKey,
        options: RequestOptions,
    ) -> Result<MediaPacket> {
        options.validate()?;
        if self.closed.load(Ordering::Acquire) {
            return Err(Error::request(ErrorKind::Cancelled, None, false));
        }
        let binding =
            native_binding(&self.native, key).map_err(|e| Error::request(e.kind, None, false))?;
        if binding.stream.stream_local_id.is_none()
            || !matches!(
                binding.stream.direction,
                Direction::Recvonly | Direction::Sendrecv
            )
        {
            return Err(Error::request(ErrorKind::Unsupported, None, false));
        }
        self.media.receive(key, options).await
    }
    pub(super) async fn send_media(
        &self,
        key: streams::StreamKey,
        packet: MediaPacket,
        options: RequestOptions,
    ) -> Result<LocalMediaSubmission> {
        options.validate()?;
        if self.closed.load(Ordering::Acquire) || options.signal.is_cancelled() {
            return Err(Error::media(ErrorKind::Cancelled, false));
        }
        // Validate all retained packet/report bytes before entering the bounded queue.
        let binding = native_binding(&self.native, &key)?;
        media::encode(&binding, &packet)?;
        let expires = tokio::time::Instant::now() + options.timeout;
        let permit = tokio::select! {biased;
            _=options.signal.cancelled()=>return Err(Error::media(ErrorKind::Cancelled,false)),
            result=tokio::time::timeout_at(expires,self.media_requests.reserve())=>result.map_err(|_|Error::media(ErrorKind::Deadline,false))?.map_err(|_|Error::media(ErrorKind::Cancelled,false))?
        };
        let (reply, response) = oneshot::channel();
        permit.send(MediaCommand {
            key,
            packet,
            expires,
            signal: options.signal.clone(),
            reply,
        });
        tokio::select! {biased;
            _=options.signal.cancelled()=>Err(Error::media(ErrorKind::Cancelled,true)),
            result=tokio::time::timeout_at(expires,response)=>result.map_err(|_|Error::media(ErrorKind::Deadline,true))?.map_err(|_|Error::media(ErrorKind::TransportUncertain,true))?
        }
    }
    pub(super) fn native_negotiation(
        &self,
        id: &ConnectionId,
        sid: &SessionId,
        cid: &ConversationId,
    ) -> Result<Option<NativeNegotiation>> {
        if self.closed.load(Ordering::Acquire) {
            return Err(Error::local(ErrorKind::Cancelled));
        }
        let cache = self
            .native
            .lock()
            .map_err(|_| Error::local(ErrorKind::Protocol))?;
        match cache.get(id) {
            Some(entry) if &entry.session == sid && &entry.conversation == cid => {
                Ok((!entry.ended).then(|| entry.selected.clone()))
            }
            Some(_) => Err(Error::local(ErrorKind::Protocol)),
            None => Ok(None),
        }
    }
    pub(super) fn retire_connection(&self, key: &OperationKey) -> Result<()> {
        retire_native(&self.native, &self.media, key)
    }
    pub(super) fn record_connection_update(
        &self,
        key: &OperationKey,
        observed: &connection_updates::Observed,
    ) -> Result<()> {
        controls::record(&self.native, &self.media, key, observed)
    }
    pub(super) fn fail_connection_update(&self, key: &OperationKey) -> Result<()> {
        codecs::failed(&self.native, key)
    }
    pub(super) async fn request(
        &self,
        kind: &str,
        payload: Value,
        scope: CommandScope,
        key: Option<&OperationKey>,
        options: RequestOptions,
    ) -> Result<ReplyBatch> {
        self.request_inner(false, kind, payload, scope, key, options)
            .await
    }
    pub(super) async fn native_offer(
        &self,
        payload: Value,
        scope: CommandScope,
        key: Option<&OperationKey>,
        options: RequestOptions,
    ) -> Result<ReplyBatch> {
        self.request_inner(true, "connection.offer", payload, scope, key, options)
            .await
    }
    async fn request_inner(
        &self,
        native: bool,
        kind: &str,
        payload: Value,
        scope: CommandScope,
        key: Option<&OperationKey>,
        options: RequestOptions,
    ) -> Result<ReplyBatch> {
        options.validate()?;
        if self.closed.load(Ordering::Acquire) {
            return Err(Error::request(ErrorKind::Cancelled, key, false));
        }
        if options.signal.is_cancelled() {
            return Err(Error::request(ErrorKind::Cancelled, key, false));
        }
        // Validate before queuing; optional extensions remain original payload data.
        let mut envelope = serde_json::json!({"v":2,"id":"sdk-validation","ts":chrono::Utc::now(),"type":kind,"payload":payload});
        for (field, value) in [
            ("cid", scope.conversation_id.as_ref()),
            ("sid", scope.session_id.as_ref()),
            ("connid", scope.connection_id.as_ref()),
            ("operation_id", scope.operation_id.as_ref()),
        ] {
            if let Some(value) = value {
                envelope[field] = serde_json::json!(value);
            }
        }
        if let Some(revision) = scope.expected_revision {
            envelope["expected_revision"] = serde_json::json!(revision);
        }
        crate::canonical::Envelope::from_value(envelope)
            .map_err(|_| Error::request(ErrorKind::Validation, key, false))?;
        let expires = tokio::time::Instant::now() + options.timeout;
        let slot = tokio::select! {biased;
            _=options.signal.cancelled()=>return Err(Error::request(ErrorKind::Cancelled,key,false)),
            result=tokio::time::timeout_at(expires,self.slots.clone().acquire_owned())=>result.map_err(|_|Error::request(ErrorKind::Deadline,key,false))?.map_err(|_|Error::request(ErrorKind::Cancelled,key,false))?
        };
        let permit = tokio::select! {biased;
            _=options.signal.cancelled()=>return Err(Error::request(ErrorKind::Cancelled,key,false)),
            result=tokio::time::timeout_at(expires,self.requests.reserve())=>result.map_err(|_|Error::request(ErrorKind::Deadline,key,false))?.map_err(|_|Error::request(ErrorKind::TransportUncertain,key,false))?
        };
        let (reply, response) = oneshot::channel();
        permit.send(Command {
            slot,
            native,
            kind: kind.to_owned(),
            payload,
            scope,
            key: key.cloned(),
            expires,
            signal: options.signal.clone(),
            reply,
        });
        // Acceptance is conservatively uncertain once ownership entered the queue.
        tokio::select! { biased;
            _=options.signal.cancelled()=>Err(Error::request(ErrorKind::Cancelled,key,true)),
            result=tokio::time::timeout_at(expires,response)=>result.map_err(|_|Error::request(ErrorKind::Deadline,key,true))?.map_err(|_|Error::request(ErrorKind::TransportUncertain,key,true))?
        }
    }
    pub(super) async fn close(&self, options: RequestOptions) -> Result<()> {
        options.validate()?;
        self.closed.store(true, Ordering::Release);
        self.stop.cancel();
        let expires = tokio::time::Instant::now() + options.timeout;
        let mut task = tokio::select! { biased;
            _=options.signal.cancelled()=>return Err(Error::request(ErrorKind::CleanupIncomplete,None,false)),
            result=tokio::time::timeout_at(expires,self.task.lock())=>result.map_err(|_|Error::request(ErrorKind::CleanupIncomplete,None,false))?
        };
        if let Some(join) = task.as_mut() {
            // Borrow rather than take the join handle: timeout retains future joinability.
            let joined = tokio::select! {biased;
                _=options.signal.cancelled()=>return Err(Error::request(ErrorKind::CleanupIncomplete,None,false)),
                result=tokio::time::timeout_at(expires,join)=>result
            };
            match joined {
                Ok(result) => {
                    *task = None;
                    result
                        .map_err(|_| Error::request(ErrorKind::CleanupIncomplete, None, false))?
                        .map_err(|_| Error::request(ErrorKind::CleanupIncomplete, None, false))?;
                }
                Err(_) => return Err(Error::request(ErrorKind::CleanupIncomplete, None, false)),
            }
        }
        Ok(())
    }
}

async fn confirm_native(
    peer: &mut CanonicalClient,
    pending: &InFlight,
    frame: &crate::canonical::Envelope,
    cache: &std::sync::Mutex<NativeCache>,
) -> Result<(NativeNegotiation, native_negotiation::Context)> {
    let fail = || Error::request(ErrorKind::Protocol, pending.key.as_ref(), true);
    let negotiation = pending.native.as_ref().ok_or_else(fail)?;
    let answer = negotiation.confirmation(frame)?;
    let selected = negotiation.selected()?.clone();
    let context = negotiation.context()?;
    let connection = pending
        .key
        .as_ref()
        .and_then(|k| k.scope.connection_id.as_ref())
        .ok_or_else(fail)?;
    {
        let cache = cache.lock().map_err(|_| fail())?;
        // Validate physical handle ownership before signing anything. A logical
        // saved reference cannot overwrite a live binding on this peer.
        if cache.len() >= NATIVE_CONNECTION_CAPACITY
            || cache.contains_key(connection)
            || context.3.values().copied().any(|handle| {
                cache
                    .values()
                    .flat_map(|entry| &entry.reserved_handles)
                    .any(|old| *old == handle)
            })
        {
            return Err(fail());
        }
    }
    peer.answer_connection_offer(frame, answer, &negotiation.scope)
        .await
        .map_err(|e| Error::transport(e, pending.key.as_ref()))?;
    Ok((selected, context))
}

fn native_binding(
    cache: &std::sync::Mutex<NativeCache>,
    key: &streams::StreamKey,
) -> Result<media::Binding> {
    let cache = cache
        .lock()
        .map_err(|_| Error::media(ErrorKind::Protocol, false))?;
    let entry = cache
        .get(&key.connection_id)
        .ok_or_else(|| Error::media(ErrorKind::Unsupported, false))?;
    if entry.session != key.session_id || entry.conversation != key.conversation_id {
        return Err(Error::media(ErrorKind::Validation, false));
    }
    if entry.ended {
        return Err(Error::media(ErrorKind::Unsupported, false));
    }
    let mut stream = entry
        .selected
        .streams
        .iter()
        .find(|s| s.id == key.stream_id)
        .cloned()
        .ok_or_else(|| Error::media(ErrorKind::Unsupported, false))?;
    if let Some(gate) = entry.gates.get(&key.stream_id) {
        stream.direction = gate.direction(stream.direction);
    }
    Ok(media::Binding {
        stream,
        clock: entry.selected.source_clock,
    })
}

fn observe_end(
    cache: &std::sync::Mutex<NativeCache>,
    media: &media::Inbox,
    key: &OperationKey,
    frame: &crate::canonical::Envelope,
) -> Result<()> {
    let fail = || Error::request(ErrorKind::Protocol, Some(key), true);
    let crate::canonical::Payload::OperationUpdated(update) = frame.payload() else {
        return Err(fail());
    };
    if update.operation_id.as_str() != key.operation_id.as_str()
        || update.revision > 9_007_199_254_740_991
    {
        return Err(fail());
    }
    if serde_json::to_value(update.state).map_err(|_| fail())? == serde_json::json!("completed") {
        connection_controls::validate_end(update.result.as_ref().ok_or_else(fail)?, key)?;
        retire_native(cache, media, key)?;
    }
    Ok(())
}
fn retire_native(
    cache: &std::sync::Mutex<NativeCache>,
    media: &media::Inbox,
    key: &OperationKey,
) -> Result<()> {
    let fail = || Error::request(ErrorKind::Protocol, Some(key), true);
    let cid = key.scope.conversation_id.as_ref().ok_or_else(fail)?;
    let sid = key.scope.session_id.as_ref().ok_or_else(fail)?;
    let conn = key.scope.connection_id.as_ref().ok_or_else(fail)?;
    let mut cache = cache.lock().map_err(|_| fail())?;
    if let Some(entry) = cache.get_mut(conn) {
        if &entry.session != sid || &entry.conversation != cid {
            return Err(fail());
        }
        media.retire(cid, sid, conn)?;
        entry.ended = true;
        entry.prepared = None;
    }
    Ok(())
}
