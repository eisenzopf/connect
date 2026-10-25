//! One bounded owner multiplexes signed exchanges despite caller cancellation.
use super::*;
use crate::canonical::{CanonicalClient, CommandScope, ReplyBatch};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::{mpsc, oneshot, Mutex, OwnedSemaphorePermit, Semaphore};
const REQUEST_CAPACITY: usize = 16;
pub(super) struct Shared {
    requests: mpsc::Sender<Command>,
    stop: CancellationToken,
    pub(super) closed: Arc<AtomicBool>,
    slots: Arc<Semaphore>,
    task: Mutex<Option<tokio::task::JoinHandle<crate::Result<()>>>>,
}
struct InFlight {
    exchange: crate::canonical::exchange::Exchange,
    key: Option<OperationKey>,
    reply: oneshot::Sender<Result<ReplyBatch>>,
    _slot: OwnedSemaphorePermit,
}
struct Command {
    slot: OwnedSemaphorePermit,
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
                        let index = active.iter().position(|pending| frame.scope(crate::canonical::Scope::Reply) == Some(pending.exchange.id.as_str()));
                        if let Some(index) = index {
                            match peer.accept_exchange_frame(&mut active[index].exchange,frame) {
                                Ok(true)=> {
                                    let pending=active.swap_remove(index);
                                    let _=pending.reply.send(Ok(pending.exchange.finish()));
                                }
                                Ok(false)=>{},
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
                            Ok(exchange)=>active.push(InFlight {exchange,key:command.key,reply:command.reply,_slot:command.slot}),
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
                let _ =
                    pending
                        .reply
                        .send(Err(Error::request(failure, pending.key.as_ref(), true)));
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
            task: Mutex::new(Some(task)),
        }))
    }
    pub(super) async fn request(
        &self,
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
