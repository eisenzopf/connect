//! Actual native or established telephone source; neither borrows the other's identity.
use super::*;
use connect_store::TelephoneNativeMediaRead;
use rvoip_core::staged_streams::MediaParticipantSource;
use serde_json::{json, Value};
#[derive(Clone)]
pub(super) enum StreamSource {
    Initial(
        Arc<crate::initial_telephone::policy::Leg>,
        Arc<connect_store::InitialTelephoneNativeRead>,
    ),
    Native(Arc<Connection>, Arc<NativeMediaRead>),
    Telephone(
        Arc<crate::telephone_replacement::media::Policy>,
        Arc<TelephoneNativeMediaRead>,
    ),
}
fn receiver(
    target: &Connection,
    view: &PeerConnectionProjection,
    principal: &AuthenticatedPrincipal,
) -> ControlResult<MediaOwner> {
    let owner = target.owner(view, principal, true)?;
    target
        .activation()?
        .ok_or(StreamControlError::NotReady)?
        .current(owner)
}
impl StreamSource {
    pub(super) fn descriptor(&self) -> Value {
        match self {
            Self::Initial(leg, read) => {
                let s = read.phone().source();
                json!({"connection_id":leg.origin().ok().map(|o|o.connection_id().to_owned()),"runtime_connection_id":leg.connection().as_str(),"participant_id":s.target.participant_id,"membership_epoch":s.target.membership_epoch,"generation":1})
            }
            Self::Native(entry, read) => {
                json!({"connection_id":entry.origin.connection_id(),"runtime_connection_id":entry.origin.runtime_connection_id(),"participant_id":read.source().map(|v|v.connection.participant_id.as_str()),"membership_epoch":entry.origin.membership_epoch(),"generation":entry.origin.generation()})
            }
            Self::Telephone(_, read) => {
                let s = read.telephone().snapshot();
                json!({"connection_id":s.connection_id,"runtime_connection_id":s.runtime_connection_id,"participant_id":s.participant_id,"membership_epoch":s.membership_epoch,"generation":s.generation})
            }
        }
    }
    pub(super) fn receive(
        &self,
        target: &Connection,
        principal: &AuthenticatedPrincipal,
        source: &MediaSource,
    ) -> ControlResult<Box<MediaOwner>> {
        match self {
            Self::Initial(leg, read) => {
                if !read.current()
                    || !read.native().permits_subscriptions()
                    || read.native().target().handoff_preparation
                    || read.native().target().publication_suspended
                {
                    return Err(StreamControlError::Authority);
                }
                leg.physical_source_matches(source, read.phone())?;
                Ok(Box::new(receiver(
                    target,
                    read.native().target(),
                    principal,
                )?))
            }
            Self::Native(entry, read) => {
                if !read.current() || !read.permits_subscriptions() {
                    return Err(StreamControlError::Authority);
                }
                source_matches(
                    source,
                    entry,
                    read.source().ok_or(StreamControlError::Authority)?,
                )?;
                Ok(Box::new(receiver(target, read.target(), principal)?))
            }
            Self::Telephone(entry, read) => {
                if !read.permits_native_receive() {
                    return Err(StreamControlError::Authority);
                }
                entry.source_matches(source)?;
                let _owner = entry.established_owner(read.telephone())?;
                Ok(Box::new(receiver(
                    target,
                    read.native().target(),
                    principal,
                )?))
            }
        }
    }
    pub(super) fn participant(
        &self,
        policy: &Policy,
        target: &Connection,
        principal: &AuthenticatedPrincipal,
        source: &MediaParticipantSource,
    ) -> ControlResult<Box<MediaOwner>> {
        match self {
            Self::Initial(leg, read) => {
                if !read.current()
                    || !read.native().permits_subscriptions()
                    || read.native().target().handoff_preparation
                    || read.native().target().publication_suspended
                {
                    return Err(StreamControlError::Authority);
                }
                leg.validate_participant_source(source, read.phone())?;
                let owner = receiver(target, read.native().target(), principal)?;
                if !owner.send_media.contains(&Medium::Audio) {
                    return Err(StreamControlError::Authority);
                }
                Ok(Box::new(owner))
            }
            Self::Native(entry, read) => {
                if !read.current() || !read.permits_subscriptions() {
                    return Err(StreamControlError::Authority);
                }
                Ok(Box::new(policy.participant_receive_owner(
                    target,
                    read.target(),
                    principal,
                    source,
                    entry,
                    read.source().ok_or(StreamControlError::Authority)?,
                )?))
            }
            Self::Telephone(entry, read) => {
                if !read.permits_native_receive() {
                    return Err(StreamControlError::Authority);
                }
                entry.participant_source_matches(source, read.telephone())?;
                // The telephone connection witnesses current logical membership.
                // Future Participant intent uses Session acceptance, independently
                // of this witness endpoint's audio-only physical capability.
                let owner = receiver(target, read.native().target(), principal)?;
                if !owner.send_media.contains(&source.medium) {
                    return Err(StreamControlError::Authority);
                }
                let supported = crate::canonical_native::base_profile().map_err(denied)?;
                if !supported.engine_capabilities().receive.iter().any(|cap| {
                    cap.medium == source.medium
                        && cap.formats.iter().any(|format| {
                            session_capabilities_allow(
                                read.telephone().session(),
                                read.telephone().accepted_capabilities(),
                                source.medium,
                                format,
                            )
                        })
                }) {
                    return Err(StreamControlError::Unsupported);
                }
                Ok(Box::new(owner))
            }
        }
    }
}
impl Policy {
    pub(crate) fn initial_sources(
        &self,
        target: &Connection,
    ) -> connect_protocol::Result<Vec<Arc<crate::initial_telephone::policy::Leg>>> {
        Ok(self
            .initial_targets
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?
            .values()
            .filter_map(Weak::upgrade)
            .filter(|leg| {
                leg.origin().is_ok_and(|o| {
                    o.source().conversation_id == target.origin.conversation_id()
                        && o.source().session_id == target.origin.session_id()
                })
            })
            .collect())
    }
    /// Only a host continuation calls this before entering Core. No registry
    /// mutex survives Store reads or engine callbacks.
    pub(crate) fn telephone_sources(
        &self,
        target: &Connection,
    ) -> connect_protocol::Result<Vec<Arc<crate::telephone_replacement::media::Policy>>> {
        Ok(self
            .telephone_targets
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?
            .values()
            .filter_map(Weak::upgrade)
            .filter(|p| {
                p.established_origin().is_some_and(|o| {
                    let s = o.snapshot();
                    s.conversation_id == target.origin.conversation_id()
                        && s.session_id == target.origin.session_id()
                })
            })
            .collect())
    }
}
