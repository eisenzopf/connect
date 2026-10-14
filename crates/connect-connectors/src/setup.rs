//! Operator provisioning with private, durable ownership and uncertainty records.
use crate::{Telnyx, Vapi};
use anyhow::{ensure, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fs, io::Write, path::Path};

const ASSISTANT_PROMPT: &str = include_str!("../prompts/assistant.txt");
mod secrets;
pub use secrets::{migrate_secrets, restore_secret, SecretMigration, SetupSecretStore};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub name: String,
    /// Explicit reuse skips assistant creation; existing assistants are never edited.
    pub assistant_id: Option<String>,
    /// Exact existing profile ID; reused read-only, never edited by setup.
    #[serde(default)]
    pub messaging_profile_id: Option<String>,
    /// Exact existing credential connection ID; never edited or treated as owned.
    #[serde(default)]
    pub sip_connection_id: Option<String>,
    /// Existing Telnyx number ID; purchases are deliberately separate.
    pub phone_number_id: Option<String>,
    pub callback_url: Option<String>,
    #[serde(default)]
    pub create_sip: bool,
    pub outbound_voice_profile_id: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct SetupState {
    #[serde(default)]
    operation_id: Option<String>,
    plan: Plan,
    name: String,
    assistant_id: Option<String>,
    messaging_profile_id: Option<String>,
    sip_connection_id: Option<String>,
    sip_user: String,
    /// Read-only compatibility input; never serialized into a new checkpoint.
    #[serde(default, skip_serializing)]
    sip_password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sip_password_ref: Option<String>,
    /// Protected one-way check used to reject importing a different password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sip_password_fingerprint: Option<String>,
    pending: Option<String>,
    number_messaging_applied: bool,
    number_sip_applied: bool,
    previous_number: Option<Value>,
}
/// Validate approved intent without provider I/O or checkpoint mutation.
pub fn verify_checkpoint_intent(path: &Path, plan: &Plan) -> Result<()> {
    ensure!(
        secrets::load_private(path)?.plan == *plan,
        "Setup plan changed; preserve the original checkpoint and select a fresh state path"
    );
    Ok(())
}
pub fn load_plan(path: &Path) -> Result<Plan> {
    let plan: Plan = serde_json::from_slice(&fs::read(path)?)?;
    validate(&plan)?;
    Ok(plan)
}
fn validate(p: &Plan) -> Result<()> {
    ensure!(
        !(p.create_sip && p.sip_connection_id.is_some()),
        "Select an existing SIP connection or create one, not both"
    );
    for id in [&p.messaging_profile_id, &p.sip_connection_id]
        .into_iter()
        .flatten()
    {
        ensure!(
            !id.is_empty()
                && id.len() <= 128
                && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "Invalid selected resource ID"
        );
    }
    ensure!(
        !p.name.is_empty()
            && p.name.len() <= 48
            && p.name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "Use a short resource name containing letters, digits, '-' or '_'"
    );
    for id in [&p.phone_number_id, &p.outbound_voice_profile_id]
        .into_iter()
        .flatten()
    {
        ensure!(
            !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()),
            "Invalid Telnyx resource ID"
        );
    }
    if let Some(id) = &p.assistant_id {
        ensure!(
            uuid::Uuid::parse_str(id).is_ok(),
            "Invalid Vapi assistant ID"
        );
    }
    if let Some(url) = &p.callback_url {
        ensure!(
            url.starts_with("https://")
                && url.ends_with("/webhooks/telnyx")
                && !url.contains('@')
                && !url.contains('?')
                && !url.contains('#'),
            "Use an HTTPS callback URL ending in /webhooks/telnyx"
        );
    }
    ensure!(
        p.phone_number_id.is_none() || p.callback_url.is_some() || p.messaging_profile_id.is_some(),
        "Number setup needs an existing messaging profile or public callback URL"
    );
    Ok(())
}
pub fn describe(p: &Plan) -> Value {
    json!({"name":p.name,"assistant":if p.assistant_id.is_some(){"reuse; no edits"}else{"create isolated Connect assistant"},"sms":if p.messaging_profile_id.is_some(){"reuse exact messaging profile; no edits"}else if p.callback_url.is_some(){"create messaging profile"}else{"not configured"},"number":if p.phone_number_id.is_some(){"explicitly associate selected existing number; retain previous associations"}else{"not selected"},"sip":if p.sip_connection_id.is_some(){"reuse exact connection; no edits or credential export"}else if p.create_sip{"create owned credential connection"}else{"not configured"},"selected":{"assistant_id":p.assistant_id,"messaging_profile_id":p.messaging_profile_id,"sip_connection_id":p.sip_connection_id,"phone_number_id":p.phone_number_id,"outbound_voice_profile_id":p.outbound_voice_profile_id},"purchases":false,"sms_consent":false,"deployment":"local only; callback and SIP/RTP reachability need separate checks"})
}
pub async fn discover() -> Result<Value> {
    let t = Telnyx::from_env()?;
    let v = Vapi::from_env()?;
    discover_with_clients(&v, &t).await
}
fn catalog_metadata(value: &Value) -> Value {
    let row = |value: &Value| {
        let mut fields = serde_json::Map::new();
        for name in [
            "id",
            "name",
            "phone_number",
            "connection_name",
            "active",
            "enabled",
        ] {
            if let Some(value) = value.get(name).filter(|value| {
                value.is_boolean() || value.as_str().is_some_and(|s| s.len() <= 256)
            }) {
                fields.insert(name.into(), value.clone());
            }
        }
        if let Some(features) = value.get("features").and_then(Value::as_array) {
            fields.insert(
                "features".into(),
                json!(features
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|name| matches!(*name, "sms" | "mms" | "voice" | "fax"))
                    .collect::<Vec<_>>()),
            );
        }
        Value::Object(fields)
    };
    if let Some(rows) = value.as_array() {
        return Value::Array(rows.iter().map(row).collect());
    }
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .map(|rows| rows.iter().map(row).collect::<Vec<_>>())
        .unwrap_or_default();
    json!({"data":data})
}
/// Legacy category names remain stable; provider credentials/configuration never
/// enter the saved discovery output. Clients can target isolated fixture origins.
pub async fn discover_with_clients(v: &Vapi, t: &Telnyx) -> Result<Value> {
    discover_selected_clients(Some(v), Some(t)).await
}
/// Read selected metadata categories only; omitted providers contribute no rows.
pub async fn discover_selected_clients(v: Option<&Vapi>, t: Option<&Telnyx>) -> Result<Value> {
    ensure!(
        v.is_some() || t.is_some(),
        "Select a provider for discovery"
    );
    let mut catalog = serde_json::Map::new();
    if v.is_none() || t.is_none() {
        catalog.insert("complete".into(), json!(false));
    }
    if let Some(t) = t {
        for (category, endpoint) in [
            ("phone_numbers", "/phone_numbers"),
            ("messaging_profiles", "/messaging_profiles"),
            ("connections", "/connections"),
        ] {
            let value: Value = t
                .client
                .get(endpoint)
                .query(&json!({"page[size]":100}))
                .await
                .map_err(|_| anyhow::anyhow!("Telnyx catalog discovery failed"))?;
            catalog.insert(category.into(), catalog_metadata(&value));
        }
    }
    if let Some(v) = v {
        let assistants = v
            .client
            .get("/assistant")
            .await
            .map_err(|_| anyhow::anyhow!("Vapi discovery failed"))?;
        catalog.insert("assistants".into(), catalog_metadata(&assistants));
    }
    catalog.insert("note".into(), json!("Metadata only; omitted providers were not requested; first 100 Telnyx resources per category; inspect pagination before selecting on larger accounts"));
    Ok(Value::Object(catalog))
}

fn write_state(path: &Path, s: &SetupState) -> Result<()> {
    ensure!(
        s.sip_password.is_none(),
        "Refuse to publish a checkpoint before confirming SIP secret migration"
    );
    let tmp = path.with_extension("pending.json");
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&tmp)
        .context("Cannot create private setup checkpoint; inspect any leftover pending file")?;
    file.write_all(&serde_json::to_vec_pretty(s)?)?;
    file.sync_all()?;
    drop(file);
    fs::rename(tmp, path)?;
    if let Some(parent) = path.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}
fn extract_id(value: &Value) -> Result<String> {
    value["data"]["id"]
        .as_str()
        .or_else(|| value["id"].as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            anyhow::anyhow!("Provider accepted setup but returned no ID; do not repeat creation")
        })
}
fn existing_named(value: &Value, name: &str, field: &str) -> Result<Option<String>> {
    let rows = value
        .as_array()
        .or_else(|| value["data"].as_array())
        .context("Invalid resource list")?;
    let ids: Vec<_> = rows
        .iter()
        .filter(|r| r[field].as_str() == Some(name))
        .map(extract_id)
        .collect::<Result<_>>()?;
    ensure!(
        ids.len() <= 1,
        "Multiple owned-name matches; resolve setup uncertainty manually"
    );
    Ok(ids.into_iter().next())
}
pub async fn apply(plan: &Plan, path: &Path) -> Result<()> {
    let v = Vapi::from_env()?;
    let t = Telnyx::from_env()?;
    apply_with_clients(plan, path, &v, &t).await
}

/// Injection point for provider contract tests; production uses fixed HTTPS origins.
pub async fn apply_with_clients(plan: &Plan, path: &Path, v: &Vapi, t: &Telnyx) -> Result<()> {
    apply_with_clients_and_secrets(plan, path, v, t, &secrets::NoSecrets).await
}
/// Provision using an authenticated server vault. Checkpoints retain only references.
pub async fn apply_with_clients_and_secrets(
    plan: &Plan,
    path: &Path,
    v: &Vapi,
    t: &Telnyx,
    secret_store: &dyn SetupSecretStore,
) -> Result<()> {
    validate(plan)?;
    secret_store.authorize()?;
    let lock_path = path.with_extension("lock");
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(lock_path)?;
    lock.try_lock_exclusive()
        .context("Another provider setup is running")?;
    let mut state = if path.exists() {
        secrets::load_private(path)?
    } else {
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        SetupState {
            operation_id: Some(suffix.clone()),
            plan: plan.clone(),
            name: format!("{}-{}", plan.name, &suffix[..12]),
            assistant_id: plan.assistant_id.clone(),
            messaging_profile_id: plan.messaging_profile_id.clone(),
            sip_connection_id: plan.sip_connection_id.clone(),
            sip_user: if plan.create_sip {
                format!("connect{}", &suffix[..20])
            } else {
                String::new()
            },
            sip_password: None,
            sip_password_ref: None,
            sip_password_fingerprint: None,
            pending: None,
            number_messaging_applied: false,
            number_sip_applied: false,
            previous_number: None,
        }
    };
    ensure!(
        state.plan == *plan,
        "Setup plan changed; use a fresh state path for a different plan"
    );
    if !path.exists() {
        write_state(path, &state)?;
    }
    if secrets::protect(&mut state, secret_store)? {
        secret_store.authorize()?;
        write_state(path, &state)?;
    }
    if let Some(id) = &plan.messaging_profile_id {
        let _: Value = t
            .client
            .get(format!("/messaging_profiles/{id}"))
            .await
            .map_err(|_| anyhow::anyhow!("Selected messaging profile is inaccessible"))?;
    }
    if let Some(id) = &plan.sip_connection_id {
        let _: Value = t
            .client
            .get(format!("/credential_connections/{id}"))
            .await
            .map_err(|_| anyhow::anyhow!("Selected SIP connection is inaccessible"))?;
    }
    if let Some(kind) = state.pending.clone() {
        let (rows, field) = match kind.as_str() {
            "assistant" => (
                v.client
                    .get("/assistant")
                    .await
                    .map_err(|_| anyhow::anyhow!("Assistant reconciliation failed"))?,
                "name",
            ),
            "messaging" => (
                t.client
                    .get::<Value>("/messaging_profiles")
                    .query(&json!({"page[size]":100}))
                    .await
                    .map_err(|_| anyhow::anyhow!("Messaging reconciliation failed"))?,
                "name",
            ),
            "sip" => (
                t.client
                    .get::<Value>("/credential_connections")
                    .query(&json!({"page[size]":100}))
                    .await
                    .map_err(|_| anyhow::anyhow!("SIP reconciliation failed"))?,
                "connection_name",
            ),
            _ => anyhow::bail!("Unknown setup checkpoint"),
        };
        let id = existing_named(&rows, &state.name, field)?.context(
            "Previous creation has an unknown outcome; inspect provider resources before retrying",
        )?;
        match kind.as_str() {
            "assistant" => state.assistant_id = Some(id),
            "messaging" => state.messaging_profile_id = Some(id),
            "sip" => state.sip_connection_id = Some(id),
            _ => {}
        };
        state.pending = None;
        write_state(path, &state)?;
    }
    if state.assistant_id.is_none() {
        secret_store.authorize()?;
        state.pending = Some("assistant".into());
        write_state(path, &state)?;
        let assistant = json!({
            "name": state.name,
            "metadata": {"connect": true,"connect_operation_id":state.operation_id},
            "model": {
                "provider": "openai", "model": "gpt-4o-mini",
                "messages": [{"role": "system", "content": ASSISTANT_PROMPT}],
            },
            "voice": {"provider": "vapi", "voiceId": "Elliot"},
        });
        let value = v
            .client
            .post("/assistant", &assistant)
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "Vapi assistant creation failed (status {:?}); checkpoint preserved",
                    error.status_code()
                )
            })?;
        state.assistant_id = Some(extract_id(&value)?);
        state.pending = None;
        write_state(path, &state)?;
    } else {
        v.client
            .get(&format!(
                "/assistant/{}",
                state.assistant_id.as_deref().unwrap_or_default()
            ))
            .await
            .map_err(|_| anyhow::anyhow!("Selected assistant is inaccessible"))?;
    }
    if let Some(callback) = &plan.callback_url {
        if state.messaging_profile_id.is_none() {
            secret_store.authorize()?;
            state.pending = Some("messaging".into());
            write_state(path, &state)?;
            let mut params = telnyx::messaging::CreateMessagingProfileParams::new(
                state.name.clone(),
                vec!["US".into()],
            );
            params.enabled = Some(true);
            params.webhook_url = Some(callback.clone());
            params.webhook_api_version = Some(telnyx::messaging::WebhookApiVersion::V2);
            let value: Value = t
                .client
                .post("/messaging_profiles", &params)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Messaging profile creation failed (status {:?}); checkpoint preserved",
                        e.status().map(|s| s.as_u16())
                    )
                })?;
            state.messaging_profile_id = Some(extract_id(&value)?);
            state.pending = None;
            write_state(path, &state)?;
        }
    }
    if plan.create_sip && state.sip_connection_id.is_none() {
        secret_store.authorize()?;
        state.pending = Some("sip".into());
        write_state(path, &state)?;
        let password = secret_store.read(
            state
                .sip_password_ref
                .as_deref()
                .context("SIP vault reference is missing")?,
        )?;
        let mut body = json!({"connection_name":state.name,"user_name":state.sip_user,"password":password,"active":true});
        if let Some(id) = &plan.outbound_voice_profile_id {
            body["outbound"] = json!({"outbound_voice_profile_id":id});
        }
        let value: Value = t
            .client
            .post("/credential_connections", &body)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "SIP connection creation failed (status {:?}); checkpoint preserved",
                    e.status().map(|s| s.as_u16())
                )
            })?;
        state.sip_connection_id = Some(extract_id(&value)?);
        state.pending = None;
        write_state(path, &state)?;
    }
    if let Some(id) = &plan.phone_number_id {
        if state.previous_number.is_none() {
            let number: Value = t
                .client
                .get(format!("/phone_numbers/{id}"))
                .await
                .map_err(|_| anyhow::anyhow!("Selected number is inaccessible"))?;
            let messaging: Value = t
                .client
                .get(format!("/phone_numbers/{id}/messaging"))
                .await
                .map_err(|_| anyhow::anyhow!("Selected number messaging is inaccessible"))?;
            state.previous_number = Some(json!({"voice":number,"messaging":messaging}));
            write_state(path, &state)?;
        }
        if !state.number_messaging_applied {
            secret_store.authorize()?;
            let _: Value = t
                .client
                .patch(
                    format!("/phone_numbers/{id}/messaging"),
                    &json!({"messaging_profile_id":state.messaging_profile_id}),
                )
                .await
                .map_err(|_| {
                    anyhow::anyhow!(
                        "Number messaging association failed; repeated association is safe"
                    )
                })?;
            state.number_messaging_applied = true;
            write_state(path, &state)?;
        }
        if (plan.create_sip || plan.sip_connection_id.is_some()) && !state.number_sip_applied {
            secret_store.authorize()?;
            let _: Value = t
                .client
                .patch(
                    format!("/phone_numbers/{id}"),
                    &json!({"connection_id":state.sip_connection_id}),
                )
                .await
                .map_err(|_| {
                    anyhow::anyhow!("Number SIP association failed; repeated association is safe")
                })?;
            state.number_sip_applied = true;
            write_state(path, &state)?;
        }
    }
    Ok(())
}

/// Fetch the account verification key so the two provider API keys suffice.
pub async fn public_key() -> Result<String> {
    let t = Telnyx::from_env()?;
    public_key_with_client(&t).await
}
/// Explicit server secret injection without writing provider tokens to the environment.
pub async fn public_key_with_client(t: &Telnyx) -> Result<String> {
    let value: Value = t.client.get("/public_key").await.map_err(|_| {
        anyhow::anyhow!("Cannot retrieve Telnyx webhook key; set TELNYX_PUBLIC_KEY explicitly")
    })?;
    value["data"]["public"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .context("Telnyx webhook key missing; set TELNYX_PUBLIC_KEY explicitly")
}

/// Read back exact configured IDs. Reports readiness without keys or destinations.
pub async fn check(path: &Path) -> Result<Value> {
    let v = Vapi::from_env()?;
    let t = Telnyx::from_env()?;
    check_with_clients(path, &v, &t).await
}
/// Read-only provider preflight; no resource assignment or messaging.
pub async fn check_with_clients(path: &Path, v: &Vapi, t: &Telnyx) -> Result<Value> {
    let state: SetupState = serde_json::from_slice(&fs::read(path)?)?;
    let mut carrier_ready = false;
    let mut campaign_status = "not_checked".to_owned();
    let assistant = if let Some(id) = &state.assistant_id {
        v.client
            .get(&format!("/assistant/{id}"))
            .await
            .map_err(|_| anyhow::anyhow!("Assistant readback failed"))?;
        true
    } else {
        false
    };
    let profile = if let Some(id) = &state.messaging_profile_id {
        let data: Value = t
            .client
            .get(format!("/messaging_profiles/{id}"))
            .await
            .map_err(|_| anyhow::anyhow!("Messaging profile readback failed"))?;
        data["data"]["enabled"] == true
            && data["data"]["webhook_url"].as_str() == state.plan.callback_url.as_deref()
    } else {
        false
    };
    let (sip, outbound) = if let Some(id) = &state.sip_connection_id {
        let data: Value = t
            .client
            .get(format!("/credential_connections/{id}"))
            .await
            .map_err(|_| anyhow::anyhow!("SIP connection readback failed"))?;
        (
            data["data"]["active"] == true,
            state.plan.outbound_voice_profile_id.is_some()
                && data["data"]["outbound"]["outbound_voice_profile_id"].as_str()
                    == state.plan.outbound_voice_profile_id.as_deref(),
        )
    } else {
        (false, false)
    };
    let number = if let Some(id) = &state.plan.phone_number_id {
        let data: Value = t
            .client
            .get(format!("/phone_numbers/{id}/messaging"))
            .await
            .map_err(|_| anyhow::anyhow!("Number messaging readback failed"))?;
        if let Some(phone) = data["data"]["phone_number"].as_str().filter(|s| {
            s.starts_with("+1") && s.len() == 12 && s[1..].chars().all(|c| c.is_ascii_digit())
        }) {
            let assignment: Option<Value> = match t
                .client
                .get::<Value>(format!("/10dlc/phone_number_campaigns/{phone}"))
                .await
            {
                Ok(value) => Some(value["data"].clone()),
                Err(e) if e.status().is_some_and(|s| s.as_u16() == 404) => None,
                Err(_) => {
                    anyhow::bail!("SMS campaign assignment readback failed; readiness unverified")
                }
            };
            campaign_status = "unassigned".into();
            if let Some(assignment) = assignment {
                if let Some(cid) = assignment["telnyxCampaignId"]
                    .as_str()
                    .or_else(|| assignment["campaignId"].as_str())
                {
                    ensure!(
                        !cid.is_empty()
                            && cid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
                        "Invalid carrier campaign ID"
                    );
                    let campaign: Value = t
                        .client
                        .get(format!("/10dlc/campaign/{cid}"))
                        .await
                        .map_err(|_| {
                            anyhow::anyhow!("SMS campaign readback failed; readiness unverified")
                        })?;
                    campaign_status = campaign["data"]["campaignStatus"]
                        .as_str()
                        .unwrap_or("unknown")
                        .to_owned();
                    carrier_ready = assignment["phoneNumber"] == phone
                        && assignment["assignmentStatus"] == "ASSIGNED"
                        && campaign["data"]["campaignId"] == cid
                        && campaign["data"]["mock"] == false
                        && matches!(campaign_status.as_str(), "MNO_ACCEPTED" | "MNO_PROVISIONED");
                }
            }
        }
        data["data"]["messaging_profile_id"].as_str() == state.messaging_profile_id.as_deref()
            && state.messaging_profile_id.is_some()
    } else {
        false
    };
    Ok(
        json!({"assistant_ready":assistant,"sms_carrier_ready":carrier_ready,"carrier_campaign_status":campaign_status,"messaging_profile_ready":profile,"number_sms_associated":number,"sip_connection_active":sip,"outbound_voice_profile_associated":outbound,"callback_configured":state.plan.callback_url.is_some(),"pending_creation":state.pending,"live_sms_verified":false,"live_audio_verified":false}),
    )
}

pub(crate) struct OwnedSip {
    pub name: String,
    pub assistant_id: String,
    pub connection_id: String,
    pub username: String,
    pub password: String,
}
pub(crate) fn owned_sip(path: &Path, secrets: &dyn SetupSecretStore) -> Result<OwnedSip> {
    let state = secrets::load_private(path)?;
    secrets.authorize()?;
    ensure!(
        state.sip_password.is_none(),
        "Migrate the legacy SIP checkpoint to protected vault storage before voice setup"
    );
    ensure!(
        state.pending.is_none() && state.plan.create_sip,
        "SIP setup must be complete and owned by Connect"
    );
    Ok(OwnedSip {
        name: state.name,
        assistant_id: state.assistant_id.context("Missing assistant")?,
        connection_id: state.sip_connection_id.context("Missing SIP connection")?,
        username: state.sip_user,
        password: secrets.read(
            state
                .sip_password_ref
                .as_deref()
                .context("SIP vault reference missing; import the original secret explicitly")?,
        )?,
    })
}
