//! Initial allocation custody for an original private SIP reservation.
use super::*;
use crate::SipConsultationReservation;

pub(super) fn source(
    store: &Store,
    c: &Connection,
    caller: &Actor,
    connection: &str,
    policy: &InitialTelephonePolicy,
) -> Result<(
    (InitialTelephoneSource, Actor, Actor, i64),
    Vec<AuthorityScope>,
)> {
    let reservation = SipConsultationReservation::from_connection(store, c, connection)?;
    let (origin, original_policy, required) = reservation.current_parts(store, c)?;
    if caller.credential_id != origin.credential || policy != original_policy {
        return Err(Error::forbidden());
    }
    let endpoint = actor_from_connection(c, &policy.endpoint_credential_id)?;
    let route = actor_from_connection(c, &policy.route_credential_id)?;
    let caller_member = crate::audience::member(
        c,
        &caller.identity_id,
        &origin.request.conversation_id,
        true,
    )?;
    let target = crate::audience::member(
        c,
        &endpoint.identity_id,
        &origin.request.conversation_id,
        true,
    )?;
    let TransitionIntent::Consult {
        private_participants,
        ..
    } = &origin.request.intent
    else {
        return Err(Error::forbidden());
    };
    let mut audience = vec![];
    for pid in private_participants {
        let identity: String = c.query_row("SELECT identity_id FROM operation_members WHERE operation_record_id=?1 AND participant_id=?2",params![origin.record_id(),pid],|r|r.get(0)).map_err(|_|Error::storage())?;
        audience.push(crate::audience::member(
            c,
            &identity,
            &origin.request.conversation_id,
            true,
        )?);
    }
    audience.sort_by(|a, b| a.participant_id.cmp(&b.participant_id));
    let (revision,device,kind,platform,address,owner,transport):(u64,String,String,String,String,String,String)=c.query_row("SELECT e.revision,e.device_id,d.kind,d.platform,e.address,e.route_owner_id,e.transport FROM endpoints e JOIN devices d ON d.id=e.device_id WHERE e.id=?1",[&policy.endpoint_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).map_err(|_|Error::storage())?;
    let expires = caller
        .expires_at
        .min(endpoint.expires_at)
        .min(route.expires_at)
        .min(origin.request.expires_at);
    Ok((
        (
            InitialTelephoneSource {
                parent: InitialTelephoneParent::Consultation {
                    record_id: origin.record_id().into(),
                    connection_id: connection.into(),
                },
                conversation_id: origin.request.conversation_id.clone(),
                session_id: origin.request.session_id.clone(),
                caller: caller_member,
                target,
                audience,
                endpoint_id: policy.endpoint_id.clone(),
                endpoint_revision: revision,
                device_id: device,
                device_kind: kind,
                device_platform: platform,
                destination: crate::session_gateways::destination(&transport, &address, policy)?,
                address,
                route_owner_id: owner,
            },
            endpoint,
            route,
            expires,
        ),
        required,
    ))
}

pub(super) fn allocation_scopes(
    store: &Store,
    c: &Connection,
    origin: &InitialTelephoneOrigin,
    write: bool,
) -> Result<Vec<AuthorityScope>> {
    let mut required = scopes(&origin.source, write)?;
    if let InitialTelephoneParent::Consultation { connection_id, .. } = &origin.source.parent {
        let caller = actor_from_connection(c, &origin.credential)?;
        required.extend(source(store, c, &caller, connection_id, &origin.policy)?.1);
    }
    AuthorityScope::merged(&required)
}

impl Store {
    /// Materialize the private initial-call intent on the already reserved ID.
    /// This grants no public invitation, physical allocation or media readiness.
    pub fn prepare_private_initial_telephone(
        &self,
        reservation: &SipConsultationReservation,
    ) -> Result<InitialTelephoneOrigin> {
        let held = self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let (origin, policy, mut required) = reservation.current_parts(self, &c)?;
                let caller = actor_from_connection(&c, &origin.credential)?;
                let (source, _, _, _) =
                    source(self, &c, &caller, reservation.connection_id(), policy)?.0;
                required.extend(scopes(&source, true)?);
                AuthorityScope::merged(&required)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let (parent, policy, mut required) = reservation.current_parts(self, &tx)?;
        let caller = actor_from_connection(&tx, &parent.credential)?;
        let (source, _, _, expiry) =
            source(self, &tx, &caller, reservation.connection_id(), policy)?.0;
        required.extend(scopes(&source, true)?);
        if !self.authority_covers(&held, &AuthorityScope::merged(&required)?) {
            return Err(Error::forbidden());
        }
        let (hash, json) = digest(
            &serde_json::json!({"parent":source.parent,"target":source.target.participant_id,"policy":policy}),
        )?;
        let operation = format!(
            "tpinit_{}",
            hex::encode(Sha256::digest(format!(
                "{}\0{}",
                parent.record_id(),
                reservation.connection_id()
            )))
        );
        let tenant = authority::tenant(&tx, &caller)?;
        let record = if let Some(row) = rows::find(&tx, &tenant, &caller.identity_id, &operation)? {
            if !row.initial_telephone
                || row.digest != hash
                || row.record.connection_id.as_deref() != Some(reservation.connection_id())
            {
                return Err(conflict());
            }
            read(&tx, &caller, &row)?;
            row.record.record_id
        } else {
            capacity(&tx, self, 1)?;
            let unclaimed:bool=tx.query_row("SELECT state='accepted' AND lease_owner IS NULL AND cancel_requested=0 AND NOT EXISTS(SELECT 1 FROM operation_effects WHERE operation_record_id=?1) FROM operations WHERE record_id=?1",[parent.record_id()],|r|r.get(0)).map_err(|_|Error::storage())?;
            let unknown:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM connections WHERE session_id=?1 AND state='unknown')",[&source.session_id],|r|r.get(0)).map_err(|_|Error::storage())?;
            if !unclaimed || unknown {
                return Err(conflict());
            }
            let record = format!("opr_{}", uuid::Uuid::new_v4());
            let expires = (crate::now() + i64::from(policy.timeout_seconds)).min(expiry);
            tx.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,conversation_id,session_id,connection_id,intent_json,state,created_at,updated_at,expires_at,ledger_version) VALUES(?1,?2,?3,?4,?5,'connection.offer',?6,?7,?8,?9,'accepted',?10,?10,?11,1)",params![record,tenant,caller.identity_id,operation,hash,source.conversation_id,source.session_id,reservation.connection_id(),json,crate::now(),expires]).map_err(|_|Error::storage())?;
            tx.execute("INSERT INTO initial_telephone_operations(operation_record_id,parent_record_id,parent_kind,caller_credential_id,target_participant_id,endpoint_id,source_json,policy_json) VALUES(?1,?2,'consultation',?3,?4,?5,?6,?7)",params![record,parent.record_id(),caller.credential_id,source.target.participant_id,policy.endpoint_id,serde_json::to_string(&source).map_err(|_|Error::storage())?,serde_json::to_string(policy).map_err(|_|Error::storage())?]).map_err(|_|Error::storage())?;
            for member in &source.audience {
                tx.execute(
                    "INSERT INTO operation_members VALUES(?1,?2,?3,?4)",
                    params![
                        record,
                        member.participant_id,
                        member.identity_id,
                        member.membership_epoch
                    ],
                )
                .map_err(|_| Error::storage())?;
            }
            tx.execute(
                "INSERT INTO operation_endpoints VALUES(?1,?2,?3)",
                params![record, policy.endpoint_id, source.endpoint_revision],
            )
            .map_err(|_| Error::storage())?;
            rows::event(&tx, &record, "operation_accepted", self)?;
            record
        };
        let row = rows::by_record(&tx, &record)?;
        let (saved_credential,saved_source,saved_policy):(String,String,String)=tx.query_row("SELECT caller_credential_id,source_json,policy_json FROM initial_telephone_operations WHERE operation_record_id=?1",[&record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|Error::storage())?;
        if saved_credential != caller.credential_id
            || serde_json::from_str::<InitialTelephoneSource>(&saved_source)
                .map_err(|_| Error::storage())?
                != source
            || serde_json::from_str::<InitialTelephonePolicy>(&saved_policy)
                .map_err(|_| Error::storage())?
                != *policy
        {
            return Err(Error::forbidden());
        }
        let origin = InitialTelephoneOrigin {
            owner: Arc::downgrade(&self.require_runtime_owner()?),
            runtime: self.runtime_epoch().into(),
            credential: caller.credential_id,
            record,
            operation,
            connection: reservation.connection_id().into(),
            source,
            policy: policy.clone(),
            expires_at: row.record.expires_at.ok_or_else(Error::storage)?,
        };
        tx.commit().map_err(|_| Error::storage())?;
        Ok(origin)
    }
}
