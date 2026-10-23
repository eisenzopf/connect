//! Invitation references are selectors, never network destinations or grants.
use super::*;

#[derive(Clone, PartialEq, Eq)]
pub enum Target {
    Member(String),
    Telephone {
        address: String,
    },
    Sip {
        address: String,
        authority: String,
        secure: bool,
    },
}
impl std::fmt::Debug for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionTarget(redacted)")
    }
}
impl Target {
    pub fn parse(value: &str) -> Result<Self> {
        if !value.contains(':') {
            id(value)?;
            return Ok(Self::Member(value.into()));
        }
        if value.len() > 2048 || !value.bytes().all(|b| (b'!'..=b'~').contains(&b)) {
            return Err(invalid());
        }
        if let Some(address) = value.strip_prefix("tel:") {
            if !(9..=16).contains(&address.len())
                || !address.starts_with('+')
                || address.as_bytes()[1] == b'0'
                || !address.as_bytes()[1..].iter().all(u8::is_ascii_digit)
            {
                return Err(invalid());
            }
            return Ok(Self::Telephone {
                address: address.into(),
            });
        }
        let (scheme, rest) = value.split_once(':').ok_or_else(invalid)?;
        if !matches!(scheme, "sip" | "sips") {
            return Err(Error::new(
                "unsupported-feature",
                "Session gateway scheme is unavailable",
            ));
        }
        let mut uri = url::Url::parse(&format!("{scheme}://{rest}")).map_err(|_| invalid())?;
        if uri.host_str().is_none()
            || uri.username().is_empty()
            || uri.password().is_some()
            || !uri.path().is_empty()
            || uri.query().is_some()
            || uri.fragment().is_some()
            || uri.port() == Some(0)
            || rest.to_ascii_lowercase().contains("%0d")
            || rest.to_ascii_lowercase().contains("%0a")
            || rest.to_ascii_lowercase().contains("%00")
        {
            return Err(invalid());
        }
        let host = uri.host_str().ok_or_else(invalid)?.to_ascii_lowercase();
        uri.set_host(Some(&host)).map_err(|_| invalid())?;
        let secure = scheme == "sips";
        let authority = authority(&uri, if secure { 5061 } else { 5060 })?;
        Ok(Self::Sip {
            address: uri.to_string().replacen("://", ":", 1),
            authority,
            secure,
        })
    }
}
fn authority(uri: &url::Url, default: u16) -> Result<String> {
    Ok(format!(
        "{}:{}",
        uri.host_str().ok_or_else(invalid)?.to_ascii_lowercase(),
        uri.port().unwrap_or(default)
    ))
}
/// Normalize a trusted configured next-hop authority for exact comparison.
pub fn sip_authority(value: &str) -> Result<String> {
    let uri = url::Url::parse(&format!("sip://route@{value}")).map_err(|_| invalid())?;
    if uri.host_str().is_none()
        || !uri.path().is_empty()
        || uri.query().is_some()
        || uri.fragment().is_some()
        || uri.password().is_some()
        || uri.port() == Some(0)
        || uri.username() != "route"
    {
        return Err(invalid());
    }
    authority(&uri, 5060)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invitation_selectors_keep_ids_distinct_from_bounded_gateway_uris() {
        assert!(matches!(
            Target::parse("part-one").unwrap(),
            Target::Member(_)
        ));
        assert_eq!(
            Target::parse("tel:+14155550123").unwrap(),
            Target::Telephone {
                address: "+14155550123".into()
            }
        );
        assert_eq!(
            Target::parse("sip:Bob@EXAMPLE.com").unwrap(),
            Target::Sip {
                address: "sip:Bob@example.com".into(),
                authority: "example.com:5060".into(),
                secure: false
            }
        );
        assert!(matches!(
            Target::parse("sips:bob@example.com").unwrap(),
            Target::Sip { secure: true, .. }
        ));
        for bad in [
            "tel:+0123456789",
            "tel:14155550123",
            "tel:+14155550123;ext=3",
            "sip:bob@example.com?x=y",
            "sip:bob:secret@example.com",
            "sip:bob@example.com/path",
            "sip:bob%0d@example.com",
            "sip:bob@example.com:0",
            "sip:bob@",
            "sip:bob\n@example.com",
        ] {
            assert!(Target::parse(bad).is_err(), "selector should refuse");
        }
        assert!(id("sip:bob@example.com").is_err());
        assert!(Target::parse(&format!("sip:{}@example.com", "a".repeat(2048))).is_err());
        assert_eq!(sip_authority("127.0.0.1:5099").unwrap(), "127.0.0.1:5099");
        assert_eq!(sip_authority("[::1]:5099").unwrap(), "[::1]:5099");
    }
}
