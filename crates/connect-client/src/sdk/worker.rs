//! One bounded owner multiplexes signed exchanges despite caller cancellation.
use super::*;
use crate::canonical::{CanonicalClient, CommandScope, ReplyBatch};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::{mpsc, oneshot, Mutex, OwnedSemaphorePermit, Semaphore};
const REQUEST_CAPACITY: usize = 16;
const NATIVE_CONNECTION_CAPACITY: usize = 64;
type NativeCache =
    std::collections::BTreeMap<ConnectionId, (SessionId, ConversationId, NativeNegotiation)>;
pub(super) struct Shared {
    requests: mpsc::Sender<Command>,
    stop: CancellationToken,
    pub(super) closed: Arc<AtomicBool>,
    slots: Arc<Semaphore>,
    native: Arc<std::sync::Mutex<NativeCache>>,
    task: Mutex<Option<tokio::task::JoinHandle<crate::Result<()>>>>,
}
struct InFlight {
    exchange: crate::canonical::exchange::Exchange,
    key: Option<OperationKey>,
    reply: Option<oneshot::Sender<Result<ReplyBatch>>>,
    native: Option<native_negotiation::Pending>,
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
                tokio::select! { biased;
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
                                let selected=match result {Ok(Ok(selected))=>selected,Ok(Err(error))=>break error.kind,Err(_)=>break ErrorKind::Deadline};
                                let pending=active.swap_remove(index);
                                let key=pending.key.expect("native key");
                                let mut cache=match owned_native.lock(){Ok(cache)=>cache,Err(_)=>break ErrorKind::Protocol};
                                cache.insert(key.scope.connection_id.expect("native connection"),(key.scope.session_id.expect("native session"),key.scope.conversation_id.expect("native conversation"),selected));
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
                            match peer.accept_exchange_frame(&mut active[index].exchange,frame) {
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
                    command=receiver.recv()=> {
                        let command=match command {Some(command)=>command,None=>break ErrorKind::Cancelled};
                        // Cancellation/deadline while queued never becomes a write.
                        if command.reply.is_closed() || command.signal.is_cancelled()
                            || tokio::time::Instant::now() >= command.expires {
                            let kind=if tokio::time::Instant::now() >= command.expires {ErrorKind::Deadline}else{ErrorKind::Cancelled};
                            let _=command.reply.send(Err(Error::request(kind,command.key.as_ref(),false)));
                            continue;
                        }
                        if command.native {
                            let count=match owned_native.lock(){Ok(cache)=>cache.len(),Err(_)=>break ErrorKind::Protocol};
                            if count+active.iter().filter(|p|p.native.is_some()).count()>=NATIVE_CONNECTION_CAPACITY {
                                let _=command.reply.send(Err(Error::request(ErrorKind::Configuration,command.key.as_ref(),false)));
                                continue;
                            }
                        }
                        let negotiation=command.native.then(||native_negotiation::Pending::new(command.payload.clone(),command.scope.clone()));
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
                            Ok(exchange)=>active.push(InFlight {exchange,key:command.key,reply:Some(command.reply),native:negotiation,_slot:command.slot}),
                            Err(error)=> {
                                let error=Error::transport(error,command.key.as_ref());
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
            task: Mutex::new(Some(task)),
        }))
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
            Some((session, conversation, selected)) if session == sid && conversation == cid => {
                Ok(Some(selected.clone()))
            }
            Some(_) => Err(Error::local(ErrorKind::Protocol)),
            None => Ok(None),
        }
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
) -> Result<NativeNegotiation> {
    let fail = || Error::request(ErrorKind::Protocol, pending.key.as_ref(), true);
    let negotiation = pending.native.as_ref().ok_or_else(fail)?;
    let answer = negotiation.confirmation(frame)?;
    let selected = negotiation.selected()?.clone();
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
            || selected
                .streams
                .iter()
                .filter_map(|s| s.stream_local_id)
                .any(|handle| {
                    cache
                        .values()
                        .flat_map(|(_, _, n)| &n.streams)
                        .any(|s| s.stream_local_id == Some(handle))
                })
        {
            return Err(fail());
        }
    }
    peer.answer_connection_offer(frame, answer, &negotiation.scope)
        .await
        .map_err(|e| Error::transport(e, pending.key.as_ref()))?;
    Ok(selected)
}
