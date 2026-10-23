//! Authenticated discovery is routing data, never remote principal assurance.
use super::*;
use connect_connector_api::{InboundVoiceReceipt, VoiceQuery};
use serde_json::{json, Value};
mod custody;
mod observed;
mod resolution;
pub use resolution::{
    ProviderIncomingVoiceResolution, ProviderIncomingVoiceResolutionCapture,
    ProviderIncomingVoiceResolutionPreparation, ProviderIncomingVoiceResolutionResult,
};
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderVoiceIngressPolicy {
    pub id: String,
    pub conversation_id: String,
    pub policy: ProviderSessionVoicePolicy,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    caller: MemberView,
    target: MemberView,
    route_owner_id: String,
    endpoint: Value,
    assistant: Value,
}
pub struct ProviderVoiceScan {
    worker: Arc<ProviderSessionVoiceWorker>,
    owner: Arc<InstallationOwnership>,
    ingress: ProviderVoiceIngressPolicy,
    binding: Binding,
    since: i64,
    scanned_until: i64,
    scan_until: i64,
    expires: i64,
}
pub struct ProviderVoiceScanCapture {
    scan: ProviderVoiceScan,
    receipts: Vec<InboundVoiceReceipt>,
}
pub struct ProviderVoiceImport {
    pub calls: Vec<ProviderSessionVoiceOrigin>,
    pub imported: usize,
    pub scanned_until: i64,
}
macro_rules! redacted {($($t:ty),*) => {$(impl std::fmt::Debug for $t {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.write_str(concat!(stringify!($t),"(redacted)"))}})*};}
redacted!(
    ProviderVoiceIngressPolicy,
    ProviderVoiceScan,
    ProviderVoiceScanCapture,
    ProviderVoiceImport
);
impl ProviderVoiceScan {
    pub fn current(&self) -> bool {
        self.owner.current() && self.worker.current() && crate::now() < self.expires
    }
    pub fn expires_at(&self) -> i64 {
        self.expires
    }
    pub fn query(&self) -> Result<VoiceQuery> {
        if !self.current() {
            return Err(Error::forbidden());
        }
        Ok(VoiceQuery {
            assistant_id: self.ingress.policy.assistant_id.clone(),
            phone_number_id: self.ingress.policy.phone_number_id.clone(),
            since: self.since,
            scanned_until: self.scanned_until,
            scan_until: self.scan_until,
        })
    }
    pub fn capture(self, receipts: Vec<InboundVoiceReceipt>) -> Result<ProviderVoiceScanCapture> {
        validate_batch(&receipts, &self.ingress.policy)?;
        Ok(ProviderVoiceScanCapture {
            scan: self,
            receipts,
        })
    }
}
fn validate_batch(
    receipts: &[InboundVoiceReceipt],
    policy: &ProviderSessionVoicePolicy,
) -> Result<()> {
    if receipts.len() > 256 {
        return Err(Error::new(
            "capacity-exceeded",
            "Incoming provider batch exceeds its bounded receipt budget",
        ));
    }
    let mut bytes = 0usize;
    for receipt in receipts {
        validate(receipt, policy)?;
        bytes = bytes.saturating_add(
            receipt.call.provider_id.len()
                + receipt.call.assistant_id.len()
                + receipt.call.phone_number_id.len()
                + receipt.call.caller.len()
                + receipt.event_id.len()
                + receipt.call.control_url.as_ref().map_or(0, String::len)
                + receipt.call.ended_reason.as_ref().map_or(0, String::len)
                + 128,
        );
    }
    if bytes > 1024 * 1024 {
        return Err(Error::new(
            "capacity-exceeded",
            "Incoming provider batch exceeds its encoded budget",
        ));
    }
    Ok(())
}
fn text(v: &str, max: usize) -> bool {
    !v.is_empty() && v.len() <= max && !v.chars().any(char::is_control)
}
fn validate(receipt: &InboundVoiceReceipt, p: &ProviderSessionVoicePolicy) -> Result<()> {
    let r = &receipt.call;
    if !text(&r.provider_id, 256)
        || !text(&receipt.event_id, 256)
        || !text(&r.caller, 32)
        || r.assistant_id != p.assistant_id
        || r.phone_number_id != p.phone_number_id
        || r.created_at < 0
        || lifecycle::normalized(&r.state) != Some(receipt.state)
        || !matches!(
            receipt.verification,
            Verification::AuthenticatedResponse
                | Verification::Reconciliation
                | Verification::VerifiedCallback
        )
        || r.control_url.as_ref().is_some_and(|v| !text(v, 4096))
        || r.ended_reason
            .as_ref()
            .is_some_and(|v| v.len() > 256 || v.chars().any(char::is_control))
    {
        return Err(invalid());
    }
    Ok(())
}
fn endpoint(c: &Connection, id: &str) -> Result<Value> {
    let raw:String=c.query_row("SELECT json_object('id',e.id,'identity_id',e.identity_id,'tenant_id',e.tenant_id,'revision',e.revision,'device_id',e.device_id,'device_kind',d.kind,'device_platform',d.platform,'transport',e.transport,'address',e.address,'capabilities',json(e.capabilities_json),'verification',e.verification_state,'secret_ref',e.secret_ref,'route_owner_id',e.route_owner_id) FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.revoked=0 WHERE e.id=?1",[id],|r|r.get(0)).map_err(|_|Error::forbidden())?;
    serde_json::from_str(&raw).map_err(|_| Error::storage())
}
fn binding(c: &Connection, p: &ProviderVoiceIngressPolicy) -> Result<Binding> {
    id(&p.id)?;
    id(&p.conversation_id)?;
    let (ai, phone) = ownership::binding(c, &p.policy)?;
    let actor = actor_from_connection(c, &p.policy.assistant_credential_id)?;
    let room = crate::conversations::authorized_conversation(c, &actor, &p.conversation_id)?;
    if room.closed {
        return Err(Error::forbidden());
    }
    let caller = crate::audience::member(c, &ai, &p.conversation_id, true)?;
    let target = crate::audience::member(c, &phone, &p.conversation_id, true)?;
    if caller.role == "observer"
        || target.role == "observer"
        || caller.identity_id == target.identity_id
    {
        return Err(Error::forbidden());
    }
    for key in [
        &p.policy.assistant_credential_id,
        &p.policy.endpoint_credential_id,
        &p.policy.route_credential_id,
    ] {
        let actor = actor_from_connection(c, key)?;
        if !actor.permits_recipient(&ai) || !actor.permits_recipient(&phone) {
            return Err(Error::forbidden());
        }
    }
    let route = actor_from_connection(c, &p.policy.route_credential_id)?;
    Ok(Binding {
        caller,
        target,
        route_owner_id: route.identity_id,
        endpoint: endpoint(c, &p.policy.endpoint_id)?,
        assistant: endpoint(c, &p.policy.assistant_endpoint_id)?,
    })
}
fn scope_set(
    c: &Connection,
    p: &ProviderVoiceIngressPolicy,
    b: &Binding,
    mode: crate::AuthorityMode,
) -> Result<Vec<AuthorityScope>> {
    let mut scopes = crate::authority_admission::conversation_scopes(c, &p.conversation_id, mode)?;
    scopes.push(AuthorityScope::identity(&b.route_owner_id, mode)?);
    Ok(scopes)
}
fn current(
    c: &Connection,
    role: &ProviderSessionVoiceWorker,
    p: &ProviderVoiceIngressPolicy,
    b: &Binding,
) -> Result<()> {
    if binding(c, p)? != *b {
        return Err(Error::forbidden());
    }
    let row:(String,String,String,String)=c.query_row("SELECT conversation_id,policy_json,source_json,worker_id FROM provider_voice_ingress WHERE id=?1",[&p.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|Error::forbidden())?;
    if row.0 != p.conversation_id
        || serde_json::from_str::<ProviderSessionVoicePolicy>(&row.1)
            .map_err(|_| Error::storage())?
            != p.policy
        || serde_json::from_str::<Binding>(&row.2).map_err(|_| Error::storage())? != *b
        || row.3 != role.id
    {
        return Err(Error::forbidden());
    }
    Ok(())
}
fn request(scan: &ProviderVoiceScan, provider: &str) -> session_control::Request {
    let key = hex::encode(Sha256::digest(format!(
        "{}\0{}\0{}\0{}\0{}",
        scan.ingress.id,
        scan.ingress.policy.provider,
        scan.ingress.policy.route_reference,
        scan.binding.route_owner_id,
        provider
    )));
    session_control::Request {
        operation_id: format!("pinvite_{key}"),
        conversation_id: scan.ingress.conversation_id.clone(),
        session_id: format!("pinsession_{key}"),
        by_participant: Some(scan.binding.caller.participant_id.clone()),
        expected_revision: None,
        action: session_control::Action::Invite {
            participants: vec![scan.binding.target.participant_id.clone()],
            medium: "voice".into(),
            intent: "incoming-call".into(),
            capabilities: json!({"audio":["g.711-mu"]}),
        },
    }
}
impl Store {
    pub fn configure_provider_voice_ingress(
        &self,
        role: &Arc<ProviderSessionVoiceWorker>,
        p: ProviderVoiceIngressPolicy,
    ) -> Result<()> {
        role.require(self)?;
        let held = self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let b = binding(&c, &p)?;
                scope_set(&c, &p, &b, crate::AuthorityMode::Read)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        role.require(self)?;
        let b = binding(&tx, &p)?;
        if !self.authority_covers(&held, &scope_set(&tx, &p, &b, crate::AuthorityMode::Read)?) {
            return Err(Error::forbidden());
        }
        let old:Option<(String,String,String)>=tx.query_row("SELECT conversation_id,policy_json,source_json FROM provider_voice_ingress WHERE id=?1",[&p.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
        if let Some(old) = old {
            if old.0 != p.conversation_id
                || serde_json::from_str::<ProviderSessionVoicePolicy>(&old.1)
                    .map_err(|_| Error::storage())?
                    != p.policy
                || serde_json::from_str::<Binding>(&old.2).map_err(|_| Error::storage())? != b
            {
                return Err(conflict());
            }
            tx.execute(
                "UPDATE provider_voice_ingress SET worker_id=?2 WHERE id=?1",
                params![p.id, role.id],
            )
            .map_err(|_| conflict())?;
        } else {
            let count: u64 = tx
                .query_row("SELECT count(*) FROM provider_voice_ingress", [], |r| {
                    r.get(0)
                })
                .map_err(|_| Error::storage())?;
            if count >= self.limits.operation_receipts {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Incoming provider configuration budget exhausted",
                ));
            }
            let now = crate::now();
            tx.execute("INSERT INTO provider_voice_ingress(id,conversation_id,policy_json,source_json,worker_id,since,scanned_until) VALUES(?1,?2,?3,?4,?5,?6,?6*1000)",params![p.id,p.conversation_id,lifecycle::encoded(&p.policy)?,lifecycle::encoded(&b)?,role.id,now]).map_err(|_|conflict())?;
        }
        tx.commit().map_err(|_| Error::storage())
    }
    pub fn begin_provider_voice_scan(
        &self,
        role: &Arc<ProviderSessionVoiceWorker>,
        p: &ProviderVoiceIngressPolicy,
        manifest: &Manifest,
    ) -> Result<ProviderVoiceScan> {
        role.require(self)?;
        role.before_dispatch()?;
        manifest.validate().map_err(|_| invalid())?;
        if manifest.name != p.policy.provider
            || manifest.address != connect_connector_api::AddressKind::E164Pair
            || !manifest
                .operations
                .contains(&ConnectorOperation::VoiceDiscover)
        {
            return Err(Error::new(
                "unsupported-connector-operation",
                "Incoming discovery is not declared",
            ));
        }
        let c = self.connection()?;
        let b = binding(&c, p)?;
        current(&c, role, p, &b)?;
        let (since, scanned): (i64, i64) = c
            .query_row(
                "SELECT since,scanned_until FROM provider_voice_ingress WHERE id=?1",
                [&p.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| Error::storage())?;
        let upper = chrono::Utc::now()
            .timestamp_millis()
            .min(scanned.saturating_add(60_000))
            .max(scanned);
        let mut expires = crate::now() + i64::from(p.policy.timeout_seconds);
        for credential in [
            &p.policy.assistant_credential_id,
            &p.policy.endpoint_credential_id,
            &p.policy.route_credential_id,
        ] {
            expires = expires.min(actor_from_connection(&c, credential)?.expires_at);
        }
        Ok(ProviderVoiceScan {
            worker: role.clone(),
            owner: self.require_runtime_owner()?,
            ingress: p.clone(),
            binding: b,
            since,
            scanned_until: scanned,
            scan_until: upper,
            expires,
        })
    }
    /// Common Session admission, incoming resource projection, journal and cursor
    /// commit together. A failed batch rolls them all back; replay checks current
    /// configured authority before examining any previously imported resource.
    pub fn record_provider_voice_scan(
        &self,
        captured: &ProviderVoiceScanCapture,
    ) -> Result<ProviderVoiceImport> {
        let scan = &captured.scan;
        scan.worker.require(self)?;
        let held = self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                custody::scopes(&c, scan, &captured.receipts)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        scan.worker.require(self)?;
        current(&tx, &scan.worker, &scan.ingress, &scan.binding)?;
        if !self.authority_covers(&held, &custody::scopes(&tx, scan, &captured.receipts)?) {
            return Err(Error::forbidden());
        }
        let cursor: i64 = tx
            .query_row(
                "SELECT scanned_until FROM provider_voice_ingress WHERE id=?1",
                [&scan.ingress.id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if cursor != scan.scanned_until {
            return Err(conflict());
        }
        let mut calls = vec![];
        let mut imported = 0;
        for receipt in &captured.receipts {
            validate(receipt, &scan.ingress.policy)?;
            let r = &receipt.call;
            if r.created_at < scan.since
                || r.created_at.saturating_mul(1000) > scan.scan_until
                || scan.binding.endpoint.get("address").and_then(Value::as_str)
                    != Some(r.caller.as_str())
            {
                continue;
            }
            let existing:Option<(String,Option<String>,String)>=tx.query_row("SELECT id,ingress_id,direction FROM provider_session_voice WHERE provider_id=?1 AND json_extract(policy_json,'$.provider')=?2 AND json_extract(policy_json,'$.route_reference')=?3 AND json_extract(source_json,'$.route_owner_id')=?4",params![r.provider_id,scan.ingress.policy.provider,scan.ingress.policy.route_reference,scan.binding.route_owner_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
            if existing
                .as_ref()
                .is_some_and(|(_, _, direction)| direction != "inbound")
            {
                return Err(Error::new(
                    "provider-binding-conflict",
                    "Incoming provider resource already has another binding",
                ));
            }
            let Some(admission) = custody::admit(&tx, self, scan, receipt)? else {
                continue;
            };
            let call_id = if let Some((id, ingress, direction)) = existing {
                if admission.existing.as_deref() != Some(id.as_str())
                    || ingress.as_deref() != Some(scan.ingress.id.as_str())
                    || direction != "inbound"
                {
                    return Err(Error::new(
                        "provider-binding-conflict",
                        "Incoming provider resource already has another binding",
                    ));
                }
                id
            } else {
                if admission.existing.is_some() {
                    return Err(Error::storage());
                }
                let id = observed::insert(&tx, self, scan, receipt, &held)?;
                custody::delivered(&tx, self, scan, &admission, &id)?;
                imported += 1;
                id
            };
            if !calls
                .iter()
                .any(|o: &ProviderSessionVoiceOrigin| o.id() == call_id)
            {
                calls.push(observed::origin(&tx, self, scan, call_id)?);
            }
        }
        tx.execute(
            "UPDATE provider_voice_ingress SET scanned_until=?2 WHERE id=?1",
            params![scan.ingress.id, scan.scan_until],
        )
        .map_err(|_| Error::storage())?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(ProviderVoiceImport {
            calls,
            imported,
            scanned_until: scan.scan_until,
        })
    }
}
