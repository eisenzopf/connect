//! Authenticated attachment, bounded concurrent routing and owned socket cleanup.
use super::*;
use tokio::{
    net::TcpStream,
    sync::mpsc,
    task::{JoinHandle, JoinSet},
};

pub(in crate::sdk::helper) struct Channel {
    stop: CancellationToken,
    task: Option<JoinHandle<Result<()>>>,
}
impl Drop for Channel {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
impl Channel {
    pub(in crate::sdk::helper) async fn attach(
        client: Client,
        attachment: Attachment,
        options: RequestOptions,
    ) -> Result<Self> {
        attachment.validate()?;
        let until = tokio::time::Instant::now() + options.timeout;
        let stream = tokio::select! {
            biased;
            _ = options.signal.cancelled() => return Err(Error::local(ErrorKind::Cancelled)),
            result = tokio::time::timeout_at(until, TcpStream::connect(attachment.endpoint)) =>
                result.map_err(|_| Error::local(ErrorKind::Deadline))?
                    .map_err(|_| Error::local(ErrorKind::Configuration))?,
        };
        stream
            .set_nodelay(true)
            .map_err(|_| Error::local(ErrorKind::Configuration))?;
        let (input, mut output) = stream.into_split();
        let mut reader = media_ipc::Reader::new(input);
        let hello = response(
            1,
            media_ipc::Kind::Request,
            json!({"profile":PROFILE,"token":attachment.token}),
            Vec::new(),
        )?;
        tokio::select! {
            biased;
            _ = options.signal.cancelled() => return Err(Error::local(ErrorKind::Cancelled)),
            result = tokio::time::timeout_at(until, async {
                media_ipc::write(&mut output,&hello).await.map_err(|_| Error::local(ErrorKind::Protocol))?;
                authenticate(reader.next().await.map_err(|_| Error::local(ErrorKind::Protocol))?, &attachment.token)
            }) => result.map_err(|_| Error::local(ErrorKind::Deadline))??,
        }
        let stop = CancellationToken::new();
        let owned = stop.clone();
        let task = tokio::spawn(run(reader, output, owned, move |frame, token| {
            execute(client.clone(), frame, token)
        }));
        Ok(Self {
            stop,
            task: Some(task),
        })
    }
    pub(in crate::sdk::helper) async fn close(mut self, options: RequestOptions) -> Result<()> {
        self.stop.cancel();
        let mut task = self.task.take().unwrap();
        match tokio::time::timeout(options.timeout, &mut task).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(Error::local(ErrorKind::CleanupIncomplete)),
            Err(_) => {
                task.abort();
                let _ = task.await;
                Err(Error::local(ErrorKind::CleanupIncomplete))
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Authentication {
    profile: String,
    token: String,
}
fn authenticate(frame: Option<media_ipc::Frame>, token: &str) -> Result<()> {
    let frame = frame.ok_or_else(|| Error::local(ErrorKind::Protocol))?;
    if frame.kind != media_ipc::Kind::Response || frame.request_id != 1 || !frame.payload.is_empty()
    {
        return Err(Error::local(ErrorKind::Protocol));
    }
    let auth: Authentication =
        serde_json::from_slice(&frame.metadata).map_err(|_| Error::local(ErrorKind::Protocol))?;
    // Fixed-length comparison does not disclose a matching token prefix.
    if auth.profile != PROFILE
        || auth.token.len() != token.len()
        || auth
            .token
            .bytes()
            .zip(token.bytes())
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            != 0
    {
        return Err(Error::local(ErrorKind::Authentication));
    }
    Ok(())
}

enum Completed {
    Writer(std::io::Result<()>),
    Reply(media_ipc::Frame),
}
async fn queued(
    sender: &mpsc::Sender<media_ipc::Frame>,
    frame: media_ipc::Frame,
    stop: &CancellationToken,
) -> Result<()> {
    tokio::select! {
        biased;
        _ = stop.cancelled() => Ok(()),
        result = sender.send(frame) => result.map_err(|_| Error::local(ErrorKind::Protocol)),
    }
}
async fn run<R, W, F, T>(
    mut reader: media_ipc::Reader<R>,
    mut output: W,
    stop: CancellationToken,
    execute: F,
) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
    F: Fn(media_ipc::Frame, CancellationToken) -> T,
    T: std::future::Future<Output = media_ipc::Frame> + Send + 'static,
{
    let (sender, mut replies) = mpsc::channel::<media_ipc::Frame>(WINDOW);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let result = async {
            while let Some(frame) = replies.recv().await {
                media_ipc::write(&mut output, &frame).await?;
            }
            tokio::io::AsyncWriteExt::shutdown(&mut output).await
        }
        .await;
        Completed::Writer(result)
    });
    let mut last = 1;
    let result = loop {
        tokio::select! {
            biased;
            _ = stop.cancelled() => break Ok(()),
            complete = tasks.join_next() => match complete {
                Some(Ok(Completed::Reply(frame))) => if let Err(error)=queued(&sender,frame,&stop).await { break Err(error); },
                Some(Ok(Completed::Writer(result))) => break result.map_err(|_| Error::local(ErrorKind::Protocol)),
                _ => break Err(Error::local(ErrorKind::Protocol)),
            },
            frame = reader.next() => {
                let frame=match frame {
                    Ok(Some(frame)) => frame,
                    Ok(None) => break Ok(()),
                    Err(_) => break Err(Error::local(ErrorKind::Protocol)),
                };
                // Each attached channel owns a strictly increasing correlation sequence.
                if frame.kind != media_ipc::Kind::Request || frame.request_id <= last {
                    break Err(Error::local(ErrorKind::Protocol));
                }
                last=frame.request_id;
                if tasks.len()>WINDOW { // writer + at most16 active requests
                    let refusal=failure(frame.request_id,Error::media(ErrorKind::Application,false));
                    if let Err(error)=queued(&sender,refusal,&stop).await { break Err(error); }
                } else {
                    let future=execute(frame,stop.child_token());
                    tasks.spawn(async move { Completed::Reply(future.await) });
                }
            }
        }
    };
    stop.cancel();
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handshake_requires_exact_profile_token_correlation_and_direction() {
        let token = "a".repeat(64);
        let valid = || {
            response(
                1,
                media_ipc::Kind::Response,
                json!({"profile":PROFILE,"token":token}),
                Vec::new(),
            )
            .unwrap()
        };
        authenticate(Some(valid()), &token).unwrap();
        for wrong in ["b".repeat(64), "a".repeat(63)] {
            assert!(authenticate(Some(valid()), &wrong).is_err());
        }
        for field in ["profile", "credential"] {
            let mut value = json!({"profile":PROFILE,"token":token});
            value[field] = json!("private");
            assert!(authenticate(
                Some(response(1, media_ipc::Kind::Response, value, Vec::new()).unwrap()),
                &token
            )
            .is_err());
        }
        let mut bad = valid();
        bad.request_id = 2;
        assert!(authenticate(Some(bad), &token).is_err());
        let mut bad = valid();
        bad.kind = media_ipc::Kind::Request;
        assert!(authenticate(Some(bad), &token).is_err());
        let mut bad = valid();
        bad.payload.push(1);
        assert!(authenticate(Some(bad), &token).is_err());
    }
    #[tokio::test]
    async fn stopped_channel_joins_blocked_read_and_blocked_writer_without_detached_tasks() {
        use tokio::io::AsyncReadExt;
        let (mut peer, socket) = tokio::io::duplex(16);
        let (input, output) = tokio::io::split(socket);
        let stop = CancellationToken::new();
        let owned = stop.clone();
        let task = tokio::spawn(run(
            media_ipc::Reader::new(input),
            output,
            owned,
            |frame, _| async move {
                response(
                    frame.request_id,
                    media_ipc::Kind::Packet,
                    json!({}),
                    vec![1; media_ipc::MAX_PAYLOAD_BYTES],
                )
                .unwrap()
            },
        ));
        media_ipc::write(
            &mut peer,
            &response(2, media_ipc::Kind::Request, json!({}), Vec::new()).unwrap(),
        )
        .await
        .unwrap();
        let mut prefix = [0; 16];
        tokio::time::timeout(Duration::from_secs(1), peer.read_exact(&mut prefix))
            .await
            .unwrap()
            .unwrap();
        // The17-byte header has started; the maximum-size body cannot drain
        // into this16-byte socket while the caller has stopped reading.
        assert!(!task.is_finished());
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let mut rest = Vec::new();
        tokio::time::timeout(Duration::from_secs(1), peer.read_to_end(&mut rest))
            .await
            .unwrap()
            .unwrap();
        let mut partial = prefix.to_vec();
        partial.extend(rest);
        let mut retired = media_ipc::Reader::new(partial.as_slice());
        assert_eq!(
            retired.next().await.unwrap_err().kind(),
            std::io::ErrorKind::UnexpectedEof
        );
    }
}
