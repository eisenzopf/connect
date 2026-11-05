//! Physical owner for one Core-reserved RTC replacement. Original effect custody
//! remains in the service; this worker never allocates a second logical attempt.
use super::*;
use connect_service::canonical_webrtc::{
    ReplacementAttempt, ReplacementPreparation, ReplacementSource,
};
use rvoip_uctp::v2::payloads::ConnectionAnswer;

pub(super) fn start(
    p: ReplacementPreparation,
    config: WebRtcConfig,
) -> Result<Arc<dyn ReplacementAttempt>> {
    if p.deadline <= Instant::now()
        || p.deadline > Instant::now() + Duration::from_secs(30)
        || config.trickle_ice
    {
        return Err(Error::new(
            "invalid-request",
            "RTC replacement requires bounded complete ICE preparation",
        ));
    }
    let stop = p.physical.child_token();
    let (state_tx, state) = watch::channel(State::Preparing);
    let source = p.source.clone();
    let av = config.require_vp8_audio_video_profile;
    let task = tokio::spawn(run(p, config, stop.clone(), state_tx));
    let completion = async move {
        task.await
            .map_err(|_| Error::new("cleanup-unconfirmed", "RTC replacement worker join failed"))?
    }
    .boxed()
    .shared();
    Ok(Arc::new(Owned {
        source,
        stop,
        state,
        completion,
        av,
    }))
}
struct Owned {
    source: Arc<dyn ReplacementSource>,
    stop: CancellationToken,
    state: watch::Receiver<State>,
    completion: Completion,
    av: bool,
}
#[async_trait::async_trait]
impl ReplacementAttempt for Owned {
    async fn answer(&self, deadline: Instant) -> Result<ConnectionAnswer> {
        let mut state = self.state.clone();
        loop {
            let current = state.borrow().clone();
            match current {
                State::Answered(answer) | State::Connected(answer) => {
                    let offer = bounded_read(&self.stop, deadline, || self.source.offer()).await?;
                    return super::signaling::selected_answer(
                        &offer.participant_id,
                        &offer.offer,
                        &answer,
                        self.av,
                    );
                }
                State::Closed(result) => {
                    return result.and_then(|_| {
                        Err(Error::new("closed", "RTC replacement closed before answer"))
                    });
                }
                State::Preparing => {}
            }
            tokio::select! {biased;
                _=self.stop.cancelled()=>return Err(Error::new("closed","RTC replacement retired")),
                _=tokio::time::sleep_until(deadline)=>return Err(Error::new("timeout","RTC replacement answer deadline")),
                changed=state.changed()=>changed.map_err(|_|Error::new("cleanup-unconfirmed","RTC replacement state unavailable"))?,
            }
        }
    }
    async fn shutdown(&self) -> Result<()> {
        self.source.release_invocation();
        self.stop.cancel();
        self.completion.clone().await
    }
    async fn closed(&self) -> Result<()> {
        self.completion.clone().await
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        self.source.release_invocation();
        self.stop.cancel();
    }
}
async fn run(
    p: ReplacementPreparation,
    config: WebRtcConfig,
    stop: CancellationToken,
    state: watch::Sender<State>,
) -> Result<()> {
    let result:Result<()>=async {
        let offer=bounded_read(&stop,p.deadline,||p.source.offer()).await?;
        super::signaling::validate_offer(&offer.offer,config.require_vp8_audio_video_profile)?;
        let sdp=offer.offer["substrate_setup"]["sdp"].as_str().ok_or_else(Error::forbidden)?;
        p.binding.start_peer(config,PeerRole::Answerer).map_err(|_|engine_error())?;
        let peer=p.binding.peer(p.deadline).await.map_err(|_|engine_error())?;
        let answer=tokio::select! {biased;
            _=stop.cancelled()=>return Err(Error::new("cancelled","RTC replacement preparation cancelled")),
            _=tokio::time::sleep_until(p.deadline)=>return Err(Error::new("timeout","RTC replacement preparation deadline")),
            answer=peer.accept_offer_and_gather(sdp)=>answer.map_err(|_|engine_error())?,
        };
        if !answer.starts_with("v=0\r\n") || answer.len()>512*1024 || answer.contains('\0') {return Err(engine_error());}
        bounded_read(&stop,p.deadline,||p.source.current()).await?;
        state.send_replace(State::Answered(answer.clone()));
        let mut interval=tokio::time::interval(Duration::from_millis(50));interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {biased;
                _=stop.cancelled()=>return Ok(()),
                _=peer.wait_failed()=>return Err(engine_error()),
                _=interval.tick()=>{
                    match p.source.current() {
                        Ok(())=>{},
                        Err(error) if matches!(error.code.as_str(),"authority-busy"|"storage-unavailable"|"handoff-pending")=>continue,
                        Err(error)=>return Err(error),
                    }
                    if peer.is_connected(){state.send_if_modified(|s|if matches!(s,State::Connected(_)){false}else{*s=State::Connected(answer.clone());true});}
                }
            }
        }
    }.await;
    p.source.release_invocation();
    let cleanup = p.binding.shutdown().await.map_err(|_| {
        Error::new(
            "cleanup-unconfirmed",
            "RTC replacement original physical close failed",
        )
    });
    state.send_replace(State::Closed(cleanup.clone().and(result)));
    cleanup
}
