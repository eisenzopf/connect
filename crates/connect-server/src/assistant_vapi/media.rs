//! Current opaque-memory authority covers every public provider audio poll.
use super::*;
pub(super) async fn forward(
    service: &Arc<Service>,
    context: &AssistantProviderContext,
    applied: &Arc<AssistantProviderApplied>,
    socket: &mut Socket,
    native: &mut PublicNativeAudio,
    stop: &CancellationToken,
    updates: &mut mpsc::Receiver<UpdateRequest>,
    status: &watch::Sender<AssistantVapiStatus>,
    ended: &mut bool,
    capture: &mut Option<crate::agent_observations::Capture>,
) -> Result<bool> {
    let mut queue = crate::private_vapi::AudioQueue::default();
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut guard: Option<AssistantProviderContextRead> = None;
    let mut refresh = tokio::time::Instant::now();
    loop {
        tokio::select! {biased;
            _=stop.cancelled()=>return Ok(false),
            request=updates.recv()=>match request {
                Some(request)=>{
                    guard=None;queue=crate::private_vapi::AudioQueue::default();
                    updates::process(service,context,applied,socket,stop,status,ended,capture,request).await?;
                    refresh=tokio::time::Instant::now();
                },None=>return Ok(false)
            },
            _=tick.tick()=>{
                if tokio::time::Instant::now()>=refresh || guard.as_ref().is_none_or(|g|!g.current()) {
                    guard=None;
                    let s=service.clone();let c=context.clone();let p=applied.clone();
                    match blocking(move||s.try_assistant_memory_read(&c,&p)).await {
                        Ok(read)=>guard=Some(read),
                        Err(e) if matches!(e.code.as_str(),"authority-busy"|"storage-unavailable")=>{},
                        Err(e)=>return Err(e),
                    }
                    refresh=tokio::time::Instant::now()+Duration::from_millis(100);
                }
                if guard.as_ref().is_some_and(|g|g.current()) {
                    native.send_pcmu(&queue.frame(tokio::time::Instant::now()))
                        .map_err(|_|problem_error("assistant-native-ended"))?;
                }
            },
            packet=native.receive_pcmu()=>{
                let packet=match packet {
                    Ok(packet)=>packet,
                    Err(connect_client::ClientError::Timeout)=>continue,
                    Err(_)=>return Err(problem_error("assistant-native-ended")),
                };
                if let Some(read)=guard.as_ref().filter(|g|g.current()) {
                    current_io(read,async {
                        tokio::time::timeout(Duration::from_millis(100),socket.send(Message::Binary(packet.into())))
                            .await.map_err(|_|problem_error("provider-media-timeout"))?
                            .map_err(|_|problem_error("provider-media-ended"))
                    }).await?;
                }
            },
            frame=socket.next()=>match frame {
                Some(Ok(Message::Binary(bytes)))=>{
                    if guard.as_ref().is_some_and(|g|g.current()) {
                        queue.push(&bytes,tokio::time::Instant::now());
                    }
                },
                Some(Ok(Message::Text(text)))=>{
                    let decoded=rvoip_vapi::decode_agent_event(&text,applied.provider_ref()).map_err(|_|problem_error("provider-event-invalid"))?;
                    if let Some(capture)=capture.as_mut(){capture.observe(&decoded);}
                    if decoded.is_terminal() {return Ok(true);}
                },
                Some(Ok(Message::Close(_)))|None|Some(Err(_))=>return Err(problem_error("provider-media-ended")),
                _=>{},
            }
        }
    }
}
