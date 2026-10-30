//! One complete current physical pair; logical ownership is not subscribe permission.
use super::lifetime::{scopes, TelephoneMediaOrigin, TelephoneMediaRead};
use super::*;
pub struct TelephoneNativeMediaRead {
    telephone: TelephoneMediaRead,
    native: crate::NativeMediaRead,
    native_receive: bool,
    telephone_receive: bool,
}
redacted!(TelephoneNativeMediaRead);
impl TelephoneNativeMediaRead {
    pub fn current(&self) -> bool {
        self.telephone.current() && self.native.current()
    }
    pub fn telephone(&self) -> &TelephoneMediaRead {
        &self.telephone
    }
    pub fn native(&self) -> &crate::NativeMediaRead {
        &self.native
    }
    pub fn permits_native_receive(&self) -> bool {
        self.current() && self.native_receive
    }
    pub fn permits_telephone_receive(&self) -> bool {
        self.current() && self.telephone_receive
    }
}
impl Store {
    pub fn try_telephone_native_media_read(
        &self,
        telephone: Arc<TelephoneMediaOrigin>,
        native: Arc<crate::NativeMediaOrigin>,
    ) -> Result<TelephoneNativeMediaRead> {
        let s = telephone.snapshot();
        if native.conversation_id() != s.conversation_id || native.session_id() != s.session_id {
            return Err(Error::forbidden());
        }
        let c = self.try_connection()?;
        let mut required = scopes(&c, s)?;
        required.extend(
            self.native_scene_facts(&c, &[native.clone()], &s.conversation_id, &s.session_id)?
                .1,
        );
        let held = self.try_acquire_authority(&crate::AuthorityScope::merged(&required)?)?;
        let (facts, current, expiry) =
            self.native_scene_facts(&c, &[native.clone()], &s.conversation_id, &s.session_id)?;
        if !self.authority_covers(&held, &current) {
            return Err(Error::forbidden());
        }
        let mut reads = self.native_scene_reads(facts, held, expiry)?;
        let read = reads
            .remove(&(native.runtime_connection_id().into(), None))
            .ok_or_else(Error::forbidden)?;
        self.telephone_native_pair_in(&c, telephone, &native, read)
    }
    /// Exact signed native stream command, sharing its original complete proof.
    /// No second reader may be acquired behind a queued authority writer.
    pub fn telephone_native_stream_read_with_peer(
        &self,
        telephone: Arc<TelephoneMediaOrigin>,
        native: &Arc<crate::NativeMediaOrigin>,
        proof: &crate::PeerAuthorityGuard,
        lease: &crate::NativeStreamLease,
    ) -> Result<TelephoneNativeMediaRead> {
        let read = self.native_stream_media_read_with_peer(native, None, proof, lease)?;
        let c = self.try_connection()?;
        self.telephone_native_pair_in(&c, telephone, native, read)
    }
    /// Read-only preflight shares the accepted stream command's source readers.
    pub fn telephone_native_stream_preflight_read_with_peer(
        &self,
        telephone: Arc<TelephoneMediaOrigin>,
        native: &Arc<crate::NativeMediaOrigin>,
        proof: &crate::PeerAuthorityGuard,
    ) -> Result<TelephoneNativeMediaRead> {
        let (read, _) = self.native_stream_preflight_media_reads_with_peer(native, &[], proof)?;
        let c = self.try_connection()?;
        self.telephone_native_pair_in(&c, telephone, native, read)
    }
    fn telephone_native_pair_in(
        &self,
        c: &Connection,
        telephone: Arc<TelephoneMediaOrigin>,
        native: &crate::NativeMediaOrigin,
        read: crate::NativeMediaRead,
    ) -> Result<TelephoneNativeMediaRead> {
        let s = telephone.snapshot();
        if native.conversation_id() != s.conversation_id
            || native.session_id() != s.session_id
            || !read.current()
            || read.target().runtime_connection_id != native.runtime_connection_id()
            || read.target().connection.generation != native.generation()
        {
            return Err(Error::forbidden());
        }
        let phone = self.telephone_media_established_in(
            c,
            telephone,
            read.interval().clone(),
            read.expires_at(),
        )?;
        self.require_sip_native_pair_context(
            c,
            &phone.snapshot().connection_id,
            native,
            read.target().private_consultation.as_deref(),
        )?;
        let actor = self.native_media_route_actor(c, native)?;
        if !phone
            .route_actor()
            .permits_recipient(&read.target().session.actor.identity_id)
            || !actor.permits_recipient(&phone.member().identity_id)
        {
            return Err(Error::forbidden());
        }
        let native_receive = read.permits_subscriptions();
        let telephone_receive = phone.route_actor().permits("stream.subscribe");
        Ok(TelephoneNativeMediaRead {
            telephone: phone,
            native: read,
            native_receive,
            telephone_receive,
        })
    }
    /// Planning metadata only. Actual current origin/binding and named receive
    /// permission must still be resolved independently by the installed host.
    pub(crate) fn telephone_scene_scopes(
        &self,
        c: &Connection,
        cid: &str,
        sid: &str,
    ) -> Result<Vec<crate::AuthorityScope>> {
        let mut q=c.prepare("SELECT DISTINCT json_extract(t.descriptor_json,'$.route_owner_id') FROM telephone_connection_lifetimes l JOIN telephone_replacement_commits t ON t.operation_record_id=l.operation_record_id JOIN connections n ON n.id=l.connection_id JOIN sessions s ON s.id=n.session_id WHERE s.conversation_id=?1 AND n.session_id=?2 AND l.runtime_epoch=?3 AND l.retired=0 AND n.generation=l.generation AND n.observed_runtime_id=l.runtime_epoch AND n.state IN ('ready','active') UNION SELECT DISTINCT json_extract(t.source_json,'$.route_owner_id') FROM initial_telephone_operations t JOIN operations o ON o.record_id=t.operation_record_id JOIN initial_telephone_answers a ON a.operation_record_id=t.operation_record_id JOIN connections n ON n.id=o.connection_id WHERE o.conversation_id=?1 AND o.session_id=?2 AND t.attempt_runtime=?3 AND a.accepted=1 AND a.connected_at IS NOT NULL AND a.retired_at IS NULL AND n.generation=1 AND n.observed_runtime_id=?3 AND n.state IN ('ready','active') ORDER BY 1 LIMIT ?4").map_err(|_|Error::storage())?;
        let owners = q
            .query_map(
                params![
                    cid,
                    sid,
                    self.runtime_epoch(),
                    self.limits().connections + 1
                ],
                |r| r.get::<_, String>(0),
            )
            .map_err(|_| Error::storage())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| Error::storage())?;
        if owners.len() > self.limits().connections {
            return Err(Error::forbidden());
        }
        owners
            .iter()
            .map(|id| crate::AuthorityScope::identity(id, crate::AuthorityMode::Read))
            .collect()
    }
}
