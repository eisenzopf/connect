//! Serialized updates use this owned socket and leave independent receipts.
use super::*;
pub(super) async fn process(
    service: &Arc<Service>,
    context: &AssistantProviderContext,
    applied: &Arc<AssistantProviderApplied>,
    socket: &mut Socket,
    stop: &CancellationToken,
    status: &watch::Sender<AssistantVapiStatus>,
    ended: &mut bool,
    capture: &mut Option<crate::agent_observations::Capture>,
    request: UpdateRequest,
) -> Result<()> {
    if request.reply.is_closed() {
        return Ok(());
    }
    let (result, fatal) = apply(
        service,
        context,
        applied,
        socket,
        stop,
        status,
        ended,
        capture,
        &request.operation_id,
    )
    .await;
    let problem = result.as_ref().err().cloned();
    let _ = request.reply.send(result);
    if fatal {
        Err(problem.unwrap_or_else(|| problem_error("provider-context-stopped")))
    } else {
        status.send_replace(AssistantVapiStatus::ContextReady);
        Ok(())
    }
}
async fn not_exported(
    service: &Arc<Service>,
    update: &Arc<connect_store::AssistantProviderUpdate>,
    error: Error,
) -> (Result<String>, bool) {
    let result = retry(|| {
        let s = service.store.clone();
        let u = update.clone();
        blocking(move || s.record_native_assistant_update_rejected(&u.not_exported()))
    })
    .await;
    match result {
        Ok(()) => (Err(error), false),
        Err(record) => (Err(record), true),
    }
}
async fn apply(
    service: &Arc<Service>,
    context: &AssistantProviderContext,
    applied: &Arc<AssistantProviderApplied>,
    socket: &mut Socket,
    stop: &CancellationToken,
    status: &watch::Sender<AssistantVapiStatus>,
    ended: &mut bool,
    capture: &mut Option<crate::agent_observations::Capture>,
    operation_id: &str,
) -> (Result<String>, bool) {
    let prepared = retry(|| {
        let s = service.clone();
        let c = context.clone();
        let p = applied.clone();
        let operation = operation_id.to_owned();
        blocking(move || {
            let read = s.try_assistant_memory_read(&c, &p)?;
            s.prepare_assistant_update(&operation, &c, &p, read)
        })
    })
    .await;
    let update = match prepared {
        Ok(update) => Arc::new(update),
        Err(error) => return (Err(error), false),
    };
    let state = retry(|| {
        let s = service.store.clone();
        let u = update.clone();
        blocking(move || s.native_assistant_update_status(&u))
    })
    .await;
    let state = match state {
        Ok(state) => state,
        Err(error) => return (Err(error), true),
    };
    if state.state == "delivered" {
        return (Ok(update.delivery_id().into()), false);
    }
    if state.state != "pending" || state.export_started {
        return (
            Err(problem_error("reconciliation-required")),
            state.export_started,
        );
    }
    if stop.is_cancelled() {
        return not_exported(service, &update, problem_error("cancelled")).await;
    }
    let selected = retry(|| {
        let s = service.clone();
        let u = update.clone();
        let p = applied.clone();
        blocking(move || s.try_assistant_update_read(&u, &p))
    })
    .await;
    let selected = match selected {
        Ok(read) => read,
        Err(error) => return not_exported(service, &update, error).await,
    };
    let content = match selected.context_json() {
        Ok(body) => body.to_owned(),
        Err(error) => {
            drop(selected);
            return not_exported(service, &update, error).await;
        }
    };
    let s = service.clone();
    let u = update.clone();
    let admitted = blocking(move || s.begin_assistant_update(&u, selected)).await;
    let (export, selected) = match admitted {
        Ok(admitted) => admitted,
        Err(error) => {
            let s = service.store.clone();
            let u = update.clone();
            let state = blocking(move || s.native_assistant_update_status(&u)).await;
            return match state {
                Ok(state) if !state.export_started => not_exported(service, &update, error).await,
                _ => (Err(error), true),
            };
        }
    };
    status.send_replace(AssistantVapiStatus::Updating {
        delivery_id: update.delivery_id().into(),
    });
    let mut truncated = false;
    let result=current_io(&selected,async {
        if stop.is_cancelled(){return Err(problem_error("cancelled"));}
        socket.send(Message::Text(json!({"type":"add-message","message":{"role":"system","content":content},"triggerResponseEnabled":false}).to_string().into())).await.map_err(|_|problem_error("provider-context-unconfirmed"))?;
        let body=initialization::acknowledge(socket,&content,applied.provider_ref(),capture.as_mut(),&mut truncated,ended,stop).await?;
        export.acknowledged(&body)
    }).await;
    drop(selected);
    match result {
        Ok(ack) => {
            let ack = Arc::new(ack);
            let recorded = retry(|| {
                let s = service.store.clone();
                let p = ack.clone();
                blocking(move || s.record_native_assistant_update_delivered(&p))
            })
            .await;
            match recorded {
                Ok(()) => (Ok(update.delivery_id().into()), false),
                Err(error) => (Err(error), true),
            }
        }
        Err(error) => {
            let recorded = if truncated {
                let rejected = Arc::new(export.rejected(AssistantContextFailure::Truncated));
                retry(|| {
                    let s = service.store.clone();
                    let p = rejected.clone();
                    blocking(move || s.record_native_assistant_update_rejected(&p))
                })
                .await
            } else if *ended {
                let export = Arc::new(export);
                retry(|| {
                    let s = service.store.clone();
                    let p = export.clone();
                    blocking(move || s.native_assistant_update_incomplete(&p))
                })
                .await
            } else {
                let export = Arc::new(export);
                retry(|| {
                    let s = service.store.clone();
                    let p = export.clone();
                    blocking(move || s.native_assistant_update_uncertain(&p))
                })
                .await
            };
            (Err(recorded.err().unwrap_or(error)), true)
        }
    }
}
