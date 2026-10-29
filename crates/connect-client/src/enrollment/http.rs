use super::*;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Separate bearer-free enrollment contact. HTTPS uses platform trust and an
/// optional explicit local root; plaintext is restricted to loopback hosts.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentClientConfig {
    pub origin: String,
    pub trust_certificate: Option<PathBuf>,
    pub timeout_ms: u64,
}
impl EnrollmentClientConfig {
    pub fn validate(&self) -> Result<()> {
        let url = reqwest::Url::parse(&self.origin).map_err(|_| ClientError::Configuration)?;
        let loopback = url.host_str().is_some_and(|h| {
            h == "localhost"
                || h.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        if self.origin.len() > 2048
            || url.origin().ascii_serialization() != self.origin
            || url.username() != ""
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.port() == Some(0)
            || !(1..=60_000).contains(&self.timeout_ms)
            || !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        {
            return Err(ClientError::Configuration);
        }
        Ok(())
    }
}
pub struct EnrollmentClient {
    config: EnrollmentClientConfig,
    http: reqwest::Client,
}
impl EnrollmentClient {
    pub async fn open(config: EnrollmentClientConfig) -> Result<Self> {
        config.validate()?;
        let root = config.trust_certificate.clone();
        let http = tokio::task::spawn_blocking(move || {
            let mut builder = reqwest::Client::builder()
                .no_proxy()
                .http1_only()
                .referer(false)
                .redirect(reqwest::redirect::Policy::none())
                .pool_max_idle_per_host(0);
            if let Some(path) = root {
                let meta = std::fs::metadata(&path).map_err(|_| ClientError::Configuration)?;
                if !meta.is_file() || meta.len() > 65_536 {
                    return Err(ClientError::Configuration);
                }
                let bytes = std::fs::read(path).map_err(|_| ClientError::Configuration)?;
                builder = builder.add_root_certificate(
                    reqwest::Certificate::from_pem(&bytes)
                        .map_err(|_| ClientError::Configuration)?,
                );
            }
            builder.build().map_err(|_| ClientError::Configuration)
        })
        .await
        .map_err(|_| ClientError::Configuration)??;
        Ok(Self { config, http })
    }
    /// One attempt only. Cancellation/timeout can follow a committed claim;
    /// load the saved enrollment and explicitly retry this same original offer.
    pub async fn claim(&self, enrollment: &NativeEnrollment) -> Result<EnrollmentCredential> {
        self.claim_with_cancel(enrollment, CancellationToken::new())
            .await
    }
    pub async fn claim_with_cancel(
        &self,
        enrollment: &NativeEnrollment,
        cancel: CancellationToken,
    ) -> Result<EnrollmentCredential> {
        if cancel.is_cancelled() {
            return Err(ClientError::Cancelled);
        }
        let claim = enrollment.claim()?;
        let body = serde_json::to_vec(&claim).map_err(|_| ClientError::Configuration)?;
        if body.len() > 65_536 {
            return Err(ClientError::Configuration);
        }
        let deadline = tokio::time::Instant::now() + Duration::from_millis(self.config.timeout_ms);
        let work = async {
            let mut response = self
                .http
                .post(format!(
                    "{}/v2/enrollments/{}/claim",
                    self.config.origin, claim.enrollment_id
                ))
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .header(reqwest::header::CONNECTION, "close")
                .header(reqwest::header::CACHE_CONTROL, "no-store")
                .body(body)
                .send()
                .await
                .map_err(|_| ClientError::Transport)?;
            let status = response.status();
            if status.is_redirection() || response.content_length().is_some_and(|n| n > 65_536) {
                return Err(ClientError::Protocol);
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| ClientError::Transport)? {
                if bytes.len().saturating_add(chunk.len()) > 65_536 {
                    return Err(ClientError::Protocol);
                }
                bytes.extend_from_slice(&chunk);
            }
            let response: EnrollmentResponse<EnrollmentCredential> =
                serde_json::from_slice(&bytes).map_err(|_| ClientError::Protocol)?;
            if response.profile != PROFILE {
                return Err(ClientError::Protocol);
            }
            let value = response.result.map_err(|error| {
                // The server's text/details may contain attacker-controlled or
                // secret input. Preserve only a closed, meaningful error code.
                let code = match error.code.as_str() {
                    "unauthorized" => "unauthorized",
                    "forbidden" => "forbidden",
                    "conflict" => "conflict",
                    "gone" => "gone",
                    "capacity-exceeded" => "capacity-exceeded",
                    "invalid-request" => "invalid-request",
                    "storage-unavailable" => "storage-unavailable",
                    _ => "enrollment-refused",
                };
                ClientError::Application(connect_protocol::Error::new(
                    code,
                    "Enrollment refused; preserve the saved original offer and key",
                ))
            })?;
            if status != reqwest::StatusCode::OK {
                return Err(ClientError::Protocol);
            }
            enrollment.check_credential(&value)?;
            Ok(value)
        };
        let result = tokio::select! {biased;_=cancel.cancelled()=>Err(ClientError::Cancelled),r=tokio::time::timeout_at(deadline,work)=>r.map_err(|_|ClientError::Timeout)?};
        if cancel.is_cancelled() {
            return Err(ClientError::Cancelled);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(ClientError::Timeout);
        }
        result
    }
}
