//! Explicit bounded HTTP canonical transport. It never falls back to native or
//! an older application operation dialect.
use super::*;
use crate::application::{ApplicationClient, ApplicationClientConfig};
use connect_protocol::canonical_http as contract;
use tokio_util::sync::CancellationToken;

pub struct CanonicalHttpClient {
    pub(super) application: ApplicationClient,
    pub(super) record_download: Option<records::RecordDownload>,
}
impl CanonicalHttpClient {
    pub async fn open(config: ApplicationClientConfig) -> Result<Self> {
        Ok(Self {
            application: ApplicationClient::open(config).await?,
            record_download: None,
        })
    }
    /// Pin the HTTPS record origin explicitly before following reference replies.
    pub fn with_record_tls(mut self, config: RecordTlsConfig) -> Result<Self> {
        let app = self.application.record_config()?;
        let certificate = bounded_file(&config.trust_certificate, trust::MAX_TRUST_BYTES)?;
        self.record_download = Some(records::RecordDownload::new(
            app.endpoint.ip(),
            &config.server_name,
            &certificate,
            app.credential.clone(),
            NativeNetworkProfile::Local,
        )?);
        Ok(self)
    }
    pub async fn request(&mut self, payload: Payload, scope: CommandScope) -> Result<ReplyBatch> {
        self.request_with_cancel(payload, scope, CancellationToken::new())
            .await
    }
    pub async fn request_with_cancel(
        &mut self,
        payload: Payload,
        scope: CommandScope,
        cancellation: CancellationToken,
    ) -> Result<ReplyBatch> {
        self.request_value_with_cancel(
            payload.kind(),
            serde_json::to_value(&payload).map_err(|_| ClientError::Protocol)?["payload"].clone(),
            scope,
            cancellation,
        )
        .await
    }
    /// Validate and send original payload values, retaining optional extensions.
    pub async fn request_value(
        &mut self,
        kind: &str,
        payload: Value,
        scope: CommandScope,
    ) -> Result<ReplyBatch> {
        self.request_value_with_cancel(
            kind,
            payload,
            scope,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
    }
    pub async fn request_value_with_cancel(
        &mut self,
        kind: &str,
        payload: Value,
        scope: CommandScope,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<ReplyBatch> {
        let create_scope = matches!(kind, "conversation.create" | "conversation.list");
        let value = command_value(kind, payload, scope.clone())?;
        let id = value["id"]
            .as_str()
            .ok_or(ClientError::Protocol)?
            .to_owned();
        let body = serde_json::to_vec(&value).map_err(|_| ClientError::Protocol)?;
        self.application
            .transact(
                "/v2/commands",
                body,
                0,
                cancellation,
                move |status, bytes| {
                    if let Ok(rejection) = serde_json::from_slice::<contract::Rejection>(bytes) {
                        if rejection.profile != contract::PROFILE
                            || status.as_u16() != rejection.problem.code
                        {
                            return Err(ClientError::Protocol);
                        }
                        return Err(remote_error(rejection.problem.code as u64));
                    }
                    let response: contract::ReplyBatch =
                        serde_json::from_slice(bytes).map_err(|_| ClientError::Protocol)?;
                    if response.profile != contract::PROFILE
                        || response.request_id != id
                        || response.envelopes.is_empty()
                        || response.envelopes.len() > MAX_BATCH_FRAMES
                    {
                        return Err(ClientError::Protocol);
                    }
                    let mut frames = Vec::with_capacity(response.envelopes.len());
                    for value in response.envelopes {
                        let envelope =
                            Envelope::from_value(value).map_err(|_| ClientError::Protocol)?;
                        if envelope.original().get("signature").is_some()
                            || envelope.scope(Scope::Reply) != Some(id.as_str())
                            || frames
                                .last()
                                .is_some_and(|f: &Envelope| matches!(f.kind(), "ack" | "error"))
                        {
                            return Err(ClientError::Protocol);
                        }
                        check_reply_scope(&envelope, &scope, create_scope)?;
                        frames.push(envelope);
                    }
                    let last = frames.last().ok_or(ClientError::Protocol)?;
                    let expected = if last.kind() == "error" {
                        last.original()["payload"]["code"]
                            .as_u64()
                            .ok_or(ClientError::Protocol)?
                    } else if last.kind() == "ack" {
                        200
                    } else {
                        return Err(ClientError::Protocol);
                    };
                    if status.as_u16() as u64 != expected {
                        return Err(ClientError::Protocol);
                    }
                    Ok(ReplyBatch {
                        request_id: id,
                        frames,
                    })
                },
            )
            .await
    }
}
