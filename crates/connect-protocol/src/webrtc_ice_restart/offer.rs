//! Conservative restart-only comparison against the original peer's last
//! accepted remote offer. This grants neither authority nor ICE readiness.
use super::{invalid, validate_sdp};
use crate::Result;
use std::net::IpAddr;

const MAX_MEDIA: usize = 32;
const MAX_CANDIDATES: usize = 128;
const MAX_LINE_BYTES: usize = 8192;

#[derive(Default)]
struct Section<'a> {
    ufrag: Option<&'a str>,
    password: Option<&'a str>,
    fingerprint: Option<&'a str>,
    setup: Option<&'a str>,
    end: bool,
    active: bool,
}
struct Offer<'a> {
    projection: Vec<String>,
    version: u64,
    credentials: Vec<(&'a str, &'a str)>,
}

/// Compare an offer with trusted, retained SDP from the same original peer.
/// Only ICE credentials/candidates, numeric transport addresses/ports and the
/// increasing origin version may change. Every other line (including unknown
/// extensions, DTLS identity, codecs, source IDs, MID/BUNDLE and direction) must
/// remain in its original order. This deliberately refuses wider renegotiation.
///
/// The RTC engine must still parse/apply the full offer under current authority
/// and observe this attempt's new ICE generation; this is no readiness receipt.
pub fn validate_offer_change(previous: &str, proposed: &str) -> Result<()> {
    let previous = parse(previous)?;
    let proposed = parse(proposed)?;
    if previous.projection != proposed.projection
        || proposed.version <= previous.version
        || previous.credentials.len() != proposed.credentials.len()
        || previous
            .credentials
            .iter()
            .zip(&proposed.credentials)
            .any(|((old_u, old_p), (new_u, new_p))| old_u == new_u || old_p == new_p)
    {
        return Err(invalid());
    }
    Ok(())
}

fn put<'a>(slot: &mut Option<&'a str>, value: &'a str) -> Result<()> {
    if slot.replace(value).is_some() {
        return Err(invalid());
    }
    Ok(())
}
fn ice(value: &str, minimum: usize) -> Result<()> {
    if !(minimum..=256).contains(&value.len())
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
    {
        return Err(invalid());
    }
    Ok(())
}
fn port(value: &str) -> Result<u16> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    value.parse().map_err(|_| invalid())
}
fn address(parts: &[&str]) -> Result<()> {
    if parts.len() != 3 || parts[0] != "IN" {
        return Err(invalid());
    }
    match (parts[1], parts[2].parse::<IpAddr>()) {
        ("IP4", Ok(IpAddr::V4(_))) | ("IP6", Ok(IpAddr::V6(_))) => Ok(()),
        _ => Err(invalid()),
    }
}
fn parse(sdp: &str) -> Result<Offer<'_>> {
    validate_sdp(sdp)?;
    if !sdp.ends_with("\r\n") {
        return Err(invalid());
    }
    let mut sections = vec![Section::default()];
    let mut projection = Vec::new();
    let mut version = None;
    let mut candidates = 0;
    for line in sdp[..sdp.len() - 2].split("\r\n") {
        if line.len() < 2
            || line.len() > MAX_LINE_BYTES
            || line.as_bytes()[1] != b'='
            || !line.as_bytes()[0].is_ascii_lowercase()
            || line.chars().any(char::is_control)
        {
            return Err(invalid());
        }
        if let Some(value) = line.strip_prefix("m=") {
            if sections.len() > MAX_MEDIA {
                return Err(invalid());
            }
            let mut fields: Vec<_> = value.split(' ').collect();
            if fields.len() < 4 || fields.iter().any(|v| v.is_empty()) {
                return Err(invalid());
            }
            let active = port(fields[1])? != 0;
            fields[1] = if active { "active-port" } else { "0" };
            projection.push(format!("m={}", fields.join(" ")));
            sections.push(Section {
                active,
                ..Default::default()
            });
            continue;
        }
        let media = sections.len() > 1;
        let section = sections.last_mut().ok_or_else(invalid)?;
        if let Some(value) = line.strip_prefix("o=") {
            let mut fields: Vec<_> = value.split(' ').collect();
            if media
                || version.is_some()
                || fields.len() != 6
                || fields.iter().any(|v| v.is_empty())
            {
                return Err(invalid());
            }
            if !fields[2].bytes().all(|b| b.is_ascii_digit()) {
                return Err(invalid());
            }
            version = Some(fields[2].parse::<u64>().map_err(|_| invalid())?);
            fields[2] = "origin-version";
            projection.push(format!("o={}", fields.join(" ")));
        } else if let Some(value) = line.strip_prefix("c=") {
            address(&value.split(' ').collect::<Vec<_>>())?;
            projection.push("c=transport-address".into());
        } else if let Some(value) = line.strip_prefix("a=rtcp:") {
            let fields: Vec<_> = value.split(' ').collect();
            port(fields[0])?;
            if fields.len() != 1 {
                address(&fields[1..])?;
            }
            if !media {
                return Err(invalid());
            }
            projection.push("a=rtcp:transport-address".into());
        } else if let Some(value) = line.strip_prefix("a=ice-ufrag:") {
            ice(value, 4)?;
            put(&mut section.ufrag, value)?;
            projection.push("a=ice-ufrag:restart".into());
        } else if let Some(value) = line.strip_prefix("a=ice-pwd:") {
            ice(value, 22)?;
            put(&mut section.password, value)?;
            projection.push("a=ice-pwd:restart".into());
        } else if let Some(value) = line.strip_prefix("a=candidate:") {
            // Candidate grammar/ICE policy belongs to the RTC engine. Only
            // bounded printable media-scoped candidate lines are omitted from
            // this immutable-media comparison; no other attribute is omitted.
            candidates += 1;
            if !media
                || candidates > MAX_CANDIDATES
                || value.is_empty()
                || !value.bytes().all(|b| b.is_ascii_graphic() || b == b' ')
            {
                return Err(invalid());
            }
        } else if line == "a=end-of-candidates" {
            if !media || section.end {
                return Err(invalid());
            }
            section.end = true;
        } else {
            if let Some(value) = line.strip_prefix("a=fingerprint:") {
                put(&mut section.fingerprint, value)?;
            }
            if let Some(value) = line.strip_prefix("a=setup:") {
                put(&mut section.setup, value)?;
            }
            projection.push(line.to_owned());
        }
    }
    let session = &sections[0];
    let mut credentials = Vec::new();
    for section in sections.iter().skip(1).filter(|s| s.active) {
        let ufrag = section.ufrag.or(session.ufrag).ok_or_else(invalid)?;
        let password = section.password.or(session.password).ok_or_else(invalid)?;
        if section.fingerprint.or(session.fingerprint).is_none()
            || section.setup.or(session.setup).is_none()
        {
            return Err(invalid());
        }
        credentials.push((ufrag, password));
    }
    if credentials.is_empty() {
        return Err(invalid());
    }
    Ok(Offer {
        projection,
        version: version.ok_or_else(invalid)?,
        credentials,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn original() -> String {
        [
            "v=0",
            "o=- 123 4 IN IP4 127.0.0.1",
            "s=-",
            "t=0 0",
            "a=group:BUNDLE 0",
            "m=audio 9 UDP/TLS/RTP/SAVPF 111",
            "c=IN IP4 0.0.0.0",
            "a=rtcp:9 IN IP4 0.0.0.0",
            "a=ice-ufrag:oldU",
            "a=ice-pwd:oldPassword12345678901234",
            "a=ice-options:trickle",
            "a=fingerprint:sha-256 OLD-DTLS-IDENTITY",
            "a=setup:actpass",
            "a=mid:0",
            "a=sendrecv",
            "a=rtcp-mux",
            "a=rtpmap:111 opus/48000/2",
            "a=ssrc:123 cname:source",
            "a=msid:original track",
            "a=candidate:1 1 udp 123 192.0.2.1 1234 typ host",
            "a=end-of-candidates",
        ]
        .join("\r\n")
            + "\r\n"
    }
    fn restart() -> String {
        original()
            .replace("123 4 IN", "123 5 IN")
            .replace("oldU", "newU")
            .replace("oldPassword", "newPassword")
    }
    #[test]
    fn fresh_credentials_and_transport_address_changes_preserve_media_identity() {
        let next = restart()
            .replace("192.0.2.1 1234", "198.51.100.8 5678")
            .replace("m=audio 9", "m=audio 5678")
            .replace("c=IN IP4 0.0.0.0", "c=IN IP6 ::")
            .replace("a=rtcp:9 IN IP4 0.0.0.0", "a=rtcp:5679 IN IP6 ::");
        validate_offer_change(&original(), &next).unwrap();
    }
    #[test]
    fn both_credentials_and_an_increasing_origin_version_are_required() {
        for next in [
            original(),
            restart().replace("newU", "oldU"),
            restart().replace("newPassword", "oldPassword"),
            restart().replace("123 5 IN", "123 4 IN"),
            restart().replace("123 5 IN", "123 3 IN"),
            restart().replace("123 5 IN", "123 18446744073709551616 IN"),
        ] {
            assert!(validate_offer_change(&original(), &next).is_err());
        }
    }
    #[test]
    fn dtls_codecs_gates_source_ids_mids_and_unknown_extensions_cannot_change() {
        for (from, to) in [
            ("OLD-DTLS-IDENTITY", "NEW-DTLS-IDENTITY"),
            ("actpass", "active"),
            ("sendrecv", "inactive"),
            ("opus/48000/2", "PCMU/8000"),
            ("ssrc:123", "ssrc:456"),
            ("msid:original", "msid:other"),
            ("mid:0", "mid:1"),
            ("BUNDLE 0", "BUNDLE 1"),
            ("rtcp-mux", "rtcp-mux-only"),
            ("123 5 IN", "456 5 IN"),
            ("SAVPF 111", "SAVPF 0"),
            ("m=audio 9", "m=audio 0"),
            ("m=audio", "m=video"),
        ] {
            assert!(
                validate_offer_change(&original(), &restart().replace(from, to)).is_err(),
                "{from}"
            );
        }
        assert!(
            validate_offer_change(&original(), &(restart() + "a=future-extension:new\r\n"))
                .is_err()
        );
        let old = original() + "a=future-extension:retained\r\n";
        validate_offer_change(&old, &(restart() + "a=future-extension:retained\r\n")).unwrap();
    }
    #[test]
    fn duplicate_credentials_identity_and_ambiguous_lines_refuse() {
        for extra in [
            "a=ice-ufrag:newU",
            "a=ice-pwd:newPassword12345678901234",
            "a=fingerprint:sha-256 OTHER",
            "a=setup:actpass",
            "a=end-of-candidates",
            "o=- 123 6 IN IP4 127.0.0.1",
            "a=ice-ufrag:bad value",
            "c=IN IP4 example.org",
            "a=rtcp:65536",
            "a=rtcp:9 IN IP6 192.0.2.1",
            "a=hidden\ncontrol",
        ] {
            assert!(
                validate_offer_change(&original(), &(restart() + extra + "\r\n")).is_err(),
                "{extra}"
            );
        }
        for next in [
            restart().replace("\r\n", "\n"),
            restart().trim_end().to_owned(),
            restart().replace("newPassword12345678901234", "short"),
            restart().replace("newU", "bad!"),
        ] {
            assert!(validate_offer_change(&original(), &next).is_err());
        }
    }
    #[test]
    fn inherited_session_credentials_and_each_active_media_section_are_checked() {
        let old = original()
            .replace(
                "m=audio",
                "a=ice-ufrag:sessionU\r\na=ice-pwd:sessionPassword1234567890\r\nm=audio",
            )
            .replace("a=ice-ufrag:oldU\r\n", "")
            .replace("a=ice-pwd:oldPassword12345678901234\r\n", "");
        let new = old
            .replace("123 4 IN", "123 5 IN")
            .replace("sessionU", "changedU")
            .replace("sessionPassword", "changedPassword");
        validate_offer_change(&old, &new).unwrap();
        let media = "m=video 9 UDP/TLS/RTP/SAVPF 96\r\na=mid:1\r\na=sendrecv\r\na=fingerprint:sha-256 SAME\r\na=setup:actpass\r\na=ice-ufrag:videoU\r\na=ice-pwd:videoPassword123456789012\r\n";
        assert!(validate_offer_change(&(original() + media), &(restart() + media)).is_err());
        let new_media = media
            .replace("videoU", "freshU")
            .replace("videoPassword", "freshPassword");
        validate_offer_change(&(original() + media), &(restart() + &new_media)).unwrap();
    }
    #[test]
    fn candidate_count_media_count_and_line_capacity_are_bounded() {
        let candidate = "a=candidate:1 1 udp 123 192.0.2.1 1234 typ host\r\n";
        assert!(validate_offer_change(
            &original(),
            &(restart() + &candidate.repeat(MAX_CANDIDATES))
        )
        .is_err());
        assert!(validate_offer_change(
            &original(),
            &(restart() + "a=candidate:" + &"x".repeat(MAX_LINE_BYTES) + "\r\n")
        )
        .is_err());
        assert!(validate_offer_change(
            &original(),
            &(restart() + &"m=audio 0 UDP/TLS/RTP/SAVPF 111\r\n".repeat(MAX_MEDIA))
        )
        .is_err());
        let old = original().replace("a=candidate:1 1 udp 123 192.0.2.1 1234 typ host\r\n", "");
        let next = restart().replace("a=candidate:1 1 udp 123 192.0.2.1 1234 typ host\r\n", "");
        validate_offer_change(&old, &next).unwrap(); // Gathering/ICE readiness is a separate boundary.
    }
    #[test]
    fn missing_original_identity_or_effective_credentials_and_all_disabled_media_refuse() {
        for line in [
            "a=fingerprint:sha-256 OLD-DTLS-IDENTITY\r\n",
            "a=setup:actpass\r\n",
            "a=ice-ufrag:oldU\r\n",
            "a=ice-pwd:oldPassword12345678901234\r\n",
            "o=- 123 4 IN IP4 127.0.0.1\r\n",
        ] {
            let old = original().replace(line, "");
            assert!(validate_offer_change(&old, &restart()).is_err());
        }
        let old = original().replace("m=audio 9", "m=audio 0");
        assert!(validate_offer_change(&old, &restart().replace("m=audio 9", "m=audio 0")).is_err());
    }
}
