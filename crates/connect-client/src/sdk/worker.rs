//! One bounded owner finishes each submitted exchange despite caller cancellation.
use super::*;
use crate::canonical::{CanonicalClient, CommandScope, ReplyBatch};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::{mpsc, oneshot, Mutex};
const REQUEST_CAPACITY: usize = 16;
pub(super) struct Shared {
    requests: mpsc::Sender<Command>,
    stop: CancellationToken,
    pub(super) closed: AtomicBool,
    task: Mutex<Option<tokio::task::JoinHandle<crate::Result<()>>>>,
}
struct Command {
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
        let task = tokio::spawn(async move {
            loop {
                let command = tokio::select! { biased; _=stopping.cancelled()=>break, command=receiver.recv()=>match command {Some(c)=>c,None=>break} };
                // Cancellation/deadline while queued never becomes a write.
                if command.reply.is_closed()
                    || command.signal.is_cancelled()
                    || tokio::time::Instant::now() >= command.expires
                {
                    let _ = command.reply.send(Err(Error::request(
                        ErrorKind::Cancelled,
                        command.key.as_ref(),
                        false,
                    )));
                    continue;
                }
                // Once started, only explicit client close may interrupt the exchange.
                // Dropping/cancelling the caller's response waiter is deliberately absent.
                let result = tokio::select! { biased; _=stopping.cancelled()=>break, result=peer.request_value(&command.kind,command.payload,command.scope)=>result };
                let result = result.map_err(|e| Error::transport(e, command.key.as_ref()));
                let _ = command.reply.send(result);
            }
            receiver.close();
            while let Some(command) = receiver.recv().await {
                let _ = command.reply.send(Err(Error::request(
                    ErrorKind::Cancelled,
                    command.key.as_ref(),
                    false,
                )));
            }
            peer.shutdown().await
        });
        Ok(Arc::new(Self {
            requests,
            stop,
            closed: AtomicBool::new(false),
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
        let permit = tokio::select! {biased;
            _=options.signal.cancelled()=>return Err(Error::request(ErrorKind::Cancelled,key,false)),
            result=tokio::time::timeout_at(expires,self.requests.reserve())=>result.map_err(|_|Error::request(ErrorKind::Deadline,key,false))?.map_err(|_|Error::request(ErrorKind::TransportUncertain,key,false))?
        };
        let (reply, response) = oneshot::channel();
        permit.send(Command {
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
