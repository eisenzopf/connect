//! Bounded, authenticated enrollment synchronization. Provider work remains local;
//! an expired or absent authority snapshot can never authorize an SMS.
use async_trait::async_trait;
use connect_store::{SmsGrantSnapshot, Store};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};

/// Trusted server composition supplies an authenticated complete enrollment snapshot.
/// Connect remains the one dispatch authority for leases, consent, STOP and replay.
#[async_trait]
pub trait SmsPolicy: Send + Sync {
    /// Stable authority identity, never selected by an agent or incoming message.
    fn source(&self) -> &str;
    async fn snapshot(&self) -> anyhow::Result<SmsGrantSnapshot>;
}
/// Synchronize one policy through the existing atomic store boundary. Failure never
/// extends a stale lease or clears durable STOP state.
pub async fn synchronize(policy: &dyn SmsPolicy, store: Arc<Store>) -> anyhow::Result<()> {
    let snapshot = policy.snapshot().await?;
    let source = policy.source().to_owned();
    tokio::task::spawn_blocking(move || store.refresh_sms_grants(&source, &snapshot)).await??;
    Ok(())
}

pub struct Authority {
    client: reqwest::Client,
    url: reqwest::Url,
    token: String,
    source: String,
}
impl Authority {
    pub(crate) fn from_env() -> anyhow::Result<Option<Self>> {
        let Some(url) = std::env::var("CONNECT_SMS_AUTHORITY_URL").ok() else {
            return Ok(None);
        };
        let token = crate::secrets::resolve("CONNECT_SMS_AUTHORITY_TOKEN")?
            .expose()
            .to_owned();
        Self::new(&url, token).map(Some)
    }
    pub fn new(url: &str, token: String) -> anyhow::Result<Self> {
        let url =
            reqwest::Url::parse(url).map_err(|_| anyhow::anyhow!("Invalid SMS authority URL"))?;
        let loopback = url.host_str().is_some_and(|h| {
            h == "localhost"
                || h.parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        anyhow::ensure!(
            (url.scheme() == "https" || (url.scheme() == "http" && loopback))
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && token.len() >= 32,
            "SMS authority requires HTTPS (or local HTTP) and a private bearer token"
        );
        let source = hex::encode(Sha256::digest(url.as_str().as_bytes()));
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            client,
            url,
            token,
            source,
        })
    }
    #[cfg(test)]
    pub(crate) async fn refresh(&self, store: Arc<Store>) -> anyhow::Result<()> {
        synchronize(self, store).await
    }
}
#[async_trait]
impl SmsPolicy for Authority {
    fn source(&self) -> &str {
        &self.source
    }
    async fn snapshot(&self) -> anyhow::Result<SmsGrantSnapshot> {
        let mut response = self
            .client
            .get(self.url.clone())
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("SMS authority unavailable"))?;
        anyhow::ensure!(
            response.status().is_success(),
            "SMS authority rejected request"
        );
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow::anyhow!("SMS authority response failed"))?
        {
            anyhow::ensure!(
                body.len().saturating_add(chunk.len()) <= 8 * 1024 * 1024,
                "SMS authority response exceeds limit"
            );
            body.extend_from_slice(&chunk);
        }
        let snapshot: SmsGrantSnapshot = serde_json::from_slice(&body)
            .map_err(|_| anyhow::anyhow!("Invalid SMS authority snapshot"))?;
        Ok(snapshot)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reject_unsafe_authority_configuration() {
        for url in [
            "http://example.com/feed",
            "https://u:p@example.com/feed",
            "https://example.com/feed?token=secret",
            "https://example.com/feed#secret",
        ] {
            assert!(Authority::new(url, "x".repeat(32)).is_err());
        }
        assert!(Authority::new("https://example.com/feed", "short".into()).is_err());
        assert!(Authority::new("http://127.0.0.1:1234/feed", "x".repeat(32)).is_ok());
    }
}

#[cfg(test)]
mod synchronization_tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use wiremock::{
        matchers::{header, method, path},
        Mock, MockServer, ResponseTemplate,
    };
    #[tokio::test]
    async fn authenticated_snapshot_rejects_redirects_and_revision_replay() {
        let remote = MockServer::start().await;
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::initialize(&temp.path().join("state.sqlite3"), Default::default()).unwrap(),
        );
        let authority = Authority::new(&format!("{}/feed", remote.uri()), "x".repeat(32)).unwrap();
        let accepted = Mock::given(method("GET"))
            .and(path("/feed"))
            .and(header(
                "authorization",
                format!("Bearer {}", "x".repeat(32)),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"revision":2,"grants":[]})),
            )
            .expect(1)
            .mount_as_scoped(&remote)
            .await;
        authority.refresh(store.clone()).await.unwrap();
        drop(accepted);
        let stale = Mock::given(path("/feed"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"revision":1,"grants":[]})),
            )
            .mount_as_scoped(&remote)
            .await;
        assert!(authority.refresh(store.clone()).await.is_err());
        drop(stale);
        let redirect = Mock::given(path("/feed"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "/credentials-leak"))
            .mount_as_scoped(&remote)
            .await;
        assert!(authority.refresh(store.clone()).await.is_err());
        drop(redirect);
        let bad = Mock::given(path("/feed"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not a snapshot"))
            .mount_as_scoped(&remote)
            .await;
        assert!(authority.refresh(store).await.is_err());
        drop(bad);
        assert!(!remote
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.url.path() == "/credentials-leak"));
    }
}
