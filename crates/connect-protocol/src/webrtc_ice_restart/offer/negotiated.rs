//! Offer admission is only a candidate check. A removed header extension must
//! separately be proved declined by the SAME peer's trusted original answer.
use super::{invalid, parse, MAX_MEDIA};
use crate::Result;

/// Check immutable offer shape and fresh ICE credentials. This permits only
/// removal of media-level extmap lines; it is NOT authorization for RTC IO.
/// Call validate_negotiated_offer_change with the trusted actual answer first.
pub fn validate_restart_candidate(previous: &str, proposed: &str) -> Result<()> {
    candidate(previous, proposed).map(|_| ())
}

fn candidate(previous: &str, proposed: &str) -> Result<Vec<(usize, String)>> {
    let previous = parse(previous)?;
    let proposed = parse(proposed)?;
    if proposed.version <= previous.version
        || previous.credentials.len() != proposed.credentials.len()
        || previous
            .credentials
            .iter()
            .zip(&proposed.credentials)
            .any(|((u, p), (next_u, next_p))| u == next_u || p == next_p)
    {
        return Err(invalid());
    }
    let mut cursor = 0;
    let mut section = 0;
    let mut removed = Vec::new();
    for line in previous.projection {
        if line.starts_with("m=") {
            section += 1;
        }
        if proposed.projection.get(cursor) == Some(&line) {
            cursor += 1;
        } else if section > 0 && line.starts_with("a=extmap:") {
            removed.push((section, line));
        } else {
            return Err(invalid());
        }
    }
    if cursor != proposed.projection.len() {
        return Err(invalid());
    }
    Ok(removed)
}

struct Extension<'a> {
    id: u16,
    uri: &'a str,
}
fn extension(line: &str, removed: bool) -> Result<Extension<'_>> {
    let mut fields = line
        .strip_prefix("a=extmap:")
        .ok_or_else(invalid)?
        .split_ascii_whitespace();
    let identity = fields.next().ok_or_else(invalid)?;
    let (id, direction) = identity
        .split_once('/')
        .map_or((identity, None), |(id, direction)| (id, Some(direction)));
    if id.is_empty()
        || !id.bytes().all(|b| b.is_ascii_digit())
        || direction.is_some_and(|v| !["sendonly", "recvonly", "sendrecv", "inactive"].contains(&v))
    {
        return Err(invalid());
    }
    let id = id
        .parse::<u16>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(invalid)?;
    let uri = fields.next().ok_or_else(invalid)?;
    // Unknown extension parameters remain immutable. Do not infer that their
    // negotiation semantics permit removing them from a restart.
    if removed && fields.next().is_some() {
        return Err(invalid());
    }
    Ok(Extension { id, uri })
}
#[derive(Default)]
struct Headers<'a> {
    mid: Option<&'a str>,
    extensions: Vec<Extension<'a>>,
}
fn headers(sdp: &str) -> Result<Vec<Headers<'_>>> {
    let mut sections = vec![Headers::default()];
    for line in sdp.split_terminator("\r\n") {
        if line.starts_with("m=") {
            if sections.len() > MAX_MEDIA {
                return Err(invalid());
            }
            sections.push(Headers::default());
        }
        let media = sections.len() > 1;
        let section = sections.last_mut().ok_or_else(invalid)?;
        if let Some(mid) = line.strip_prefix("a=mid:") {
            if !media
                || mid.is_empty()
                || mid.len() > 256
                || mid.chars().any(char::is_whitespace)
                || section.mid.replace(mid).is_some()
            {
                return Err(invalid());
            }
        }
        if line.starts_with("a=extmap:") {
            let value = extension(line, false)?;
            if section.extensions.len() >= 256
                || section.extensions.iter().any(|e| e.id == value.id)
            {
                return Err(invalid());
            }
            section.extensions.push(value);
        }
    }
    let mut mids = std::collections::BTreeSet::new();
    for section in sections.iter().skip(1) {
        if !mids.insert(section.mid.ok_or_else(invalid)?) {
            return Err(invalid());
        }
    }
    Ok(sections)
}

/// Validate against a trusted answer captured from the SAME original admitted
/// physical peer under current authority. An answer supplied by the requester
/// cannot provide this proof. Only extmaps absent by BOTH ID and URI from that
/// answer's same MID (including session-level mappings) may disappear. Every
/// retained mapping and all other non-transport SDP remain byte/order stable.
pub fn validate_negotiated_offer_change(
    previous: &str,
    trusted_answer: &str,
    proposed: &str,
) -> Result<()> {
    let removed = candidate(previous, proposed)?;
    if removed.is_empty() {
        return Ok(());
    }
    parse(trusted_answer)?;
    let old = headers(previous)?;
    let answer = headers(trusted_answer)?;
    for (section, line) in removed {
        let mid = old.get(section).and_then(|s| s.mid).ok_or_else(invalid)?;
        let accepted = answer
            .iter()
            .skip(1)
            .find(|s| s.mid == Some(mid))
            .ok_or_else(invalid)?;
        let removed = extension(&line, true)?;
        if answer[0]
            .extensions
            .iter()
            .chain(&accepted.extensions)
            .any(|e| e.id == removed.id || e.uri == removed.uri)
        {
            return Err(invalid());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn offer(version: u64, tag: &str) -> String {
        format!("v=0\r\no=- 1 {version} IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 0.0.0.0\r\na=ice-ufrag:{tag}U\r\na=ice-pwd:{tag}PASSWORD1234567890123456\r\na=fingerprint:sha-256 SOURCE\r\na=setup:actpass\r\na=mid:0\r\na=sendrecv\r\na=rtpmap:111 opus/48000/2\r\na=extmap:3 urn:negotiated\r\na=extmap:14 urn:declined\r\na=x-immutable:original\r\n")
    }
    fn answer() -> String {
        offer(1, "server").replace("a=extmap:14 urn:declined\r\n", "")
    }
    fn proposed() -> String {
        offer(2, "fresh").replace("a=extmap:14 urn:declined\r\n", "")
    }
    #[test]
    fn declined_header_omission_requires_original_answer_before_io() {
        let previous = offer(1, "old");
        validate_restart_candidate(&previous, &proposed()).unwrap();
        assert!(super::super::validate_offer_change(&previous, &proposed()).is_err());
        validate_negotiated_offer_change(&previous, &answer(), &proposed()).unwrap();
        // Even another ID cannot turn a negotiated URI into a declined one.
        for accepted in [
            offer(1, "server"),
            answer() + "a=extmap:23 urn:declined\r\n",
            answer() + "a=extmap:14 urn:other\r\n",
        ] {
            assert!(validate_negotiated_offer_change(&previous, &accepted, &proposed()).is_err());
        }
    }
    #[test]
    fn negotiated_mappings_and_all_other_media_fields_remain_immutable() {
        let previous = offer(1, "old");
        let removed = proposed().replace("a=extmap:3 urn:negotiated\r\n", "");
        validate_restart_candidate(&previous, &removed).unwrap(); // Candidate only, not an IO grant.
        assert!(validate_negotiated_offer_change(&previous, &answer(), &removed).is_err());
        for (from, to) in [
            ("extmap:3", "extmap:4"),
            ("urn:negotiated", "urn:changed"),
            ("SOURCE", "OTHER"),
            ("a=sendrecv", "a=recvonly"),
            ("x-immutable:original", "x-immutable:changed"),
            ("opus/48000/2", "PCMU/8000"),
        ] {
            assert!(validate_restart_candidate(&previous, &proposed().replace(from, to)).is_err());
        }
        assert!(
            validate_restart_candidate(&previous, &(proposed() + "a=extmap:24 urn:added\r\n"))
                .is_err()
        );
    }
    #[test]
    fn ambiguous_foreign_missing_mid_and_session_negotiation_cannot_prove_decline() {
        let previous = offer(1, "old");
        for answer in [
            answer().replace("a=mid:0", "a=mid:9"),
            answer().replace("a=mid:0\r\n", ""),
            answer() + "a=mid:0\r\n",
            answer() + "a=extmap:3 urn:duplicate\r\n",
            answer().replace("m=audio", "a=extmap:14 urn:declined\r\nm=audio"),
        ] {
            assert!(validate_negotiated_offer_change(&previous, &answer, &proposed()).is_err());
        }
        let parameterized = previous.replace("urn:declined", "urn:declined unknown-parameter");
        assert!(validate_negotiated_offer_change(&parameterized, &answer(), &proposed()).is_err());
    }
    #[test]
    fn accepted_extension_in_another_mid_does_not_reattribute_declined_mapping() {
        let section = |s: String| {
            s.split_once("m=audio")
                .unwrap()
                .1
                .to_owned()
                .replace("a=mid:0", "a=mid:1")
        };
        let previous = offer(1, "old") + "m=audio" + &section(offer(1, "old"));
        let next = offer(2, "fresh") + "m=audio" + &section(proposed());
        let accepted = offer(1, "server") + "m=audio" + &section(answer());
        validate_negotiated_offer_change(&previous, &accepted, &next).unwrap();
        let removed_from_negotiated_mid = next.replacen("a=extmap:14 urn:declined\r\n", "", 1);
        assert!(validate_negotiated_offer_change(
            &previous,
            &accepted,
            &removed_from_negotiated_mid
        )
        .is_err());
    }
}
