//! Current target facts for a protected initializer; these are not an IO grant.
use super::*;
pub struct NativeConsultationTarget {
    pub identity_id: String,
    pub participant_id: String,
    pub membership_epoch: u64,
    pub kind: String,
    pub present: bool,
    pub registered: bool,
    /// Original protected reservation, never inferred from public presence.
    pub candidate_kind: Option<String>,
}
impl Store {
    pub fn native_consultation_target(
        &self,
        origin: &NativeConsultationOrigin,
    ) -> Result<NativeConsultationTarget> {
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                origin.require(self, &c).map(|(_, scopes)| scopes)
            },
        )?;
        let c = self.connection()?;
        let (_, scopes) = origin.require(self, &c)?;
        if !self.authority_covers(&held, &scopes) {
            return Err(Error::forbidden());
        }
        let TransitionIntent::Consult {
            private_participants,
            ..
        } = &origin.request.intent
        else {
            return Err(Error::forbidden());
        };
        if private_participants.len() != 2 {
            return Err(Error::new(
                "unsupported-media-profile",
                "Private initialization requires two private Participants",
            ));
        }
        let primary = origin
            .request
            .connection_id
            .as_ref()
            .ok_or_else(Error::forbidden)?;
        let primary_pid: String = c
            .query_row(
                "SELECT participant_id FROM connections WHERE id=?1",
                [primary],
                |r| r.get(0),
            )
            .map_err(|_| Error::forbidden())?;
        let target = private_participants
            .iter()
            .find(|p| **p != primary_pid)
            .ok_or_else(Error::forbidden)?;
        let (identity,epoch,kind,identity_kind,present):(String,u64,String,String,bool)=c.query_row("SELECT p.identity_id,p.membership_epoch,p.kind,i.kind,COALESCE(sp.state='present',0) FROM operation_members m JOIN participants p ON p.participant_id=m.participant_id AND p.identity_id=m.identity_id AND p.membership_epoch=m.membership_epoch AND p.active=1 JOIN identities i ON i.id=p.identity_id AND i.revoked=0 LEFT JOIN session_participants sp ON sp.session_id=?3 AND sp.participant_id=p.participant_id AND sp.membership_epoch=p.membership_epoch WHERE m.operation_record_id=?1 AND m.participant_id=?2",params![origin.record,target,origin.request.session_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|_|Error::forbidden())?;
        if kind != identity_kind {
            return Err(Error::forbidden());
        }
        let candidate_kind:Option<String>=c.query_row("SELECT candidate_kind FROM consultation_candidate_bindings WHERE operation_record_id=?1",[&origin.record],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
        Ok(NativeConsultationTarget {
            identity_id: identity,
            participant_id: target.clone(),
            membership_epoch: epoch,
            kind,
            present,
            registered: candidate_kind.is_some(),
            candidate_kind,
        })
    }
}
