//! Exact acknowledgment for inline initialization and post-creation exports.
use super::*;
pub(super) async fn acknowledge(
    socket: &mut Socket,
    content: &str,
    provider_ref: &str,
    mut capture: Option<&mut crate::agent_observations::Capture>,
    truncated: &mut bool,
    ended: &mut bool,
    stop: &CancellationToken,
) -> Result<String> {
    loop {
        let frame = tokio::select! {biased;_=stop.cancelled()=>return Err(problem_error("cancelled")),frame=socket.next()=>frame};
        match frame {
            Some(Ok(Message::Text(text))) => {
                let decoded = rvoip_vapi::decode_agent_event(&text, provider_ref)
                    .map_err(|_| problem_error("provider-event-invalid"))?;
                if let Some(capture) = capture.as_deref_mut() {
                    capture.observe(&decoded);
                }
                let value: Value = serde_json::from_str(&text)
                    .map_err(|_| problem_error("provider-event-invalid"))?;
                if value["type"] == "conversation-update" {
                    for key in ["messagesOpenAIFormatted", "conversation", "messages"] {
                        for message in value[key].as_array().into_iter().flatten() {
                            if message["role"] != "system" {
                                continue;
                            }
                            if let Some(body) = message["content"].as_str() {
                                if body == content {
                                    return Ok(body.to_owned());
                                }
                                if !body.is_empty() && content.starts_with(body) {
                                    *truncated = true;
                                }
                            }
                        }
                    }
                }
                if decoded.is_terminal() {
                    *ended = true;
                    return Err(problem_error("provider-context-incomplete"));
                }
            }
            Some(Ok(Message::Close(_))) | None => {
                return Err(problem_error("provider-context-incomplete"))
            }
            Some(Err(_)) => return Err(problem_error("provider-context-unconfirmed")),
            _ => {}
        }
    }
}
pub(super) async fn deliver(
    service: &Arc<Service>,
    context: &AssistantProviderContext,
    created: &Arc<connect_store::AssistantProviderCreated>,
    initialization: Option<&connect_store::AssistantProviderInitialization>,
    url: &Url,
    secret: &crate::secrets::SecretValue,
    allowed: bool,
    ws: &mut Option<Socket>,
    export_started: &mut bool,
    ended: &mut bool,
    stop: &CancellationToken,
) -> Result<Arc<AssistantProviderApplied>> {
    let mut proof = initialization
        .map(|init| init.export_for(created))
        .transpose()?;
    *export_started = proof.is_some();
    let prepared = async {
        if !allowed {
            return Err(problem_error("provider-transport-refused"));
        }
        if stop.is_cancelled() {
            return Err(problem_error("cancelled"));
        }
        let selected = if initialization.is_some() {
            retry(|| {
                let s = service.clone();
                let c = context.clone();
                blocking(move || s.try_assistant_initialization_read(&c))
            })
            .await?
        } else {
            read(service, context).await?
        };
        let content = selected.context_json()?.to_owned();
        let selected = if proof.is_none() {
            let s = service.clone();
            let c = context.clone();
            let (export, selected) =
                blocking(move || s.begin_assistant_export(c, selected)).await?;
            proof = Some(export);
            *export_started = true;
            selected
        } else {
            selected
        };
        Ok((content, selected))
    }
    .await;
    let (content, selected) = match prepared {
        Ok(selected) => selected,
        Err(error) => {
            if let Some(proof) = proof {
                let proof = Arc::new(proof);
                retry(|| {
                    let s = service.store.clone();
                    let p = proof.clone();
                    blocking(move || s.native_assistant_export_uncertain(&p))
                })
                .await?;
            }
            return Err(error);
        }
    };
    let proof = proof.ok_or_else(Error::storage)?;
    let mut truncated = false;
    let delivered=current_io(&selected,async {
        *ws=Some(socket_with_limit(url,secret,512*1024).await?);
        if stop.is_cancelled()||!selected.current(){return Err(problem_error("cancelled"));}
        let socket=ws.as_mut().ok_or_else(Error::storage)?;
        if initialization.is_none() {
            socket.send(Message::Text(json!({"type":"add-message","message":{"role":"system","content":content},"triggerResponseEnabled":false}).to_string().into())).await.map_err(|_|problem_error("provider-context-unconfirmed"))?;
        }
        let body=acknowledge(socket,&content,created.provider_ref(),None,&mut truncated,ended,stop).await?;
        proof.acknowledged(&body)
    }).await;
    drop(selected);
    match delivered {
        Ok(applied) => {
            let applied = Arc::new(applied);
            retry(|| {
                let s = service.store.clone();
                let p = applied.clone();
                blocking(move || s.record_native_assistant_delivered(&p))
            })
            .await?;
            Ok(applied)
        }
        Err(error) => {
            if truncated {
                let rejected = Arc::new(proof.rejected(AssistantContextFailure::Truncated));
                retry(|| {
                    let s = service.store.clone();
                    let p = rejected.clone();
                    blocking(move || s.record_native_assistant_rejected(&p))
                })
                .await?;
            } else if *ended {
                let proof = Arc::new(proof);
                retry(|| {
                    let s = service.store.clone();
                    let p = proof.clone();
                    blocking(move || s.native_assistant_export_incomplete(&p))
                })
                .await?;
            } else {
                let proof = Arc::new(proof);
                retry(|| {
                    let s = service.store.clone();
                    let p = proof.clone();
                    blocking(move || s.native_assistant_export_uncertain(&p))
                })
                .await?;
            }
            Err(error)
        }
    }
}
