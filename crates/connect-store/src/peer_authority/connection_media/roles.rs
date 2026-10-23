//! One complete current physical footprint for durable Session role projection.
//! This grants no role command; the host must bind only the already stored role.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
pub struct SessionRoleMediaRead {
    native: BTreeMap<String, NativeMediaRead>,
    telephone: BTreeMap<String, crate::TelephoneMediaRead>,
}
impl SessionRoleMediaRead {
    pub fn current(&self) -> bool {
        (!self.native.is_empty() || !self.telephone.is_empty())
            && self.native.values().all(NativeMediaRead::current)
            && self
                .telephone
                .values()
                .all(crate::TelephoneMediaRead::current)
    }
    pub fn native(&self, id: &str) -> Option<&NativeMediaRead> {
        self.native.get(id)
    }
    pub fn telephone(&self, id: &str) -> Option<&crate::TelephoneMediaRead> {
        self.telephone.get(id)
    }
}
impl Store {
    pub fn try_session_role_media_read(
        &self,
        native: &[Arc<NativeMediaOrigin>],
        telephone: &[Arc<crate::TelephoneMediaOrigin>],
        conversation: &str,
        session: &str,
        participant: &str,
        epoch: u64,
    ) -> Result<SessionRoleMediaRead> {
        if native.len() + telephone.len() > self.limits().connections
            || native.is_empty() && telephone.is_empty()
        {
            return Err(Error::forbidden());
        }
        let c = self.try_connection()?;
        let mut required = Vec::new();
        for source in native {
            required.extend(facts(self, &c, source)?.1);
        }
        for source in telephone {
            required.extend(self.telephone_established_scopes(&c, source)?);
        }
        let held = self.try_acquire_authority(&AuthorityScope::merged(&required)?)?;
        let mut required = Vec::new();
        let mut seen = BTreeSet::new();
        let mut native_reads = BTreeMap::new();
        let mut telephone_reads = BTreeMap::new();
        let owner = self.require_runtime_owner()?;
        for source in native {
            let (view, scopes, expires) = facts(self, &c, source)?;
            if view.connection.conversation_id != conversation
                || view.connection.session_id != session
                || view.connection.participant_id != participant
                || view.connection.membership_epoch != epoch
                || !seen.insert((
                    view.connection.connection_id.clone(),
                    view.connection.generation,
                ))
            {
                return Err(Error::forbidden());
            }
            required.extend(scopes);
            let may_subscribe = subscription_permission(&c, source)?;
            native_reads.insert(
                source.runtime_connection_id().into(),
                NativeMediaRead {
                    target: view,
                    source: None,
                    _scopes: held.clone(),
                    owner: owner.clone(),
                    expires_at: expires.min(crate::now() + 5),
                    may_subscribe,
                },
            );
        }
        for source in telephone {
            let read = self.telephone_media_established_in(
                &c,
                source.clone(),
                held.clone(),
                crate::now() + 5,
            )?;
            let s = read.snapshot();
            if s.conversation_id != conversation
                || s.session_id != session
                || s.participant_id != participant
                || s.membership_epoch != epoch
                || !seen.insert((s.connection_id.clone(), s.generation))
            {
                return Err(Error::forbidden());
            }
            required.extend(self.telephone_established_scopes(&c, source)?);
            telephone_reads.insert(s.runtime_connection_id.clone(), read);
        }
        if !self.authority_covers(&held, &required) {
            return Err(Error::forbidden());
        }
        // Every live observed physical source of this member must be represented.
        // A partial registry snapshot cannot lend another device's permissions.
        let mut q=c.prepare("SELECT n.id,n.generation FROM connections n JOIN sessions s ON s.id=n.session_id WHERE s.conversation_id=?1 AND n.session_id=?2 AND n.participant_id=?3 AND n.membership_epoch=?4 AND n.state IN ('ready','active','held') AND n.observed_runtime_id=?5 AND (EXISTS(SELECT 1 FROM native_connection_lifetimes l WHERE l.connection_id=n.id AND l.generation=n.generation AND l.runtime_epoch=?5 AND l.retired=0) OR EXISTS(SELECT 1 FROM telephone_connection_lifetimes l WHERE l.connection_id=n.id AND l.generation=n.generation AND l.runtime_epoch=?5 AND l.retired=0)) LIMIT ?6").map_err(|_|Error::storage())?;
        let current = q
            .query_map(
                params![
                    conversation,
                    session,
                    participant,
                    epoch,
                    self.runtime_epoch(),
                    self.limits().connections + 1
                ],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)),
            )
            .map_err(|_| Error::storage())?
            .collect::<std::result::Result<BTreeSet<_>, _>>()
            .map_err(|_| Error::storage())?;
        if current != seen {
            return Err(Error::forbidden());
        }
        let read = SessionRoleMediaRead {
            native: native_reads,
            telephone: telephone_reads,
        };
        if !read.current() {
            return Err(Error::forbidden());
        }
        Ok(read)
    }
}
