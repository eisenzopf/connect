//! Exact selected resource metadata only. Store and Core final guards remain mandatory.
use super::*;
use rvoip_core::orchestrator::participant_control::{
    ManagedCutResourceRequirements, ManagedReceiveResourceRequirement,
};
use rvoip_core::{
    staged_streams::{PreparedMediaObservation, StreamController},
    stream_lifecycle::ReadyMediaBinding,
};

pub(crate) enum FinalResourceWitness {
    Native {
        activation: Arc<rvoip_uctp::v2::native_negotiation::NativeMediaActivation>,
        observation: Arc<PreparedMediaObservation>,
        bindings: Vec<ReadyMediaBinding>,
    },
    WebRtc(rvoip_webrtc::canonical::RtcMediaResourceWitness),
}
impl FinalResourceWitness {
    pub(crate) fn is_current(&self) -> bool {
        match self {
            Self::Native {
                activation,
                observation,
                ..
            } => activation
                .try_current_resource_observation()
                .is_ok_and(|current| {
                    Arc::ptr_eq(&current, observation) && observation.is_committed()
                }),
            Self::WebRtc(current) => current.is_current(),
        }
    }
    pub(crate) fn belongs_to(&self, controller: &Arc<StreamController>) -> bool {
        match self {
            Self::Native { observation, .. } => observation.belongs_to(controller),
            Self::WebRtc(current) => current.belongs_to(controller),
        }
    }
    pub(crate) fn bindings(&self) -> &[ReadyMediaBinding] {
        match self {
            Self::Native { bindings, .. } => bindings,
            Self::WebRtc(current) => current.bindings(),
        }
    }
}
pub(crate) enum FinalPhysicalWitness {
    // Native metadata is NOT a native physical/IO proof. Core's actual exact
    // Connection/PeerAdmission and Controller/Port/ticket final guards supply it.
    Native {
        activation: Arc<rvoip_uctp::v2::native_negotiation::NativeMediaActivation>,
        observation: Arc<PreparedMediaObservation>,
    },
    WebRtc(rvoip_webrtc::canonical::RtcMediaPhysicalWitness),
}
impl FinalPhysicalWitness {
    fn is_current(&self, controller: &Arc<StreamController>) -> bool {
        match self {
            Self::Native {
                activation,
                observation,
            } => {
                observation.belongs_to(controller)
                    && activation
                        .try_current_resource_observation()
                        .is_ok_and(|actual| {
                            Arc::ptr_eq(&actual, observation) && actual.is_committed()
                        })
            }
            Self::WebRtc(w) => w.belongs_to(controller) && w.is_current(),
        }
    }
}
/// Actual supported branch, resolved before entering any final Core gates.
/// SIP does not get a copied-live fallback: every use borrows the exact route
/// proof whose real guards are still held in the synchronous final turn.
pub(crate) enum FinalResourceOwner {
    Canonical {
        activation: Arc<crate::canonical_replacement::Activation>,
        generation: u64,
    },
    Sip {
        generation: u64,
    },
}
enum Physical {
    Canonical(FinalPhysicalWitness),
    Sip { generation: u64 },
}
struct Resource {
    connection: ConnectionId,
    controller: Arc<StreamController>,
    generation: u64,
    physical: Physical,
    control: bool,
    participant: bool,
    concrete: Vec<ReadyMediaBinding>,
    selected: Vec<FinalResourceWitness>,
}
pub(crate) struct FinalResourceBank {
    resources: Vec<Resource>,
}
fn same_binding(a: &ReadyMediaBinding, b: &ReadyMediaBinding) -> bool {
    a.negotiated == b.negotiated
        && a.transport_id == b.transport_id
        && a.stream_local_id == b.stream_local_id
        && a.generation == b.generation
        && a.connection_generation == b.connection_generation
        && a.source_ready == b.source_ready
        && a.sink_ready == b.sink_ready
}
impl FinalResourceBank {
    pub(crate) fn connections(&self) -> impl Iterator<Item = &ConnectionId> {
        self.resources.iter().map(|r| &r.connection)
    }
    pub(crate) fn capture(
        requirements: ManagedCutResourceRequirements,
        mut resolve: impl FnMut(&ConnectionId) -> ControlResult<FinalResourceOwner>,
    ) -> ControlResult<Self> {
        let mut bank = Self {
            resources: Vec::new(),
        };
        for (connection, controller, bindings) in requirements.controls {
            bank.add(connection, controller, bindings, true, false, &mut resolve)?;
        }
        for source in requirements.sources {
            match source {
                ManagedReceiveResourceRequirement::Concrete {
                    connection,
                    controller,
                    binding,
                } => bank.add(
                    connection,
                    controller,
                    vec![binding],
                    false,
                    false,
                    &mut resolve,
                )?,
                ManagedReceiveResourceRequirement::Participant {
                    connection,
                    controller,
                } => bank.add(
                    connection,
                    controller,
                    Vec::new(),
                    false,
                    true,
                    &mut resolve,
                )?,
            }
        }
        Ok(bank)
    }
    fn add(
        &mut self,
        connection: ConnectionId,
        controller: Arc<StreamController>,
        bindings: Vec<ReadyMediaBinding>,
        control: bool,
        participant: bool,
        resolve: &mut impl FnMut(&ConnectionId) -> ControlResult<FinalResourceOwner>,
    ) -> ControlResult<()> {
        let (physical, generation, selected) = match resolve(&connection)? {
            FinalResourceOwner::Canonical {
                activation,
                generation,
            } => {
                let physical = activation.try_final_physical()?;
                let selected = if bindings.is_empty() {
                    None
                } else {
                    Some(activation.try_final_resources_for(&bindings)?)
                };
                (Physical::Canonical(physical), generation, selected)
            }
            FinalResourceOwner::Sip { generation } => {
                (Physical::Sip { generation }, generation, None)
            }
        };
        if generation == 0
            || bindings
                .iter()
                .any(|b| b.connection_generation != generation)
        {
            return Err(StreamControlError::Authority);
        }
        if let Some(selected) = &selected {
            if !selected.belongs_to(&controller)
                || !selected.is_current()
                || selected.bindings().len() != bindings.len()
                || !bindings
                    .iter()
                    .zip(selected.bindings())
                    .all(|(a, b)| same_binding(a, b))
            {
                return Err(StreamControlError::Authority);
            }
        }
        if let Some(existing) = self
            .resources
            .iter_mut()
            .find(|r| r.connection == connection)
        {
            if !Arc::ptr_eq(&existing.controller, &controller) || existing.generation != generation
            {
                return Err(StreamControlError::Authority);
            }
            existing.control |= control;
            existing.participant |= participant;
            if !control && !participant {
                existing.concrete.extend(bindings);
            }
            if let Some(selected) = selected {
                existing.selected.push(selected);
            }
        } else {
            self.resources.push(Resource {
                connection,
                controller,
                generation,
                physical,
                control,
                participant,
                concrete: if !control && !participant {
                    bindings
                } else {
                    Vec::new()
                },
                selected: selected.into_iter().collect(),
            });
        }
        Ok(())
    }
    fn exact(
        &self,
        connection: &ConnectionId,
        controller: &Arc<StreamController>,
        sip: Option<&rvoip_sip::adapter::SipCurrentMediaRoute<'_>>,
    ) -> Option<&Resource> {
        let r = self
            .resources
            .iter()
            .find(|r| &r.connection == connection && Arc::ptr_eq(&r.controller, controller))?;
        let current = match &r.physical {
            Physical::Canonical(w) => w.is_current(controller),
            Physical::Sip { generation } => sip.is_some_and(|proof| {
                proof.scope().connection() == connection
                    && proof.scope().generation() == *generation
                    && proof.belongs_to(controller)
                    && proof.is_current()
            }),
        };
        (current
            && r.selected
                .iter()
                .all(|w| w.belongs_to(controller) && w.is_current()))
        .then_some(r)
    }
    pub(crate) fn control_current(
        &self,
        connection: &ConnectionId,
        controller: &Arc<StreamController>,
        sip: Option<&rvoip_sip::adapter::SipCurrentMediaRoute<'_>>,
    ) -> bool {
        self.exact(connection, controller, sip)
            .is_some_and(|r| r.control)
    }
    pub(crate) fn source_current(
        &self,
        controller: &Arc<StreamController>,
        source: &MediaSource,
        sip: Option<&rvoip_sip::adapter::SipCurrentMediaRoute<'_>>,
    ) -> bool {
        self.exact(&source.connection, controller, sip)
            .is_some_and(|r| {
                r.generation == source.connection_generation
                    && r.concrete.iter().any(|b| {
                        b.negotiated.requested.logical_id == source.logical_id
                            && b.transport_id == source.transport_id
                            && b.generation == source.stream_generation
                            && b.connection_generation == source.connection_generation
                    })
            })
    }
    pub(crate) fn participant_current(
        &self,
        controller: &Arc<StreamController>,
        source: &rvoip_core::staged_streams::MediaParticipantSource,
        sip: Option<&rvoip_sip::adapter::SipCurrentMediaRoute<'_>>,
    ) -> bool {
        self.exact(&source.connection, controller, sip)
            .is_some_and(|r| r.participant)
    }
}

/// Declare before PostCustody; install before final Writer acquisition. The
/// inner frame borrows these exact owners, including all Connection Arcs whose
/// last Drop could retire transport resources. No authority is retained here.
pub(crate) struct FinalTurnResources {
    pub(crate) bank: FinalResourceBank,
    pub(crate) entries: std::collections::BTreeMap<ConnectionId, Arc<super::Connection>>,
    // Last field drops after actual bank and Connection owners. Its weak
    // observer proves this container's real field Drop finished, not liveness.
    #[cfg(feature = "test-support")]
    last_resource: Arc<()>,
}
impl FinalTurnResources {
    pub(crate) fn new(
        bank: FinalResourceBank,
        entries: std::collections::BTreeMap<ConnectionId, Arc<super::Connection>>,
    ) -> Self {
        Self {
            bank,
            entries,
            #[cfg(feature = "test-support")]
            last_resource: Arc::new(()),
        }
    }
    #[cfg(feature = "test-support")]
    pub(crate) fn observe_for_test(
        &self,
        probe: &Option<Arc<super::preparation_custody_fixture::Probe>>,
    ) {
        if let Some(probe) = probe {
            probe.record_turn_resource(Arc::downgrade(&self.last_resource));
        }
    }
}
