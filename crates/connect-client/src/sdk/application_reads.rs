//! A bounded owner for explicitly selected read-only application HTTP work.
use super::*;
use crate::application::{ApplicationClient, ApplicationOperation};
use connect_protocol::{Operation as ApplicationCommand, Outcome};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{mpsc, oneshot, Mutex, OwnedSemaphorePermit, Semaphore};
struct Read {
    command: ApplicationOperation,
    expires: tokio::time::Instant,
    signal: CancellationToken,
    reply: oneshot::Sender<Result<Outcome>>,
    _slot: OwnedSemaphorePermit,
}
pub(super) struct OwnedReads {
    requests: mpsc::Sender<Read>,
    slots: Arc<Semaphore>,
    stop: CancellationToken,
    native_closed: Arc<AtomicBool>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}
impl Drop for OwnedReads {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
impl OwnedReads {
    pub(super) fn start(
        mut application: ApplicationClient,
        native_closed: Arc<AtomicBool>,
    ) -> Arc<Self> {
        let (requests, mut receiver) = mpsc::channel::<Read>(16);
        let slots = Arc::new(Semaphore::new(16));
        let stop = CancellationToken::new();
        let stopping = stop.clone();
        let native = native_closed.clone();
        let task = tokio::spawn(async move {
            loop {
                let read = tokio::select! {biased;_=stopping.cancelled()=>break,read=receiver.recv()=>match read {Some(r)=>r,None=>break}};
                let fail = |kind| Error::request(kind, None, false);
                let result = if read.reply.is_closed()
                    || read.signal.is_cancelled()
                    || native.load(Ordering::Acquire)
                {
                    Err(fail(ErrorKind::Cancelled))
                } else if tokio::time::Instant::now() >= read.expires {
                    Err(fail(ErrorKind::Deadline))
                } else {
                    // Read-only IO is independently disposable. No pooled socket or
                    // canonical framing/authority can be reused after cancellation.
                    tokio::select! {biased;
                        _=stopping.cancelled()=>Err(fail(ErrorKind::Cancelled)),
                        _=read.signal.cancelled()=>Err(fail(ErrorKind::Cancelled)),
                        result=tokio::time::timeout_at(read.expires,application.execute_with_cancel(read.command,stopping.clone()))=>match result {
                            Ok(result)=>result.map_err(|e|administration::application_error(e,None)),
                            Err(_)=>Err(fail(ErrorKind::Deadline)),
                        }
                    }
                };
                let _ = read.reply.send(result);
            }
            receiver.close();
            while let Some(read) = receiver.recv().await {
                let _ = read
                    .reply
                    .send(Err(Error::request(ErrorKind::Cancelled, None, false)));
            }
        });
        Arc::new(Self {
            requests,
            slots,
            stop,
            native_closed,
            task: Mutex::new(Some(task)),
        })
    }
    pub(super) async fn receipts(
        &self,
        cid: &ConversationId,
        message: &MessageId,
        after: Option<String>,
        options: RequestOptions,
    ) -> Result<connect_protocol::receipts::ReceiptPage> {
        let command = ApplicationCommand::MessageReceipts {
            conversation_id: cid.as_str().into(),
            message_id: message.as_str().into(),
            after,
            limit: 100,
        };
        match self.read(command, options).await? {
            Outcome::MessageReceipts(page) => Ok(page),
            _ => Err(Error::request(ErrorKind::Protocol, None, false)),
        }
    }
    async fn read(&self, command: ApplicationCommand, options: RequestOptions) -> Result<Outcome> {
        let fail = |kind| Error::request(kind, None, false);
        options.validate().map_err(|e| fail(e.kind))?;
        if self.stop.is_cancelled()
            || self.native_closed.load(Ordering::Acquire)
            || options.signal.is_cancelled()
        {
            return Err(fail(ErrorKind::Cancelled));
        }
        let expires = tokio::time::Instant::now() + options.timeout;
        let slot = tokio::select! {biased;_=options.signal.cancelled()=>return Err(fail(ErrorKind::Cancelled)),result=tokio::time::timeout_at(expires,self.slots.clone().acquire_owned())=>result.map_err(|_|fail(ErrorKind::Deadline))?.map_err(|_|fail(ErrorKind::Cancelled))?};
        let permit = tokio::select! {biased;_=options.signal.cancelled()=>return Err(fail(ErrorKind::Cancelled)),result=tokio::time::timeout_at(expires,self.requests.reserve())=>result.map_err(|_|fail(ErrorKind::Deadline))?.map_err(|_|fail(ErrorKind::Cancelled))?};
        let command = ApplicationOperation::try_from(command)
            .map_err(|e| administration::application_error(e, None))?;
        let (reply, response) = oneshot::channel();
        permit.send(Read {
            command,
            expires,
            signal: options.signal.clone(),
            reply,
            _slot: slot,
        });
        tokio::select! {biased;_=options.signal.cancelled()=>Err(fail(ErrorKind::Cancelled)),result=tokio::time::timeout_at(expires,response)=>result.map_err(|_|fail(ErrorKind::Deadline))?.map_err(|_|fail(ErrorKind::Cancelled))?}
    }
    pub(super) async fn close(&self, options: RequestOptions) -> Result<()> {
        options.validate()?;
        self.stop.cancel();
        self.slots.close();
        let fail = || Error::local(ErrorKind::CleanupIncomplete);
        let expires = tokio::time::Instant::now() + options.timeout;
        let mut task = tokio::select! {biased;_=options.signal.cancelled()=>return Err(fail()),result=tokio::time::timeout_at(expires,self.task.lock())=>result.map_err(|_|fail())?};
        if let Some(job) = task.as_mut() {
            let result = tokio::select! {biased;_=options.signal.cancelled()=>return Err(fail()),result=tokio::time::timeout_at(expires,job)=>result.map_err(|_|fail())?};
            *task = None;
            result.map_err(|_| fail())?;
        }
        Ok(())
    }
}
