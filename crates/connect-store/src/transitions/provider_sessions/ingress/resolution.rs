//! Original private candidate selection through the owned read-only adapter.
use super::*;
use crate::incoming_contacts::{self as contacts, Candidate};
use connect_protocol::incoming_contacts::*;
use custody::{Admission, Snapshot};
const KIND: &str = "incoming-contact.resolve-voice";

pub enum ProviderIncomingVoiceResolutionPreparation {
    Completed(IncomingContact),
    Query(ProviderIncomingVoiceResolution),
}
/// Only Store can mint this exact original-resource query. It carries no
/// dispatch authority for creation, acceptance or media.
pub struct ProviderIncomingVoiceResolution {
    scan: ProviderVoiceScan,
    administrator: String,
    selection: IncomingContactSelection,
    candidate: Candidate,
    payload: Value,
}
pub struct ProviderIncomingVoiceResolutionCapture {
    ticket: ProviderIncomingVoiceResolution,
    receipt: InboundVoiceReceipt,
}
pub struct ProviderIncomingVoiceResolutionResult {
    pub contact: IncomingContact,
    pub origin: Option<ProviderSessionVoiceOrigin>,
}
macro_rules! private_debug {($($t:ty),*) => {$(impl std::fmt::Debug for $t {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.write_str(concat!(stringify!($t),"(redacted)"))}})*};}
private_debug!(
    ProviderIncomingVoiceResolutionPreparation,
    ProviderIncomingVoiceResolution,
    ProviderIncomingVoiceResolutionCapture,
    ProviderIncomingVoiceResolutionResult
);

fn unavailable() -> Error {
    Error::new(
        "provider-unavailable",
        "Original incoming resource requires authenticated reconciliation",
    )
}
fn payload(receipt: &InboundVoiceReceipt) -> Value {
    let r = &receipt.call;
    json!({"kind":"voice","provider_id":r.provider_id,"assistant_id":r.assistant_id,"phone_number_id":r.phone_number_id,"caller":r.caller,"created_at":r.created_at})
}
impl ProviderIncomingVoiceResolution {
    pub fn policy(&self) -> &ProviderSessionVoicePolicy {
        &self.scan.ingress.policy
    }
    pub fn expires_at(&self) -> i64 {
        self.scan.expires
    }
    pub fn query(&self) -> Result<VoiceQuery> {
        if !self.scan.current() {
            return Err(Error::forbidden());
        }
        let created = self.payload["created_at"]
            .as_i64()
            .ok_or_else(Error::storage)?;
        // A fresh account-authenticated read of the original one-second
        // creation window, independent of advancing background scan cursors.
        Ok(VoiceQuery {
            assistant_id: self.scan.ingress.policy.assistant_id.clone(),
            phone_number_id: self.scan.ingress.policy.phone_number_id.clone(),
            since: created,
            scanned_until: created.saturating_mul(1000),
            scan_until: created.saturating_mul(1000).saturating_add(999),
        })
    }
    pub fn capture(
        self,
        receipts: Vec<InboundVoiceReceipt>,
    ) -> Result<ProviderIncomingVoiceResolutionCapture> {
        if !self.scan.current() {
            return Err(Error::forbidden());
        }
        validate_batch(&receipts, &self.scan.ingress.policy)?;
        let original = self.payload["provider_id"]
            .as_str()
            .ok_or_else(Error::storage)?;
        let mut found = None;
        for receipt in receipts {
            validate(&receipt, &self.scan.ingress.policy)?;
            if receipt.call.provider_id != original {
                continue;
            }
            if payload(&receipt) != self.payload {
                return Err(conflict());
            }
            // Multiple contradictory snapshots cannot choose a state by order.
            if let Some(old) = &found {
                let old: &InboundVoiceReceipt = old;
                if old.call.state != receipt.call.state
                    || old.call.control_url != receipt.call.control_url
                    || old.call.ended_reason != receipt.call.ended_reason
                {
                    return Err(unavailable());
                }
            } else {
                found = Some(receipt);
            }
        }
        Ok(ProviderIncomingVoiceResolutionCapture {
            ticket: self,
            receipt: found.ok_or_else(unavailable)?,
        })
    }
}
fn checked(
    c: &Connection,
    role: &ProviderSessionVoiceWorker,
    credential: &str,
    selection: &IncomingContactSelection,
) -> Result<(crate::Actor, String, Candidate, Snapshot, IncomingContact)> {
    let (actor, tenant) = contacts::admin(c, credential)?;
    let original = contacts::visible(c, &actor, &tenant, &selection.contact_id)?;
    let chosen = original
        .into_iter()
        .find(|p| p.view.id == selection.candidate_id)
        .ok_or_else(Error::forbidden)?;
    let contact = contacts::view(c, &actor, &tenant, &selection.contact_id)?;
    if contact.kind != IncomingContactKind::Voice
        || matches!(
            contact.reason,
            IncomingContactReason::ResourceConflict | IncomingContactReason::Ineligible
        )
    {
        return Err(conflict());
    }
    let snapshot: Snapshot =
        serde_json::from_value(chosen.source.clone()).map_err(|_| Error::storage())?;
    current(c, role, &snapshot.ingress, &snapshot.binding)?;
    if snapshot.binding.route_owner_id != actor.identity_id {
        return Err(Error::forbidden());
    }
    Ok((actor, tenant, chosen, snapshot, contact))
}
fn intent(selection: &IncomingContactSelection) -> Result<String> {
    contacts::encoded(&(
        selection.contact_id.as_str(),
        selection.expected_revision,
        selection.candidate_id.as_str(),
    ))
}
fn replay(
    c: &Connection,
    actor: &crate::Actor,
    tenant: &str,
    selection: &IncomingContactSelection,
) -> Result<bool> {
    crate::operations::reject_control_id(c, actor, &selection.operation_id)?;
    let old: Option<(String,String,bool)>=c.query_row("SELECT digest,command_type,tombstoned FROM operations WHERE tenant_id=?1 AND owner_id=?2 AND operation_id=?3",params![tenant,actor.identity_id,selection.operation_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
    if let Some((digest, kind, tombstoned)) = old {
        if digest != hex::encode(Sha256::digest(intent(selection)?.as_bytes())) || kind != KIND {
            return Err(conflict());
        }
        if tombstoned {
            return Err(Error::new("gone", "Incoming decision receipt has expired"));
        }
        return Ok(true);
    }
    Ok(false)
}
fn scopes(
    c: &Connection,
    role: &ProviderSessionVoiceWorker,
    credential: &str,
    selection: &IncomingContactSelection,
    mode: crate::AuthorityMode,
) -> Result<Vec<AuthorityScope>> {
    let (_, _, _, snapshot, _) = checked(c, role, credential, selection)?;
    scope_set(c, &snapshot.ingress, &snapshot.binding, mode)
}
impl Store {
    pub fn prepare_incoming_voice_resolution(
        &self,
        role: &Arc<ProviderSessionVoiceWorker>,
        administrator: &str,
        selection: IncomingContactSelection,
        manifest: &Manifest,
    ) -> Result<ProviderIncomingVoiceResolutionPreparation> {
        selection.validate()?;
        role.require(self)?;
        let held = self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                scopes(
                    &*self.connection()?,
                    role,
                    administrator,
                    &selection,
                    crate::AuthorityMode::Read,
                )
            },
        )?;
        let c = self.connection()?;
        if !self.authority_covers(
            &held,
            &scopes(
                &c,
                role,
                administrator,
                &selection,
                crate::AuthorityMode::Read,
            )?,
        ) {
            return Err(Error::forbidden());
        }
        let (actor, tenant, candidate, snapshot, contact) =
            checked(&c, role, administrator, &selection)?;
        if replay(&c, &actor, &tenant, &selection)? {
            return Ok(ProviderIncomingVoiceResolutionPreparation::Completed(
                contact,
            ));
        }
        if contact.state != IncomingContactState::Held
            || contact.revision != selection.expected_revision
        {
            return Err(conflict());
        }
        let raw: String = c
            .query_row(
                "SELECT payload_json FROM incoming_contact_holds WHERE id=?1",
                [&selection.contact_id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        let payload: Value = serde_json::from_str(&raw).map_err(|_| Error::storage())?;
        if payload["created_at"]
            .as_i64()
            .is_none_or(|at| at < snapshot.since)
        {
            return Err(Error::forbidden());
        }
        drop(c);
        let mut scan = self.begin_provider_voice_scan(role, &snapshot.ingress, manifest)?;
        if scan.binding != snapshot.binding {
            return Err(Error::forbidden());
        }
        scan.expires = scan.expires.min(actor.expires_at);
        Ok(ProviderIncomingVoiceResolutionPreparation::Query(
            ProviderIncomingVoiceResolution {
                scan,
                administrator: administrator.into(),
                selection,
                candidate,
                payload,
            },
        ))
    }
    pub fn record_incoming_voice_resolution(
        &self,
        captured: &ProviderIncomingVoiceResolutionCapture,
    ) -> Result<ProviderIncomingVoiceResolutionResult> {
        let ticket = &captured.ticket;
        let scan = &ticket.scan;
        let selection = &ticket.selection;
        scan.worker.require(self)?;
        if !scan.current() {
            return Err(Error::forbidden());
        }
        let held = self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                scopes(
                    &*self.connection()?,
                    &scan.worker,
                    &ticket.administrator,
                    selection,
                    crate::AuthorityMode::Write,
                )
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        scan.worker.require(self)?;
        if !scan.current()
            || !self.authority_covers(
                &held,
                &scopes(
                    &tx,
                    &scan.worker,
                    &ticket.administrator,
                    selection,
                    crate::AuthorityMode::Write,
                )?,
            )
        {
            return Err(Error::forbidden());
        }
        let (actor, tenant, candidate, snapshot, contact) =
            checked(&tx, &scan.worker, &ticket.administrator, selection)?;
        if contacts::encoded(&candidate)? != contacts::encoded(&ticket.candidate)?
            || snapshot.ingress != scan.ingress
            || snapshot.binding != scan.binding
        {
            return Err(Error::forbidden());
        }
        if replay(&tx, &actor, &tenant, selection)? {
            return Ok(ProviderIncomingVoiceResolutionResult {
                contact,
                origin: None,
            });
        }
        if contact.state != IncomingContactState::Held
            || contact.revision != selection.expected_revision
        {
            return Err(conflict());
        }
        validate(&captured.receipt, &scan.ingress.policy)?;
        if payload(&captured.receipt) != ticket.payload {
            return Err(conflict());
        }
        let exists: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM provider_session_voice WHERE provider_id=?1 AND json_extract(policy_json,'$.provider')=?2 AND json_extract(policy_json,'$.route_reference')=?3 AND json_extract(source_json,'$.route_owner_id')=?4)",params![captured.receipt.call.provider_id,scan.ingress.policy.provider,scan.ingress.policy.route_reference,scan.binding.route_owner_id],|r|r.get(0)).map_err(|_|Error::storage())?;
        if exists {
            return Err(Error::new(
                "provider-binding-conflict",
                "Incoming resource already has a provider binding",
            ));
        }
        let call = observed::insert(&tx, self, scan, &captured.receipt, &held)?;
        let admission = Admission {
            hold: selection.contact_id.clone(),
            candidate,
            existing: None,
        };
        custody::bind(
            &tx,
            self,
            &admission,
            &call,
            &actor,
            &selection.operation_id,
            KIND,
            &intent(selection)?,
        )?;
        let contact = contacts::view(&tx, &actor, &tenant, &selection.contact_id)?;
        let origin = observed::origin(&tx, self, scan, call)?;
        if !scan.current() || actor.expires_at <= crate::now() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(ProviderIncomingVoiceResolutionResult {
            contact,
            origin: Some(origin),
        })
    }
}
