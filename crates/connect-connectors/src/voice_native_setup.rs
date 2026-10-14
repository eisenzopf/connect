//! Supported Vapi/Telnyx API integration, independent of BYO SIP provisioning.
use crate::{
    voice_setup::{id, named, resource_label, write},
    Telnyx, Vapi,
};
use anyhow::{ensure, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fs, path::Path};

#[derive(Serialize, Deserialize)]
struct State {
    name: String,
    number: String,
    number_id: String,
    assistant_id: String,
    outbound_profile_id: String,
    credential_id: Option<String>,
    phone_number_id: Option<String>,
    application_id: Option<String>,
    previous_number: Value,
    previous_voice: Value,
    messaging_profile_id: Option<String>,
    pending: Option<String>,
    completed: bool,
}

/// Operates only on the selected number already recorded in the Connect SIP
/// checkpoint. It never purchases a number or modifies existing Vapi credentials.
pub async fn apply(voice_state: &Path, state_path: &Path) -> Result<Value> {
    let key = std::env::var("TELNYX_API_KEY").context("TELNYX_API_KEY missing")?;
    apply_with_clients(
        voice_state,
        state_path,
        &Telnyx::from_env()?,
        &Vapi::from_env()?,
        &key,
    )
    .await
}

/// Compose native routing using host-selected clients and the selected Telnyx key.
/// The key is transient provider request data and is never persisted in the checkpoint.
pub async fn apply_with_clients(
    voice_state: &Path,
    state_path: &Path,
    t: &Telnyx,
    v: &Vapi,
    telnyx_api_key: &str,
) -> Result<Value> {
    ensure!(!telnyx_api_key.is_empty(), "Selected Telnyx key is empty");
    let source: Value = serde_json::from_slice(&fs::read(voice_state)?)?;
    let number_id = source["number_id"]
        .as_str()
        .context("Missing owned number ID")?;
    let number = source["number"].as_str().context("Missing owned number")?;
    let assistant = source["assistant_id"]
        .as_str()
        .context("Missing owned assistant")?;
    let profile = source["outbound_profile_id"]
        .as_str()
        .context("Missing owned outbound profile")?;
    ensure!(
        number_id.chars().all(|c| c.is_ascii_digit()) && !number_id.is_empty(),
        "Invalid number ID"
    );
    ensure!(
        uuid::Uuid::parse_str(assistant).is_ok(),
        "Invalid assistant ID"
    );
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(state_path.with_extension("lock"))?;
    lock.try_lock_exclusive()?;
    let current: Value = t
        .client
        .get(format!("/phone_numbers/{number_id}"))
        .await
        .map_err(|_| anyhow::anyhow!("Cannot inspect owned number"))?;
    ensure!(
        current
            .pointer("/data/phone_number")
            .and_then(Value::as_str)
            == Some(number),
        "Number ownership checkpoint mismatch"
    );
    let messaging: Value = t
        .client
        .get(format!("/phone_numbers/{number_id}/messaging"))
        .await
        .map_err(|_| anyhow::anyhow!("Cannot inspect SMS association"))?;
    let mut s = if state_path.exists() {
        serde_json::from_slice::<State>(&fs::read(state_path)?)?
    } else {
        let previous_voice: Value = t
            .client
            .get(format!("/phone_numbers/{number_id}/voice"))
            .await
            .map_err(|_| anyhow::anyhow!("Cannot inspect previous voice routing"))?;
        State {
            name: resource_label(
                source["name"]
                    .as_str()
                    .context("Missing owned setup name")?,
                "native",
            ),
            number: number.into(),
            number_id: number_id.into(),
            assistant_id: assistant.into(),
            outbound_profile_id: profile.into(),
            credential_id: None,
            phone_number_id: None,
            application_id: None,
            previous_number: current,
            previous_voice,
            messaging_profile_id: messaging
                .pointer("/data/messaging_profile_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            pending: None,
            completed: false,
        }
    };
    ensure!(
        s.number == number
            && s.number_id == number_id
            && s.assistant_id == assistant
            && s.outbound_profile_id == profile,
        "Native voice setup inputs changed; reconcile checkpoint"
    );
    for kind in ["credential", "phone"] {
        let present = if kind == "credential" {
            s.credential_id.is_some()
        } else {
            s.phone_number_id.is_some()
        };
        if present {
            continue;
        }
        let label = if kind == "credential" {
            s.name.clone()
        } else {
            resource_label(&s.name, "phone")
        };
        let endpoint = if kind == "credential" {
            "/credential"
        } else {
            "/phone-number"
        };
        let list = v
            .client
            .get(endpoint)
            .await
            .map_err(|_| anyhow::anyhow!("Native resource discovery failed"))?;
        let existing = named(&list, &label, "name")?;
        ensure!(
            s.pending.as_deref() != Some(kind) || existing.is_some(),
            "Uncertain creation; reconcile before retry"
        );
        let resource = if let Some(id) = existing {
            id
        } else {
            s.pending = Some(kind.into());
            write(state_path, &s)?;
            let result = if kind == "credential" {
                v.client
                    .post(
                        endpoint,
                        &json!({"provider":"telnyx","name":label,"apiKey":telnyx_api_key}),
                    )
                    .await
            } else {
                v.client.post(endpoint,&json!({"provider":"telnyx","name":label,"number":number,"credentialId":s.credential_id,"assistantId":assistant})).await
            };
            match result {
                Ok(value) => id(&value)?,
                Err(e) => {
                    if e.status_code()
                        .is_some_and(|n| (400..500).contains(&n) && n != 408)
                    {
                        s.pending = None;
                        write(state_path, &s)?;
                    }
                    anyhow::bail!(
                        "Native Vapi creation failed (HTTP {:?}); inspect checkpoint",
                        e.status_code()
                    );
                }
            }
        };
        if kind == "credential" {
            s.credential_id = Some(resource);
        } else {
            s.phone_number_id = Some(resource);
        }
        s.pending = None;
        write(state_path, &s)?;
    }
    let credential = v
        .client
        .get(&format!(
            "/credential/{}",
            s.credential_id.as_deref().context("Missing credential")?
        ))
        .await
        .map_err(|_| anyhow::anyhow!("Native credential readback failed"))?;
    ensure!(
        credential["provider"] == "telnyx" && credential["name"] == s.name,
        "Native credential ownership mismatch"
    );
    let app = credential["telnyxApplicationId"]
        .as_str()
        .context("Missing native Telnyx application ID")?;
    ensure!(
        app.chars().all(|c| c.is_ascii_digit()) && !app.is_empty(),
        "Invalid application ID"
    );
    s.application_id = Some(app.into());
    write(state_path, &s)?;
    let (active_profile, shared) = application_profile(
        t,
        v,
        app,
        s.credential_id.as_deref().context("Missing credential")?,
        profile,
    )
    .await?;
    t.client
        .patch::<Value>(
            &format!("/phone_numbers/{number_id}/voice"),
            &json!({"tech_prefix_enabled":false,"translated_number":""}),
        )
        .await
        .map_err(|_| anyhow::anyhow!("Cannot clear SIP translation for native voice routing"))?;
    t.client
        .patch::<Value>(
            &format!("/phone_numbers/{number_id}"),
            &json!({"connection_id":app}),
        )
        .await
        .map_err(|_| anyhow::anyhow!("Cannot bind owned number to native application"))?;
    let readback: Value = t
        .client
        .get(format!("/phone_numbers/{number_id}"))
        .await
        .map_err(|_| anyhow::anyhow!("Number readback failed"))?;
    let application: Value = t
        .client
        .get(format!("/call_control_applications/{app}"))
        .await
        .map_err(|_| anyhow::anyhow!("Application readback failed"))?;
    let sms: Value = t
        .client
        .get(format!("/phone_numbers/{number_id}/messaging"))
        .await
        .map_err(|_| anyhow::anyhow!("Messaging readback failed"))?;
    let phone = v
        .client
        .phone_numbers()
        .get(s.phone_number_id.as_deref().context("Missing phone ID")?)
        .await
        .map_err(|_| anyhow::anyhow!("Native phone readback failed"))?;
    ensure!(
        readback
            .pointer("/data/connection_id")
            .and_then(Value::as_str)
            == Some(app),
        "Native number routing mismatch"
    );
    ensure!(
        application
            .pointer("/data/outbound/outbound_voice_profile_id")
            .and_then(Value::as_str)
            == Some(active_profile.as_str()),
        "Native outbound profile mismatch"
    );
    ensure!(
        phone.provider.as_deref() == Some("telnyx")
            && phone.assistant_id.as_deref() == Some(assistant)
            && phone.number.as_deref() == Some(number),
        "Native assistant/number mismatch"
    );
    ensure!(
        sms.pointer("/data/messaging_profile_id")
            .and_then(Value::as_str)
            == s.messaging_profile_id.as_deref(),
        "SMS profile changed; reconcile selected number"
    );
    s.completed = true;
    write(state_path, &s)?;
    Ok(
        json!({"configured":true,"backend":"vapi-telnyx-native","number":number,"assistant_id":assistant,"vapi_phone_number_id":s.phone_number_id,"telnyx_application_id":app,"outbound_profile_id":active_profile,"provider_application_shared":shared,"audio_qualified":false,"sms_routing_changed":false,"byo_sip_qualified":false}),
    )
}

async fn application_profile(
    t: &Telnyx,
    v: &Vapi,
    app: &str,
    credential_id: &str,
    profile: &str,
) -> Result<(String, bool)> {
    let credentials = v
        .client
        .get("/credential")
        .await
        .map_err(|_| anyhow::anyhow!("Cannot inspect shared native application ownership"))?;
    let shared = credentials
        .as_array()
        .context("Invalid credential list")?
        .iter()
        .any(|c| {
            c["provider"] == "telnyx"
                && c["telnyxApplicationId"].as_str() == Some(app)
                && c["id"].as_str() != Some(credential_id)
        });
    let before_application: Value = t
        .client
        .get(format!("/call_control_applications/{app}"))
        .await
        .map_err(|_| anyhow::anyhow!("Cannot inspect native application"))?;
    let active_profile = if shared {
        before_application.pointer("/data/outbound/outbound_voice_profile_id").and_then(Value::as_str).context("Shared provider application needs an existing outbound profile; configure it explicitly")?.to_owned()
    } else {
        t.client
            .patch::<Value>(
                &format!("/call_control_applications/{app}"),
                &json!({"outbound":{"outbound_voice_profile_id":profile,"channel_limit":2}}),
            )
            .await
            .map_err(|_| {
                anyhow::anyhow!("Cannot attach owned outbound profile to native application")
            })?;
        profile.to_owned()
    };
    Ok((active_profile, shared))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use wiremock::{
        matchers::{body_json, method, path},
        Mock, MockServer, ResponseTemplate,
    };
    #[tokio::test]
    async fn selected_integration_key_is_transient_even_when_creation_is_uncertain() {
        let server = MockServer::start().await;
        let t = Telnyx::new("selected-header-key", &server.uri()).unwrap();
        let v = Vapi::new("selected-vapi-key", &server.uri()).unwrap();
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/gate5-native-provider-fixtures");
        fs::create_dir_all(&root).unwrap();
        let directory = tempfile::tempdir_in(root).unwrap();
        let source = directory.path().join("owned-voice.json");
        let checkpoint = directory.path().join("native.json");
        let number = "+15550000001";
        fs::write(
            &source,
            serde_json::to_vec(&json!({
                "name":"fixture", "number_id":"1234", "number":number,
                "assistant_id":"12345678-1234-1234-1234-123456789012",
                "outbound_profile_id":"2345"
            }))
            .unwrap(),
        )
        .unwrap();
        for (endpoint, body) in [
            (
                "/phone_numbers/1234",
                json!({"data":{"phone_number":number}}),
            ),
            (
                "/phone_numbers/1234/messaging",
                json!({"data":{"messaging_profile_id":"fixture-sms"}}),
            ),
            ("/phone_numbers/1234/voice", json!({"data":{}})),
            ("/credential", json!([])),
        ] {
            Mock::given(method("GET"))
                .and(path(endpoint))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .expect(1)
                .mount(&server)
                .await;
        }
        let selected = "selected-disposable-integration-key";
        Mock::given(method("POST")).and(path("/credential"))
            .and(body_json(json!({"provider":"telnyx","name":resource_label("fixture", "native"),"apiKey":selected})))
            .respond_with(ResponseTemplate::new(500))
            .expect(1).mount(&server).await;
        assert!(apply_with_clients(&source, &checkpoint, &t, &v, selected)
            .await
            .is_err());
        let protected = fs::read_to_string(&checkpoint).unwrap();
        assert!(!protected.contains(selected) && !protected.contains("apiKey"));
        let state: Value = serde_json::from_str(&protected).unwrap();
        assert_eq!(state["pending"], "credential");
        assert_eq!(state["number_id"], "1234");
    }
    #[tokio::test]
    async fn shared_application_is_read_only_and_retains_its_existing_limits() {
        let server = MockServer::start().await;
        let t = Telnyx::new("fixture", &server.uri()).unwrap();
        let v = Vapi::new("fixture", &server.uri()).unwrap();
        Mock::given(method("GET")).and(path("/credential")).respond_with(ResponseTemplate::new(200).set_body_json(json!([{"id":"ours","provider":"telnyx","telnyxApplicationId":"123"},{"id":"other","provider":"telnyx","telnyxApplicationId":"123"}]))).mount(&server).await;
        Mock::given(method("GET")).and(path("/call_control_applications/123")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"outbound":{"outbound_voice_profile_id":"existing","channel_limit":1}}}))).mount(&server).await;
        Mock::given(method("PATCH"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        assert_eq!(
            application_profile(&t, &v, "123", "ours", "new-profile")
                .await
                .unwrap(),
            ("existing".into(), true)
        );
        assert!(server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method == "GET"));
    }
    #[tokio::test]
    async fn exclusively_owned_application_gets_the_owned_outbound_profile() {
        let server = MockServer::start().await;
        let t = Telnyx::new("fixture", &server.uri()).unwrap();
        let v = Vapi::new("fixture", &server.uri()).unwrap();
        Mock::given(method("GET"))
            .and(path("/credential"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!([{"id":"ours","provider":"telnyx","telnyxApplicationId":"123"}]),
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/call_control_applications/123"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data":{"outbound":{"outbound_voice_profile_id":null}}})),
            )
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/call_control_applications/123"))
            .and(body_json(
                json!({"outbound":{"outbound_voice_profile_id":"new-profile","channel_limit":2}}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{}})))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            application_profile(&t, &v, "123", "ours", "new-profile")
                .await
                .unwrap(),
            ("new-profile".into(), false)
        );
    }
}
