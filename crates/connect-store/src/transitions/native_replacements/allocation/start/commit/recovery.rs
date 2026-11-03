//! Successor settlement of a durably captured, never-activated native move.
//! Historical projection facts are not a media origin, credential or IO grant.
use super::*;

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Target {
    pub connection: connect_protocol::connection_control::PreparedConnection,
    pub runtime_connection_id: String,
    pub device_id: String,
    pub identity_id: String,
}

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProjectionFacts {
    pub target: Target,
    pub resources: String,
    pub source_expires_at: i64,
}
impl ProjectionFacts {
    pub(super) fn from_attempt(a: &NativeReplacementCommitAttempt) -> Self {
        Self {
            target: Target {
                connection: a.target.connection.clone(),
                runtime_connection_id: a.target.runtime_connection_id.clone(),
                device_id: a.target.device_id.clone(),
                identity_id: a.target.session.actor.identity_id.clone(),
            },
            resources: a.resources.clone(),
            source_expires_at: a.source_expires_at,
        }
    }
}

pub(super) struct Projection {
    pub record: String,
    pub runtime: String,
    pub reservation: String,
    pub hash: String,
    pub attempt: String,
    pub effect: String,
    pub expected: String,
    pub observation: String,
    pub descriptor: String,
    pub facts: ProjectionFacts,
}
impl Projection {
    pub(super) fn from_capture(capture: &NativeReplacementCommitted) -> Result<Self> {
        let a = &capture.attempt;
        let start = &a.origin.started.attempt;
        let p = &start.allocated.attempt.prepared;
        let facts = Self::facts(&capture.observation)?;
        if facts != ProjectionFacts::from_attempt(a) {
            return Err(Error::forbidden());
        }
        Ok(Self {
            record: p.record.clone(),
            runtime: p.runtime.clone(),
            reservation: p.reservation.clone(),
            hash: p.hash.clone(),
            attempt: a.attempt.clone(),
            effect: a.effect.clone(),
            expected: a.expected.clone(),
            observation: capture.observation.clone(),
            descriptor: start.target.clone(),
            facts,
        })
    }

    fn facts(observation: &str) -> Result<ProjectionFacts> {
        let value: serde_json::Value =
            serde_json::from_str(observation).map_err(|_| Error::storage())?;
        serde_json::from_value(value["projection"].clone()).map_err(|_| {
            Error::new(
                "reconciliation-required",
                "Captured replacement lacks exact historical projection facts",
            )
        })
    }
}

struct Absent<'a> {
    source: &'a dyn crate::NativeOfferRecoveryObservation,
    target: &'a Target,
}
impl crate::NativeOfferRuntimeObservation for Absent<'_> {
    fn observe(&self) -> Result<crate::NativeOfferObservation> {
        self.source.observe_retired(
            &self.target.runtime_connection_id,
            self.target.connection.generation,
        )
    }
}

impl Store {
    /// Before successor intake, settle only original confirmed moves whose
    /// ordinary lifetime was never published. Exclusive installation ownership
    /// and exact local absence are both required. Uncaptured attempts remain
    /// Unknown; this method never executes a move or constructs a media origin.
    pub fn recover_native_replacement_activations(
        &self,
        source: &dyn crate::NativeOfferRecoveryObservation,
    ) -> Result<usize> {
        let owner = self.require_runtime_owner()?;
        let saved = {
            let c = self.connection()?;
            let mut query = c.prepare("SELECT n.operation_record_id,n.runtime_epoch,n.reservation_id,n.plan_digest,s.attempt_id,s.effect_id,e.observation_json,s.observation_json,se.observation_json FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='commit' JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=n.operation_record_id AND e.provider='native-replacement-commit' AND e.state='confirmed' AND e.attempt=1 AND e.submission_generation=1 JOIN native_replacement_steps start ON start.operation_record_id=n.operation_record_id AND start.phase='start' JOIN operation_effects se ON se.id=start.effect_id AND se.operation_record_id=n.operation_record_id AND se.provider='native-replacement-start' AND se.state='confirmed' WHERE n.runtime_epoch!=?1 AND s.started=1 AND s.observation_json IS NOT NULL AND start.started=1 AND start.observation_json IS NOT NULL AND o.state IN ('preparing','unknown') AND NOT EXISTS(SELECT 1 FROM native_replacement_lifetimes l WHERE l.operation_record_id=n.operation_record_id) ORDER BY n.operation_record_id LIMIT ?2").map_err(|_| Error::storage())?;
            let rows = query
                .query_map(
                    params![self.runtime_epoch(), self.limits().connections + 1],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                            r.get::<_, String>(4)?,
                            r.get::<_, String>(5)?,
                            r.get::<_, String>(6)?,
                            r.get::<_, String>(7)?,
                            r.get::<_, String>(8)?,
                        ))
                    },
                )
                .map_err(|_| Error::storage())?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|_| Error::storage())?
        };
        if saved.len() > self.limits().connections || !owner.current() {
            return Err(Error::new(
                "authority-unavailable",
                "Replacement recovery exceeds capacity or ownership ended",
            ));
        }
        let count = saved.len();
        for (
            record,
            runtime,
            reservation,
            hash,
            attempt,
            effect,
            expected,
            observation,
            descriptor,
        ) in saved
        {
            let facts = Projection::facts(&observation)?;
            let p = Projection {
                record,
                runtime,
                reservation,
                hash,
                attempt,
                effect,
                expected,
                observation,
                descriptor,
                facts,
            };
            let absent = Absent {
                source,
                target: &p.facts.target,
            };
            let result = self.project_replacement_metadata(
                &p,
                None,
                ActivationObservation::Stopped(&absent),
            )?;
            if result.is_some() || !owner.current() {
                return Err(Error::forbidden());
            }
        }
        Ok(count)
    }
}
