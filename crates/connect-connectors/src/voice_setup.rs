//! Explicit operator provisioning for an existing Connect-owned number and SIP trunk.
use crate::{setup::owned_sip, Telnyx, Vapi};
use anyhow::{ensure, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fs, io::Write, path::Path};
#[derive(Serialize, Deserialize)]
struct State {
    name: String,
    number: String,
    number_id: String,
    assistant_id: String,
    sip_connection_id: String,
    outbound_profile_id: Option<String>,
    inbound_connection_id: Option<String>,
    vapi_credential_id: Option<String>,
    vapi_phone_number_id: Option<String>,
    vapi_sip_number_id: Option<String>,
    fqdn_id: Option<String>,
    previous_number: Value,
    pending: Option<String>,
    completed: bool,
}
pub(crate) fn write(path: &Path, s: &impl Serialize) -> Result<()> {
    let pending = path.with_extension("pending.json");
    let mut opts = fs::OpenOptions::new();
    opts.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(&pending)?;
    file.write_all(&serde_json::to_vec_pretty(s)?)?;
    file.sync_all()?;
    drop(file);
    fs::rename(pending, path)?;
    if let Some(p) = path.parent() {
        fs::File::open(p)?.sync_all()?;
    }
    Ok(())
}
pub(crate) fn id(v: &Value) -> Result<String> {
    v.pointer("/data/id")
        .or_else(|| v.get("id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .context("Missing resource ID; do not repeat creation")
}
pub(crate) fn named(v: &Value, name: &str, field: &str) -> Result<Option<String>> {
    let rows = v
        .as_array()
        .or_else(|| v.get("data").and_then(Value::as_array))
        .context("Invalid resource list")?;
    let matches: Vec<_> = rows
        .iter()
        .filter(|r| r.get(field).and_then(Value::as_str) == Some(name))
        .collect();
    ensure!(
        matches.len() <= 1,
        "Multiple owned-name matches; reconcile manually"
    );
    matches.first().map(|v| id(v)).transpose()
}
/// Does not buy a number. Only the explicitly selected number's voice routing changes.
pub async fn apply(
    setup_state: &Path,
    number_id: &str,
    number: &str,
    state_path: &Path,
    inbound_only: bool,
) -> Result<Value> {
    let owned = owned_sip(setup_state)?;
    ensure!(
        number_id.chars().all(|c| c.is_ascii_digit()) && !number_id.is_empty(),
        "Invalid Telnyx number ID"
    );
    ensure!(
        number.starts_with('+')
            && (8..=16).contains(&number.len())
            && number[1..].chars().all(|c| c.is_ascii_digit()),
        "Use E.164 number"
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
    let t = Telnyx::from_env()?;
    let v = Vapi::from_env()?;
    let current: Value = t
        .client
        .get(format!("/phone_numbers/{number_id}"))
        .await
        .map_err(|_| anyhow::anyhow!("Cannot inspect selected Telnyx number"))?;
    ensure!(
        current
            .pointer("/data/phone_number")
            .and_then(Value::as_str)
            == Some(number),
        "Number ID does not match selected E.164 number"
    );
    let mut state = if state_path.exists() {
        serde_json::from_slice::<State>(&fs::read(state_path)?)?
    } else {
        ensure!(
            current
                .pointer("/data/connection_id")
                .and_then(Value::as_str)
                == Some(&owned.connection_id),
            "Number must be assigned to the owned Connect SIP connection before voice setup"
        );
        State {
            name: format!("{}-voice", owned.name),
            number: number.into(),
            number_id: number_id.into(),
            assistant_id: owned.assistant_id.clone(),
            sip_connection_id: owned.connection_id.clone(),
            outbound_profile_id: None,
            inbound_connection_id: None,
            vapi_credential_id: None,
            vapi_phone_number_id: None,
            vapi_sip_number_id: None,
            fqdn_id: None,
            previous_number: current,
            pending: None,
            completed: false,
        }
    };
    ensure!(
        state.number == number
            && state.number_id == number_id
            && state.assistant_id == owned.assistant_id
            && state.sip_connection_id == owned.connection_id,
        "Voice setup inputs changed; inspect existing checkpoint"
    );
    let messaging_before: Value = t
        .client
        .get(format!("/phone_numbers/{number_id}/messaging"))
        .await
        .map_err(|_| anyhow::anyhow!("Cannot inspect SMS association before voice setup"))?;
    // Named lookups recover creations without repeating an uncertain POST.
    for kind in ["outbound", "inbound", "sip", "fqdn", "credential", "phone"] {
        if inbound_only && ["credential", "phone"].contains(&kind) {
            continue;
        }
        let present = match kind {
            "outbound" => state.outbound_profile_id.is_some(),
            "inbound" => state.inbound_connection_id.is_some(),
            "credential" => state.vapi_credential_id.is_some(),
            "phone" => state.vapi_phone_number_id.is_some(),
            "sip" => state.vapi_sip_number_id.is_some(),
            _ => state.fqdn_id.is_some(),
        };
        if present {
            continue;
        }
        if kind == "credential" {
            // The carrier must be ready before Vapi validates/registers its gateway.
            t.client.patch::<Value>(&format!("/credential_connections/{}",owned.connection_id),&json!({"outbound":{"outbound_voice_profile_id":state.outbound_profile_id,"channel_limit":2,"ani_override":number,"ani_override_type":"always"}})).await.map_err(|_|anyhow::anyhow!("Cannot attach outbound profile before Vapi setup"))?;
        }
        let label = resource_label(&state.name, kind);
        let (list, field) = match kind {
            "outbound" => (
                t.client
                    .get::<Value>("/outbound_voice_profiles")
                    .query(&json!({"page[size]":100}))
                    .await
                    .map_err(|_| anyhow::anyhow!("Voice profile discovery failed"))?,
                "name",
            ),
            "inbound" => (
                t.client
                    .get::<Value>("/fqdn_connections")
                    .query(&json!({"page[size]":100}))
                    .await
                    .map_err(|_| anyhow::anyhow!("Inbound SIP discovery failed"))?,
                "connection_name",
            ),
            "credential" => (
                v.client
                    .get("/credential")
                    .await
                    .map_err(|_| anyhow::anyhow!("Vapi credential discovery failed"))?,
                "name",
            ),
            "phone" | "sip" => (
                v.client
                    .get("/phone-number")
                    .await
                    .map_err(|_| anyhow::anyhow!("Vapi number discovery failed"))?,
                "name",
            ),
            _ => (
                t.client
                    .get::<Value>("/fqdns").query(&json!({"filter[connection_id]":state.inbound_connection_id.as_deref().context("Missing inbound connection")?}))
                    .await
                    .map_err(|_| anyhow::anyhow!("FQDN discovery failed"))?,
                "fqdn",
            ),
        };
        let existing = named(
            &list,
            if kind == "fqdn" {
                "sip.vapi.ai"
            } else {
                &label
            },
            field,
        )?;
        ensure!(
            state.pending.as_deref() != Some(kind) || existing.is_some(),
            "Uncertain resource creation; inspect provider before clearing pending checkpoint"
        );
        let resource_id = if let Some(id) = existing {
            id
        } else {
            state.pending = Some(kind.into());
            write(state_path, &state)?;
            let result=match kind {
                "outbound"=>t.client.post::<Value>("/outbound_voice_profiles",&json!({"name":label,"traffic_type":"conversational","service_plan":"global","concurrent_call_limit":2,"whitelisted_destinations":["US"],"daily_spend_limit":"5.00","daily_spend_limit_enabled":true,"enabled":true})).await.map_err(|_|anyhow::anyhow!("Outbound profile creation uncertain; inspect checkpoint"))?,
                "inbound"=>t.client.post::<Value>("/fqdn_connections",&json!({"connection_name":label,"active":true,"transport_protocol":"UDP","inbound":{"codecs":["G711U"],"ani_number_format":"+E.164","dnis_number_format":"+e164","channel_limit":2}})).await.map_err(|e|creation_failure("Inbound SIP",&e,state_path,&mut state))?,
                "credential"=>v.client.post("/credential",&json!({"provider":"byo-sip-trunk","name":label,"gateways":[{"ip":"192.76.120.10","inboundEnabled":false,"outboundEnabled":true},{"ip":"64.16.250.10","inboundEnabled":false,"outboundEnabled":true}],"outboundAuthenticationPlan":{"authUsername":owned.username,"authPassword":owned.password,"sipRegisterPlan":{"realm":"sip.telnyx.com"}}})).await.map_err(|e|vapi_creation_failure(&e,state_path,&mut state))?,
                "phone"=>v.client.post("/phone-number",&json!({"provider":"byo-phone-number","name":label,"number":number,"credentialId":state.vapi_credential_id,"assistantId":state.assistant_id})).await.map_err(|e|vapi_creation_failure(&e,state_path,&mut state))?,
                "sip"=>v.client.post("/phone-number",&json!({"provider":"vapi","name":label,"sipUri":format!("sip:{}@sip.vapi.ai",sip_user(&state.name)),"assistantId":state.assistant_id})).await.map_err(|e|vapi_creation_failure(&e,state_path,&mut state))?,
                _=>t.client.post::<Value>("/fqdns",&json!({"connection_id":state.inbound_connection_id,"fqdn":"sip.vapi.ai","dns_record_type":"a","port":5060})).await.map_err(|_|anyhow::anyhow!("FQDN attachment creation uncertain; inspect checkpoint"))?,
            };
            id(&result)?
        };
        match kind {
            "outbound" => state.outbound_profile_id = Some(resource_id),
            "inbound" => state.inbound_connection_id = Some(resource_id),
            "credential" => state.vapi_credential_id = Some(resource_id),
            "phone" => state.vapi_phone_number_id = Some(resource_id),
            "sip" => state.vapi_sip_number_id = Some(resource_id),
            _ => state.fqdn_id = Some(resource_id),
        }
        state.pending = None;
        write(state_path, &state)?;
    }
    t.client.patch::<Value>(&format!("/credential_connections/{}",owned.connection_id),&json!({"outbound":{"outbound_voice_profile_id":state.outbound_profile_id,"channel_limit":2,"ani_override":number,"ani_override_type":"always"}})).await.map_err(|_|anyhow::anyhow!("Cannot attach outbound profile to owned trunk"))?;
    v.client
        .patch(
            &format!(
                "/phone-number/{}",
                state
                    .vapi_sip_number_id
                    .as_deref()
                    .context("Missing inbound SIP ID")?
            ),
            &json!({"sipUri":format!("sip:{}@sip.vapi.ai",sip_user(&state.name))}),
        )
        .await
        .map_err(|_| anyhow::anyhow!("Cannot update owned Vapi SIP destination"))?;
    t.client
        .patch::<Value>(
            &format!("/phone_numbers/{number_id}/voice"),
            &json!({"tech_prefix_enabled":false,"translated_number":sip_user(&state.name)}),
        )
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "Cannot configure selected number SIP URI: {:?}",
                e.api_errors()
                    .iter()
                    .map(|a| (&a.code, &a.title, a.detail()))
                    .collect::<Vec<_>>()
            )
        })?;
    t.client
        .patch::<Value>(
            &format!("/phone_numbers/{number_id}"),
            &json!({"connection_id":state.inbound_connection_id}),
        )
        .await
        .map_err(|_| anyhow::anyhow!("Cannot route selected number to owned inbound trunk"))?;
    let readback: Value = t
        .client
        .get(format!("/phone_numbers/{number_id}"))
        .await
        .map_err(|_| anyhow::anyhow!("Number route verification failed"))?;
    ensure!(
        readback
            .pointer("/data/connection_id")
            .and_then(Value::as_str)
            == state.inbound_connection_id.as_deref(),
        "Number routing readback mismatch"
    );
    if !inbound_only {
        let phone = v
            .client
            .phone_numbers()
            .get(
                state
                    .vapi_phone_number_id
                    .as_deref()
                    .context("Missing phone binding")?,
            )
            .await
            .map_err(|_| anyhow::anyhow!("Vapi readback failed"))?;
        ensure!(
            phone.assistant_id.as_deref() == Some(&state.assistant_id)
                && phone.number.as_deref() == Some(number),
            "Vapi phone binding mismatch"
        );
    }
    let messaging_after: Value = t
        .client
        .get(format!("/phone_numbers/{number_id}/messaging"))
        .await
        .map_err(|_| anyhow::anyhow!("Cannot verify SMS association after voice setup"))?;
    ensure!(
        messaging_before.pointer("/data/messaging_profile_id")
            == messaging_after.pointer("/data/messaging_profile_id"),
        "SMS association changed during voice setup; inspect selected number"
    );
    let inbound_phone = v
        .client
        .phone_numbers()
        .get(
            state
                .vapi_sip_number_id
                .as_deref()
                .context("Missing SIP binding")?,
        )
        .await
        .map_err(|_| anyhow::anyhow!("Cannot verify inbound assistant binding"))?;
    ensure!(
        inbound_phone.assistant_id.as_deref() == Some(&state.assistant_id),
        "Inbound assistant binding mismatch"
    );
    state.completed = !inbound_only;
    write(state_path, &state)?;
    Ok(
        json!({"number":number,"assistant_id":state.assistant_id,"vapi_phone_number_id":state.vapi_phone_number_id,"vapi_sip_number_id":state.vapi_sip_number_id,"outbound_profile_id":state.outbound_profile_id,"outbound_sip_connection_id":state.sip_connection_id,"inbound_sip_connection_id":state.inbound_connection_id,"configured":state.completed,"inbound_configured":true,"audio_qualified":false,"sms_routing_changed":false}),
    )
}

fn creation_failure(
    label: &str,
    error: &telnyx::Error,
    path: &Path,
    state: &mut State,
) -> anyhow::Error {
    if matches!(error, telnyx::Error::Api(_))
        && error
            .status()
            .is_some_and(|s| s.is_client_error() && s.as_u16() != 408)
    {
        state.pending = None;
        if write(path, state).is_err() {
            return anyhow::anyhow!("Rejected creation; checkpoint could not be updated");
        }
    }
    anyhow::anyhow!(
        "{label} creation failed (HTTP {:?}): {:?}",
        error.status().map(|s| s.as_u16()),
        error
            .api_errors()
            .iter()
            .map(|e| (&e.code, &e.title, e.detail()))
            .collect::<Vec<_>>()
    )
}

// Vapi resource names have a 40-character limit. Stable suffix preserves recovery.
pub(crate) fn resource_label(name: &str, kind: &str) -> String {
    use sha2::{Digest, Sha256};
    let full = format!("{name}-{kind}");
    if full.len() <= 40 {
        return full;
    }
    let digest = format!("{:x}", Sha256::digest(full.as_bytes()));
    format!(
        "{}-{}",
        full.chars().take(23).collect::<String>(),
        &digest[..16]
    )
}
fn vapi_creation_failure(error: &vapi::VapiError, path: &Path, state: &mut State) -> anyhow::Error {
    let status = error.status_code();
    if status.is_some_and(|s| (400..500).contains(&s) && s != 408) {
        state.pending = None;
        if write(path, state).is_err() {
            return anyhow::anyhow!("Rejected creation; checkpoint update failed");
        }
    }
    anyhow::anyhow!("Vapi creation failed (HTTP {status:?}); inspect checkpoint before retrying")
}
// Telnyx translated_number accepts only an alphanumeric SIP user; the FQDN
// connection supplies the domain. Sending a full sip: URI is rejected by its API.
fn sip_user(name: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("connect{:x}", Sha256::digest(name.as_bytes()))[..31].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_are_bounded_stable_and_distinct() {
        assert!(sip_user("with-hyphens")
            .chars()
            .all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(sip_user("a"), sip_user("b"));
        let name = "connect-local-gate-2-test-20260408-voice";
        for kind in ["credential", "phone", "sip"] {
            let label = resource_label(name, kind);
            assert!(label.len() <= 40);
            assert_eq!(label, resource_label(name, kind));
        }
        assert_ne!(resource_label(name, "phone"), resource_label(name, "sip"));
        assert_eq!(resource_label("short", "phone"), "short-phone");
    }
}
