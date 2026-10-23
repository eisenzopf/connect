//! Vapi telephone call control using the upstream Rust SDK.
use crate::{classify, Failure, Vapi};
use async_trait::async_trait;
pub use connect_connector_api::{InboundCall, RemoteCall, VoiceConnector, VoiceQuery, VoiceStart};
use serde_json::json;
mod reconciliation;

fn remote(call: vapi::Call) -> Result<RemoteCall, Failure> {
    if uuid::Uuid::parse_str(&call.id).is_err() {
        return Err(Failure::Uncertain);
    }
    let state = match call.status.as_deref() {
        Some("queued" | "ringing" | "in-progress" | "ended") => {
            call.status.clone().ok_or(Failure::Uncertain)?
        }
        _ => "uncertain".into(),
    };
    let url = call
        .extra
        .pointer("/monitor/controlUrl")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    if url
        .as_ref()
        .is_some_and(|u| !valid_control_url(u, &call.id))
    {
        return Err(Failure::Uncertain);
    }
    Ok(RemoteCall {
        id: call.id,
        state,
        control_url: url,
        ended_reason: call.ended_reason.filter(|r| r.len() <= 256),
    })
}
fn valid_control_url(url: &str, id: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    parsed.scheme() == "https"
        && parsed.host_str().is_some_and(|h| h.ends_with(".vapi.ai"))
        && parsed.port().is_none()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && parsed.path() == format!("/{id}/control")
}
#[async_trait]
impl VoiceConnector for Vapi {
    fn voice_manifest(&self) -> connect_connector_api::manifest::Manifest {
        crate::provider_mapping::vapi_telephone_manifest()
    }
    fn voice_configuration(&self) -> std::collections::BTreeMap<String, String> {
        crate::provider_mapping::references("private-key", &self.private_key_reference)
    }
    fn normalize(
        &self,
        mutation_id: &str,
        call: &RemoteCall,
    ) -> Result<connect_connector_api::lifecycle::Observation, Failure> {
        use connect_connector_api::lifecycle::{Observation, State, Verification};
        if mutation_id.is_empty()
            || mutation_id.len() > 256
            || uuid::Uuid::parse_str(&call.id).is_err()
        {
            return Err(Failure::Rejected);
        }
        Ok(Observation {
            mutation_id: mutation_id.into(),
            external_id: Some(call.id.clone()),
            event_id: format!("vapi-call:{}:{}", call.id, call.state),
            state: State::Voice(crate::provider_mapping::vapi_voice_state(&call.state)),
            verification: Verification::AuthenticatedResponse,
        })
    }
    async fn start_receipt(
        &self,
        mutation_id: &str,
        request: &VoiceStart,
    ) -> Result<connect_connector_api::VoiceReceipt, Failure> {
        connect_connector_api::validate_voice_mutation_id(mutation_id)?;
        let call = remote(
            self.client
                .calls()
                .create(vapi::CreateCallRequest {
                    assistant_id: Some(request.assistant_id.clone()),
                    phone_number_id: Some(request.phone_number_id.clone()),
                    customer: Some(json!({"number":request.destination})),
                    extra: json!({"name":reconciliation::correlation(mutation_id)}),
                    ..Default::default()
                })
                .await
                .map_err(|e| classify(e.status_code()))?,
        )?;
        let observation = self
            .normalize(mutation_id, &call)
            .map_err(|_| Failure::Uncertain)?;
        Ok(connect_connector_api::VoiceReceipt { call, observation })
    }
    async fn reconcile_receipt(
        &self,
        mutation_id: &str,
        query: &connect_connector_api::VoiceReconcileQuery,
    ) -> Result<connect_connector_api::VoiceReceipt, Failure> {
        reconciliation::lookup(self, mutation_id, query).await
    }
    async fn end_receipt(
        &self,
        mutation_id: &str,
        external_id: &str,
        control_url: Option<&str>,
    ) -> Result<connect_connector_api::VoiceReceipt, Failure> {
        connect_connector_api::validate_voice_mutation_id(mutation_id)?;
        // Existing end already obtains an authoritative status after control.
        // Keep that exact request path and preserve its private control capability.
        let call = self.end(external_id, control_url).await?;
        let mut observation = self
            .normalize(mutation_id, &call)
            .map_err(|_| Failure::Uncertain)?;
        observation.verification = connect_connector_api::lifecycle::Verification::Reconciliation;
        observation.event_id = format!("reconcile:{}", observation.event_id);
        Ok(connect_connector_api::VoiceReceipt { call, observation })
    }
    async fn inbound(&self, binding: &VoiceQuery) -> Result<Vec<InboundCall>, Failure> {
        // Updated timestamps catch calls indexed or changed after creation.
        // Re-read an overlap, and advance the durable watermark only after import.
        let lower = binding
            .scanned_until
            .saturating_sub(60_000)
            .max(binding.since.saturating_mul(1000));
        if binding.scan_until < lower {
            return Ok(vec![]);
        }
        let mut pending = vec![(lower, binding.scan_until)];
        let mut calls = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut requests = 0;
        while let Some((lo, hi)) = pending.pop() {
            requests += 1;
            if requests > 128 {
                return Err(Failure::Rejected);
            }
            let date = |ms| {
                chrono::DateTime::from_timestamp_millis(ms)
                    .map(|d| d.to_rfc3339())
                    .ok_or(Failure::Rejected)
            };
            let mut url =
                reqwest::Url::parse("https://api.vapi.ai/call").map_err(|_| Failure::Rejected)?;
            url.query_pairs_mut()
                .append_pair("limit", "100")
                .append_pair("assistantId", &binding.assistant_id)
                .append_pair("phoneNumberId", &binding.phone_number_id)
                .append_pair("updatedAtGe", &date(lo)?)
                .append_pair("updatedAtLe", &date(hi)?);
            let body = self
                .client
                .get(&format!("/call?{}", url.query().ok_or(Failure::Rejected)?))
                .await
                .map_err(|e| classify(e.status_code()))?;
            let page: Vec<vapi::Call> =
                serde_json::from_value(body).map_err(|_| Failure::Uncertain)?;
            if page.len() >= 100 {
                // Inclusive windows preserve timestamp ties; IDs remove overlap.
                // An unsplittable burst fails without advancing the scan cursor.
                if hi - lo <= 1 {
                    return Err(Failure::Rejected);
                }
                let mid = lo + (hi - lo) / 2;
                pending.push((mid, hi));
                pending.push((lo, mid));
                continue;
            }
            calls.extend(page.into_iter().filter(|c| seen.insert(c.id.clone())));
        }
        let mut result = Vec::new();
        for call in calls {
            if call.kind.as_deref() != Some("inboundPhoneCall") {
                continue;
            }
            let Some(assistant_id) = call.assistant_id.clone() else {
                continue;
            };
            let Some(phone_number_id) = call.phone_number_id.clone() else {
                continue;
            };
            let Some(caller) = call
                .customer
                .as_ref()
                .and_then(|c| c.get("number"))
                .and_then(|c| c.as_str())
                .map(str::to_owned)
            else {
                continue;
            };
            let created = call
                .extra
                .get("createdAt")
                .and_then(|v| v.as_str())
                .ok_or(Failure::Uncertain)?;
            let created_at = chrono::DateTime::parse_from_rfc3339(created)
                .map_err(|_| Failure::Uncertain)?
                .timestamp();
            let r = remote(call)?;
            result.push(InboundCall {
                provider_id: r.id,
                assistant_id,
                phone_number_id,
                caller,
                created_at,
                state: r.state,
                control_url: r.control_url,
                ended_reason: r.ended_reason,
            });
        }
        Ok(result)
    }

    async fn start(&self, job: &VoiceStart) -> Result<RemoteCall, Failure> {
        remote(
            self.client
                .calls()
                .create(vapi::CreateCallRequest {
                    assistant_id: Some(job.assistant_id.clone()),
                    phone_number_id: Some(job.phone_number_id.clone()),
                    customer: Some(json!({"number":job.destination})),
                    ..Default::default()
                })
                .await
                .map_err(|e| classify(e.status_code()))?,
        )
    }
    async fn status(&self, id: &str) -> Result<RemoteCall, Failure> {
        remote(
            self.client
                .calls()
                .get(id)
                .await
                .map_err(|e| classify(e.status_code()))?,
        )
    }
    async fn end(&self, id: &str, control_url: Option<&str>) -> Result<RemoteCall, Failure> {
        let url = control_url
            .filter(|u| valid_control_url(u, id))
            .ok_or(Failure::Rejected)?;
        // The Vapi SDK's PATCH status convenience is not live call control.
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|_| Failure::Rejected)?;
        let response = client
            .post(url)
            .json(&json!({"type":"end-call"}))
            .send()
            .await
            .map_err(|_| Failure::Uncertain)?;
        if !response.status().is_success() {
            return Err(classify(Some(response.status().as_u16())));
        }
        self.status(id).await
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn control_capability_is_bound_to_provider_call() {
        let id = "20bf62ee-c309-420a-af42-7a03b2f0739f";
        assert!(valid_control_url(
            &format!("https://aws-us-west-2-production1-phone-call-websocket.vapi.ai/{id}/control"),
            id
        ));
        for u in [
            format!("http://a.vapi.ai/{id}/control"),
            format!("https://a.vapi.ai.evil.example/{id}/control"),
            "https://a.vapi.ai/other/control".to_string(),
            format!("https://key@a.vapi.ai/{id}/control"),
            format!("https://a.vapi.ai/{id}/control?key=x"),
        ] {
            assert!(!valid_control_url(&u, id));
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod inbound_tests {
    use super::*;
    use wiremock::{
        matchers::{method, path, query_param},
        Mock, MockServer, ResponseTemplate,
    };
    #[tokio::test]
    async fn discovery_uses_scoped_filters_and_rejects_truncated_history() {
        let server = MockServer::start().await;
        let vapi = Vapi::new("fixture", &server.uri()).unwrap();
        let binding = VoiceQuery {
            assistant_id: "12345678-1234-1234-1234-123456789011".into(),
            phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
            since: 1,
            scanned_until: 0,
            scan_until: 1000,
        };
        let row = json!({"id":"12345678-1234-1234-1234-123456789014","type":"inboundPhoneCall","assistantId":binding.assistant_id,"phoneNumberId":binding.phone_number_id,"customer":{"number":"+15555550101"},"createdAt":"2026-10-07T00:00:00Z","status":"ended","endedReason":"customer-ended-call"});
        Mock::given(method("GET"))
            .and(path("/call"))
            .and(query_param("assistantId", &binding.assistant_id))
            .and(query_param("phoneNumberId", &binding.phone_number_id))
            .and(query_param("limit", "100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([row.clone()])))
            .mount(&server)
            .await;
        let calls = vapi.inbound_receipts(&binding).await.unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].call.caller, "+15555550101");
        assert_eq!(calls[0].call.state, "ended");
        assert_eq!(
            calls[0].state,
            connect_connector_api::lifecycle::VoiceState::Ended
        );
        assert_eq!(
            calls[0].verification,
            connect_connector_api::lifecycle::Verification::Reconciliation
        );
        assert!(calls[0].event_id.starts_with("discover:"));
        server.reset().await;
        Mock::given(method("GET"))
            .and(path("/call"))
            .respond_with(ResponseTemplate::new(200).set_body_json(vec![row; 100]))
            .mount(&server)
            .await;
        assert!(matches!(
            vapi.inbound(&binding).await,
            Err(Failure::Rejected)
        ));
    }
    #[tokio::test]
    async fn saturated_windows_split_without_losing_timestamp_ties_or_ids() {
        let server = MockServer::start().await;
        let vapi = Vapi::new("fixture", &server.uri()).unwrap();
        let binding = VoiceQuery {
            assistant_id: "12345678-1234-1234-1234-123456789011".into(),
            phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
            since: 1,
            scanned_until: 0,
            scan_until: 3000,
        };
        let rows:Vec<_>=(0..151).map(|n|json!({"id":uuid::Uuid::new_v4().to_string(),"type":"inboundPhoneCall","assistantId":binding.assistant_id,"phoneNumberId":binding.phone_number_id,"customer":{"number":"+15555550101"},"createdAt":"1970-01-01T00:00:01Z","updatedAt":chrono::DateTime::from_timestamp_millis(1500+n*5).unwrap().to_rfc3339(),"status":"ended"})).collect();
        Mock::given(method("GET"))
            .and(path("/call"))
            .respond_with(move |r: &wiremock::Request| {
                let q: std::collections::HashMap<_, _> = r.url.query_pairs().into_owned().collect();
                let lo = chrono::DateTime::parse_from_rfc3339(&q["updatedAtGe"])
                    .unwrap()
                    .timestamp_millis();
                let hi = chrono::DateTime::parse_from_rfc3339(&q["updatedAtLe"])
                    .unwrap()
                    .timestamp_millis();
                let filtered: Vec<_> = rows
                    .iter()
                    .filter(|c| {
                        let ms =
                            chrono::DateTime::parse_from_rfc3339(c["updatedAt"].as_str().unwrap())
                                .unwrap()
                                .timestamp_millis();
                        ms >= lo && ms <= hi
                    })
                    .take(100)
                    .cloned()
                    .collect();
                ResponseTemplate::new(200).set_body_json(filtered)
            })
            .mount(&server)
            .await;
        let calls = vapi.inbound(&binding).await.unwrap();
        assert_eq!(calls.len(), 151);
        assert_eq!(
            calls
                .iter()
                .map(|c| &c.provider_id)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            151
        );
        assert!(server.received_requests().await.unwrap().len() >= 3);
    }
}
