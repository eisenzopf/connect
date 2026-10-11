//! Explicit operator endpoint shared by the private SDK and media adapter.
use anyhow::{ensure, Result};

/// Read the optional operator-selected Vapi API origin. The default is Vapi's
/// public API; an explicit loopback HTTP origin supports isolated installations
/// and local rehearsals. The pinned private SDK supports HTTP on 127.0.0.1
/// or localhost; IPv6 HTTP origins are rejected consistently here. No request
/// is made and no credential is returned.
pub fn configured_api_base() -> Result<reqwest::Url> {
    let value = match std::env::var("CONNECT_VAPI_API_BASE_URL") {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => "https://api.vapi.ai/".into(),
        Err(_) => anyhow::bail!("Invalid Vapi API origin configuration"),
    };
    validate(&value)
}

fn validate(value: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(value)
        .map_err(|_| anyhow::anyhow!("Invalid Vapi API origin configuration"))?;
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/"
            && url.host_str().is_some(),
        "Vapi API configuration requires an origin without selectors or credentials"
    );
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost"));
    ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && loopback),
        "Vapi API configuration requires HTTPS or explicit loopback HTTP"
    );
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::validate;
    #[test]
    fn accepts_secure_or_explicit_loopback_origins_only() {
        for value in [
            "https://api.vapi.ai",
            "https://provider.example.invalid:8443/",
            "http://127.0.0.1:1234",
            "http://localhost:1234/",
        ] {
            assert!(validate(value).is_ok(), "{value}");
        }
        for value in [
            "",
            "not a url",
            "ftp://localhost/",
            "http://example.invalid/",
            "http://0.0.0.0/",
            "http://[::1]:1234/",
            "https://user@example.invalid/",
            "https://user:canary@example.invalid/",
            "https://example.invalid/call",
            "https://example.invalid/?token=canary",
            "https://example.invalid/#canary",
        ] {
            let error = validate(value).expect_err("unsafe origin").to_string();
            assert!(!error.contains("canary"));
        }
    }
}
