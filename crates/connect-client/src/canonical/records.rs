//! Explicit native record fetch. The digest names the exact authorized view;
//! neither the selector nor a previously fetched hash is a future read grant.
use super::calls::{invalid, typed};
use super::*;
use base64::Engine;
use sha2::Digest;
#[derive(Clone, Default)]
pub struct CanonicalRecordSelector {
    pub uuid: Option<String>,
    pub url: Option<String>,
    pub content_hash: Option<String>,
}
pub struct CanonicalRecord {
    pub uuid: String,
    pub body: String,
    pub content_hash: String,
}
impl std::fmt::Debug for CanonicalRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanonicalRecord")
            .field("body_bytes", &self.body.len())
            .finish_non_exhaustive()
    }
}
/// One bounded page over the shared Conversation snapshot. Other visible
/// topology items do not become records; continue with next_page until None.
#[derive(Clone, Debug)]
pub struct CanonicalRecordPage {
    pub records: Vec<connect_protocol::events::RecordAvailability>,
    pub next_page: Option<String>,
    pub cursor: String,
}
pub(super) fn discovery(page: payloads::ConversationSnapshot) -> Result<CanonicalRecordPage> {
    use connect_protocol::events::RecordAvailability;
    let mut records = Vec::new();
    for item in page.items {
        if item["kind"] != "record" {
            continue;
        }
        let value: RecordAvailability =
            serde_json::from_value(item["data"].clone()).map_err(|_| ClientError::Protocol)?;
        Identifier::try_from(value.session_id().to_owned()).map_err(|_| ClientError::Protocol)?;
        if let RecordAvailability::Ready { uuid, revision, .. } = &value {
            if uuid::Uuid::parse_str(uuid).is_err()
                || *revision == 0
                || *revision > 9_007_199_254_740_991
            {
                return Err(ClientError::Protocol);
            }
        }
        records.push(value);
    }
    if page.has_more != page.next_page.is_some() {
        return Err(ClientError::Protocol);
    }
    Ok(CanonicalRecordPage {
        records,
        next_page: page.next_page,
        cursor: page.cursor,
    })
}
macro_rules! record_calls {
    ($client:ty) => {
        impl $client {
            pub async fn fetch_record(
                &mut self,
                conversation: &str,
                session: &str,
                selector: CanonicalRecordSelector,
            ) -> Result<CanonicalRecord> {
                if selector.uuid.is_none() && selector.url.is_none()
                    || selector
                        .uuid
                        .as_ref()
                        .is_some_and(|id| uuid::Uuid::parse_str(id).is_err())
                    || selector.url.as_ref().is_some_and(|s| {
                        s.is_empty()
                            || s.len() > 2048
                            || s.chars().any(|c| c.is_control() || c.is_whitespace())
                    })
                    || selector
                        .content_hash
                        .as_ref()
                        .is_some_and(|s| s.is_empty() || s.len() > 512)
                {
                    return Err(invalid("Invalid local record selector"));
                }
                let deadline = self.record_deadline()?;
                let mut body = serde_json::Map::new();
                for (key, value) in [
                    ("uuid", &selector.uuid),
                    ("url", &selector.url),
                    ("content_hash", &selector.content_hash),
                ] {
                    if let Some(value) = value {
                        body.insert(key.into(), json!(value));
                    }
                }
                let batch = self
                    .request(
                        typed("recording.vcon-fetch", Value::Object(body))?,
                        CommandScope {
                            conversation_id: Some(conversation.into()),
                            session_id: Some(session.into()),
                            ..Default::default()
                        },
                    )
                    .await?;
                batch.ack()?;
                if batch.frames.len() != 2 {
                    return Err(ClientError::Protocol);
                }
                let mut matches = batch
                    .frames
                    .iter()
                    .filter(|e| e.kind() == "recording.vcon-fetched");
                let reply = matches
                    .next()
                    .filter(|_| matches.next().is_none())
                    .ok_or(ClientError::Protocol)?;
                let payload = &reply.original()["payload"];
                if tokio::time::Instant::now() >= deadline {
                    return Err(ClientError::Timeout);
                }
                if payload["delivery"] == "url" {
                    let bytes = self.download_record(payload, deadline).await?;
                    let body = std::str::from_utf8(&bytes).map_err(|_| ClientError::Protocol)?;
                    decode_body(payload, &selector, body)
                } else {
                    decode(payload, &selector)
                }
            }
        }
    };
}
record_calls!(CanonicalClient);
record_calls!(CanonicalHttpClient);

fn decode(payload: &Value, selector: &CanonicalRecordSelector) -> Result<CanonicalRecord> {
    if payload["delivery"] != "inline" {
        return Err(ClientError::Protocol);
    }
    let body = payload["vcon"]
        .as_str()
        .filter(|b| b.len() <= 128 * 1024)
        .ok_or(ClientError::Protocol)?;
    decode_body(payload, selector, body)
}
fn decode_body(
    payload: &Value,
    selector: &CanonicalRecordSelector,
    body: &str,
) -> Result<CanonicalRecord> {
    if body.len() > 16 * 1024 * 1024 {
        return Err(ClientError::Protocol);
    }
    let hash = payload["content_hash"]
        .as_str()
        .ok_or(ClientError::Protocol)?;
    let digest = format!(
        "sha512-{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(sha2::Sha512::digest(body.as_bytes()))
    );
    if digest != hash
        || selector
            .content_hash
            .as_ref()
            .is_some_and(|expected| expected != hash)
    {
        return Err(ClientError::Protocol);
    }
    let validated = rvoip_core::records::VconRecord::unattributed(body.as_bytes().to_vec().into())
        .map_err(|_| ClientError::Protocol)?;
    let id = validated.uuid();
    if selector
        .uuid
        .as_ref()
        .is_some_and(|expected| uuid::Uuid::parse_str(expected).ok() != Some(id))
    {
        return Err(ClientError::Protocol);
    }
    Ok(CanonicalRecord {
        uuid: id.to_string(),
        body: body.into(),
        content_hash: hash.into(),
    })
}

/// Explicit certificate pin for the separately bound HTTPS record endpoint.
#[derive(Clone)]
pub struct RecordTlsConfig {
    pub server_name: String,
    pub trust_certificate: PathBuf,
}
pub(super) struct RecordDownload {
    address: std::net::IpAddr,
    server_name: String,
    certificates: Vec<reqwest::Certificate>,
    pub(super) credential: String,
}
impl RecordDownload {
    pub(super) fn new(
        address: std::net::IpAddr,
        name: &str,
        der: &[u8],
        credential: String,
        profile: NativeNetworkProfile,
    ) -> Result<Self> {
        rustls::pki_types::ServerName::try_from(name.to_owned())
            .map_err(|_| ClientError::Configuration)?;
        if name.parse::<std::net::IpAddr>().is_ok_and(|ip| {
            NativeNetworkProfile::canonical_address(SocketAddr::new(ip, 1)).ip()
                != NativeNetworkProfile::canonical_address(SocketAddr::new(address, 1)).ip()
        }) || !profile.permits_endpoint(SocketAddr::new(address, 1))
            || name.is_empty()
            || name.len() > 253
            || name.contains(['/', '@', '?', '#'])
        {
            return Err(ClientError::Configuration);
        }
        Ok(Self {
            address,
            server_name: name.to_owned(),
            certificates: trust::certificates(der)?
                .iter()
                .map(|cert| {
                    reqwest::Certificate::from_der(cert.as_ref())
                        .map_err(|_| ClientError::Configuration)
                })
                .collect::<Result<Vec<_>>>()?,
            credential,
        })
    }
    fn url(&self, payload: &Value) -> Result<reqwest::Url> {
        let raw = payload["url"]
            .as_str()
            .filter(|s| s.len() <= 2048)
            .ok_or(ClientError::Protocol)?;
        let url = reqwest::Url::parse(raw).map_err(|_| ClientError::Protocol)?;
        let path = url
            .path()
            .strip_prefix("/v2/record-references/")
            .ok_or(ClientError::Protocol)?;
        let host = if self.server_name.contains(':') {
            format!("[{}]", self.server_name)
        } else {
            self.server_name.clone()
        };
        let expected = reqwest::Url::parse(&format!("https://{host}/"))
            .map_err(|_| ClientError::Configuration)?;
        if url.scheme() != "https"
            || url.host() != expected.host()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.port_or_known_default() == Some(0)
            || path.len() != 32
            || !path
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(ClientError::Protocol);
        }
        Ok(url)
    }
    async fn download(&self, payload: &Value, deadline: tokio::time::Instant) -> Result<Vec<u8>> {
        let url = self.url(payload)?;
        let expires: chrono::DateTime<chrono::Utc> =
            serde_json::from_value(payload["expires_at"].clone())
                .map_err(|_| ClientError::Protocol)?;
        let remaining = (expires - chrono::Utc::now())
            .to_std()
            .map_err(|_| ClientError::Timeout)?;
        let deadline = deadline.min(tokio::time::Instant::now() + remaining);
        let mut builder = reqwest::Client::builder().tls_built_in_root_certs(false);
        for certificate in &self.certificates {
            builder = builder.add_root_certificate(certificate.clone());
        }
        let client = builder
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .http1_only()
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .resolve(
                url.host_str().ok_or(ClientError::Protocol)?,
                SocketAddr::new(
                    self.address,
                    url.port_or_known_default().ok_or(ClientError::Protocol)?,
                ),
            )
            .build()
            .map_err(|_| ClientError::Configuration)?;
        tokio::time::timeout_at(deadline, async {
            let mut response = client
                .get(url)
                .bearer_auth(&self.credential)
                .header(reqwest::header::ACCEPT_ENCODING, "identity")
                .send()
                .await
                .map_err(|_| ClientError::Transport)?;
            if response.status() != reqwest::StatusCode::OK {
                return Err(remote_error(response.status().as_u16() as u64));
            }
            if response.version() != reqwest::Version::HTTP_11
                || response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    != Some("application/vcon+json")
                || response
                    .headers()
                    .contains_key(reqwest::header::CONTENT_ENCODING)
                || response
                    .content_length()
                    .is_some_and(|n| n > 16 * 1024 * 1024)
            {
                return Err(ClientError::Protocol);
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| ClientError::Transport)? {
                if bytes.len().saturating_add(chunk.len()) > 16 * 1024 * 1024 {
                    return Err(ClientError::Protocol);
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(bytes)
        })
        .await
        .map_err(|_| ClientError::Timeout)?
    }
}
fn deadline(
    expires: chrono::DateTime<chrono::Utc>,
    timeout: Duration,
) -> Result<tokio::time::Instant> {
    let remaining = (expires - chrono::Utc::now())
        .to_std()
        .map_err(|_| ClientError::Authentication)?;
    Ok(tokio::time::Instant::now() + timeout.min(remaining))
}
impl CanonicalClient {
    fn record_deadline(&self) -> Result<tokio::time::Instant> {
        if self.closed.load(Ordering::Acquire) || self.connection.close_reason().is_some() {
            return Err(ClientError::Transport);
        }
        deadline(
            self.authenticated_until.min(self.server_key_expires_at),
            self.deadline,
        )
    }
    async fn download_record(&self, payload: &Value, end: tokio::time::Instant) -> Result<Vec<u8>> {
        self.record_deadline()?;
        self.record_download.download(payload, end).await
    }
}
impl CanonicalHttpClient {
    fn record_deadline(&self) -> Result<tokio::time::Instant> {
        let config = self.application.record_config()?;
        deadline(config.expires_at, Duration::from_millis(config.timeout_ms))
    }
    async fn download_record(&self, payload: &Value, end: tokio::time::Instant) -> Result<Vec<u8>> {
        self.record_deadline()?;
        self.record_download
            .as_ref()
            .ok_or(ClientError::Configuration)?
            .download(payload, end)
            .await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    fn payload() -> (Value, CanonicalRecordSelector) {
        let mut id = *uuid::Uuid::new_v4().as_bytes();
        id[6] = (id[6] & 15) | 128;
        let id = uuid::Uuid::from_bytes(id);
        let body=serde_json::to_string(&json!({"vcon":"0.4.0","uuid":id,"created_at":chrono::Utc::now(),"subject":"permitted λ"})).unwrap();
        let digest = format!(
            "sha512-{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(sha2::Sha512::digest(body.as_bytes()))
        );
        (
            json!({"delivery":"inline","vcon":body,"content_hash":digest}),
            CanonicalRecordSelector {
                uuid: Some(id.to_string()),
                ..Default::default()
            },
        )
    }
    #[test]
    fn record_reference_origin_refuses_bearer_exfiltration_and_ambiguous_paths() {
        let certificate = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let target = RecordDownload::new(
            "127.0.0.1".parse().unwrap(),
            "localhost",
            certificate.cert.der().as_ref(),
            "protected-current-bearer".into(),
            NativeNetworkProfile::Local,
        )
        .unwrap();
        assert!(RecordDownload::new(
            "127.0.0.2".parse().unwrap(),
            "127.0.0.1",
            certificate.cert.der().as_ref(),
            "protected-current-bearer".into(),
            NativeNetworkProfile::Local,
        )
        .is_err());
        let path = "/v2/record-references/0123456789abcdef0123456789abcdef";
        assert!(target
            .url(&json!({"url":format!("https://localhost:1234{path}")}))
            .is_ok());
        for raw in [
            format!("http://localhost:1234{path}"),
            format!("https://other.invalid{path}"),
            format!("https://localhost.attacker.invalid{path}"),
            format!("https://user@localhost{path}"),
            format!("https://localhost{path}?bearer=secret"),
            format!("https://localhost{path}#fragment"),
            format!("https://localhost:0{path}"),
            format!("https://localhost{path}/extra"),
            "https://localhost/v2/record-references/%30%31".into(),
        ] {
            assert!(target.url(&json!({"url":raw})).is_err());
        }
    }
    #[test]
    fn record_decoder_requires_exact_view_bytes_digest_and_selected_uuid() {
        let (mut value, selector) = payload();
        let record = decode(&value, &selector).unwrap();
        assert_eq!(record.body, value["vcon"].as_str().unwrap());
        value["vcon"] = json!(format!("{} ", record.body));
        assert!(decode(&value, &selector).is_err());
        let (value, mut selector) = payload();
        selector.uuid = Some(uuid::Uuid::new_v4().to_string());
        assert!(decode(&value, &selector).is_err());
    }
    #[test]
    fn record_decoder_rejects_changed_condition_reference_and_oversize_without_fetching() {
        let (mut value, mut selector) = payload();
        selector.content_hash = Some("sha512-other-view".into());
        assert!(decode(&value, &selector).is_err());
        selector.content_hash = None;
        value["delivery"] = json!("url");
        value["url"] = json!("https://127.0.0.1:1/secret-reference");
        assert!(decode(&value, &selector).is_err());
        value["delivery"] = json!("inline");
        value["vcon"] = json!("x".repeat(128 * 1024 + 1));
        assert!(decode(&value, &selector).is_err());
    }
    #[test]
    fn record_decoder_does_not_accept_schema_invalid_json_with_a_valid_digest() {
        let (mut value, selector) = payload();
        let body =
            serde_json::to_string(&json!({"uuid":selector.uuid,"vcon":"invalid-draft"})).unwrap();
        value["content_hash"] = json!(format!(
            "sha512-{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(sha2::Sha512::digest(body.as_bytes()))
        ));
        value["vcon"] = json!(body);
        assert!(decode(&value, &selector).is_err());
    }
    #[test]
    fn record_discovery_rejects_unbounded_or_malformed_ready_metadata() {
        fn page(item: Value) -> payloads::ConversationSnapshot {
            serde_json::from_value(
                json!({"revision":1,"cursor":"cursor","items":[item],"has_more":false}),
            )
            .unwrap()
        }
        let id = uuid::Uuid::new_v4().to_string();
        assert_eq!(discovery(page(json!({"kind":"record","data":{"state":"ready","session_id":"session","uuid":id,"revision":1}}))).unwrap().records.len(),1);
        for data in [
            json!({"state":"ready","session_id":"session","uuid":"bad-id","revision":1}),
            json!({"state":"ready","session_id":"session","uuid":id,"revision":0}),
            json!({"state":"ready","session_id":"session","uuid":id,"revision":1,"canonical_hash":"secret"}),
            json!({"state":"pending","session_id":"session","uuid":id}),
            json!({"state":"unavailable","session_id":"session","url":"connect:vcon/private"}),
        ] {
            assert!(discovery(page(json!({"kind":"record","data":data}))).is_err());
        }
    }
}
