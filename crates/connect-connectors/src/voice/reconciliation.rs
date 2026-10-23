//! Authenticated account lookup. A logical correlation is never a principal.
use super::*;
use connect_connector_api::{lifecycle::Verification, VoiceReceipt, VoiceReconcileQuery};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

// The provider's name field is limited to forty characters. Keep public host
// identifiers out of it; 144 bits also fit without truncating provider results.
pub(super) fn correlation(mutation: &str) -> String {
    let digest = format!("{:x}", Sha256::digest(mutation.as_bytes()));
    format!("cv1_{}", &digest[..36])
}
fn matches(call: &vapi::Call, name: &str, query: &VoiceReconcileQuery) -> bool {
    let created = call
        .extra
        .get("createdAt")
        .and_then(serde_json::Value::as_str)
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .map(|v| v.timestamp());
    call.extra.get("name").and_then(serde_json::Value::as_str) == Some(name)
        && call.kind.as_deref() == Some("outboundPhoneCall")
        && call.assistant_id.as_deref() == Some(query.request.assistant_id.as_str())
        && call.phone_number_id.as_deref() == Some(query.request.phone_number_id.as_str())
        && call
            .customer
            .as_ref()
            .and_then(|v| v.get("number"))
            .and_then(serde_json::Value::as_str)
            == Some(query.request.destination.as_str())
        && created.is_some_and(|t| t >= query.since && t <= query.until)
}
pub(super) async fn lookup(
    adapter: &Vapi,
    mutation: &str,
    query: &VoiceReconcileQuery,
) -> Result<VoiceReceipt, Failure> {
    connect_connector_api::validate_voice_mutation_id(mutation)?;
    if query.since < 0
        || query.until < query.since
        || query.until.saturating_sub(query.since) > 180
        || uuid::Uuid::parse_str(&query.request.assistant_id).is_err()
        || uuid::Uuid::parse_str(&query.request.phone_number_id).is_err()
        || !query.request.destination.starts_with('+')
        || !(3..=32).contains(&query.request.destination.len())
        || !query.request.destination[1..]
            .bytes()
            .all(|b| b.is_ascii_digit())
    {
        return Err(Failure::Rejected);
    }
    let name = correlation(mutation);
    let mut windows = vec![(
        query.since.saturating_mul(1000),
        query.until.saturating_mul(1000).saturating_add(999),
    )];
    let mut candidates = BTreeSet::new();
    let mut requests = 0;
    let mut bytes = 0usize;
    while let Some((lo, hi)) = windows.pop() {
        requests += 1;
        if requests > 128 {
            return Err(Failure::Uncertain);
        }
        let date = |ms| {
            chrono::DateTime::from_timestamp_millis(ms)
                .map(|v| v.to_rfc3339())
                .ok_or(Failure::Rejected)
        };
        let mut url =
            reqwest::Url::parse("https://api.vapi.ai/call").map_err(|_| Failure::Rejected)?;
        url.query_pairs_mut()
            .append_pair("limit", "100")
            .append_pair("assistantId", &query.request.assistant_id)
            .append_pair("phoneNumberId", &query.request.phone_number_id)
            .append_pair("createdAtGe", &date(lo)?)
            .append_pair("createdAtLe", &date(hi)?);
        let body = adapter
            .client
            .get(&format!("/call?{}", url.query().ok_or(Failure::Rejected)?))
            .await
            .map_err(|e| classify(e.status_code()))?;
        bytes = bytes.saturating_add(body.to_string().len());
        if bytes > 1024 * 1024 {
            return Err(Failure::Uncertain);
        }
        let page: Vec<vapi::Call> = serde_json::from_value(body).map_err(|_| Failure::Uncertain)?;
        if page.len() >= 100 {
            if hi - lo <= 1 {
                return Err(Failure::Uncertain);
            }
            let mid = lo + (hi - lo) / 2;
            windows.push((mid, hi));
            windows.push((lo, mid));
            continue;
        }
        for call in page {
            if call.extra.get("name").and_then(serde_json::Value::as_str) != Some(name.as_str()) {
                continue;
            }
            if !matches(&call, &name, query) || uuid::Uuid::parse_str(&call.id).is_err() {
                return Err(Failure::Uncertain);
            }
            candidates.insert(call.id);
            if candidates.len() > 1 {
                return Err(Failure::Uncertain);
            }
        }
    }
    let id = candidates.into_iter().next().ok_or(Failure::Uncertain)?;
    // Re-read the one correlated resource; a changed binding cannot become a
    // cleanup capability merely because it appeared earlier in a list page.
    let call = adapter
        .client
        .calls()
        .get(&id)
        .await
        .map_err(|e| classify(e.status_code()))?;
    if call.id != id || !matches(&call, &name, query) {
        return Err(Failure::Uncertain);
    }
    let call = remote(call)?;
    let mut observation = adapter
        .normalize(mutation, &call)
        .map_err(|_| Failure::Uncertain)?;
    observation.verification = Verification::Reconciliation;
    observation.event_id = format!("correlated:{}", observation.event_id);
    Ok(VoiceReceipt { call, observation })
}
#[cfg(test)]
mod tests;
