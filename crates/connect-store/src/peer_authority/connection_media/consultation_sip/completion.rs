//! Return completion observes the original SIP lifetime without lending IO.
use super::*;
use crate::identity::Actor;

impl Store {
    pub(crate) fn sip_completion_scopes(
        binding: &serde_json::Value,
    ) -> Result<Vec<AuthorityScope>> {
        let _: SipSnapshot =
            serde_json::from_value(binding.clone()).map_err(|_| Error::storage())?;
        ["identity_id", "route_owner_id"]
            .into_iter()
            .map(|key| {
                AuthorityScope::identity(
                    binding[key].as_str().ok_or_else(Error::storage)?,
                    AuthorityMode::Read,
                )
            })
            .collect()
    }

    /// Physical presence is separate from current permission. Revocation must
    /// not turn a still-observed call into a fabricated retirement.
    pub(crate) fn recorded_sip_live(
        c: &Connection,
        binding: &serde_json::Value,
        runtime: &str,
    ) -> Result<bool> {
        let snapshot: SipSnapshot =
            serde_json::from_value(binding.clone()).map_err(|_| Error::storage())?;
        let text = |key: &str| binding[key].as_str().ok_or_else(Error::storage);
        let generation = binding["generation"].as_u64().ok_or_else(Error::storage)?;
        let epoch = binding["membership_epoch"]
            .as_u64()
            .ok_or_else(Error::storage)?;
        let exact: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM connections WHERE id=?1 AND participant_id=?2 AND membership_epoch=?3 AND identity_id=?4 AND endpoint_id=?5 AND observed_runtime_id=?6 AND generation=?7 AND state IN ('ready','active') AND readiness_json IS NOT NULL)",params![snapshot.connection_id(),text("participant_id")?,epoch,text("identity_id")?,text("endpoint_id")?,runtime,generation],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !exact {
            return Ok(false);
        }
        match snapshot {
            SipSnapshot::InitialSip { .. } | SipSnapshot::PrivateInitialSip { .. } => {
                if generation != 1 || !Self::initial_sip_snapshot_kind_matches(c, binding)? {
                    return Ok(false);
                }
                c.query_row("SELECT EXISTS(SELECT 1 FROM initial_telephone_operations n JOIN operations o ON o.record_id=n.operation_record_id JOIN initial_telephone_answers a ON a.operation_record_id=n.operation_record_id JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id AND e.state='confirmed' WHERE n.operation_record_id=?1 AND o.connection_id=?2 AND n.attempt_runtime=?3 AND n.runtime_connection_id=?4 AND n.sip_call_id=?5 AND n.signaling_started=1 AND a.accepted=1 AND a.connected_at IS NOT NULL AND a.retired_at IS NULL AND json_extract(a.observation_json,'$.runtime_connection_id')=?4 AND json_extract(a.observation_json,'$.call_id')=?5 AND json_extract(a.connected_json,'$.connection_id')=?2 AND json_extract(a.connected_json,'$.generation')=1 AND json_extract(a.connected_json,'$.participant_id')=?6 AND json_extract(a.connected_json,'$.membership_epoch')=?7 AND json_extract(a.connected_json,'$.endpoint_id')=?8)",params![text("record_id")?,text("connection_id")?,runtime,text("runtime_connection_id")?,text("sip_call_id")?,text("participant_id")?,epoch,text("endpoint_id")?],|r|r.get(0)).map_err(|_|Error::storage())
            }
            SipSnapshot::EstablishedSip { .. } => {
                if generation < 2 {
                    return Ok(false);
                }
                c.query_row("SELECT EXISTS(SELECT 1 FROM telephone_connection_lifetimes l JOIN telephone_replacement_commits t ON t.operation_record_id=l.operation_record_id AND t.runtime_epoch=l.runtime_epoch JOIN native_replacements n ON n.operation_record_id=l.operation_record_id AND n.candidate_connection_id=l.connection_id AND n.generation=l.generation JOIN operations o ON o.record_id=l.operation_record_id AND o.state='completed' JOIN operation_effects e ON e.id=t.effect_id AND e.operation_record_id=o.record_id AND e.state='confirmed' WHERE l.operation_record_id=?1 AND l.connection_id=?2 AND l.runtime_epoch=?3 AND l.generation=?4 AND l.retired=0 AND n.candidate_runtime_id=?5 AND json_extract(t.descriptor_json,'$.media.call_id')=?6)",params![text("record_id")?,text("connection_id")?,runtime,generation,text("runtime_connection_id")?,text("sip_call_id")?],|r|r.get(0)).map_err(|_|Error::storage())
            }
        }
    }

    /// Current receive permission and recipient ceilings for completion status,
    /// under the caller's complete interval. This never returns a media reader.
    pub(crate) fn sip_completion_actors(
        &self,
        c: &Connection,
        binding: &serde_json::Value,
        conversation: &str,
        session: &str,
        held: &AuthorityGuardSet,
    ) -> Result<Option<Vec<Actor>>> {
        let (scopes, expiry) =
            match self.sip_audience_snapshot_facts(c, binding, conversation, session) {
                Ok(facts) => facts,
                Err(error) if matches!(error.code.as_str(), "forbidden" | "unauthorized") => {
                    return Ok(None)
                }
                Err(error) => return Err(error),
            };
        if crate::now() >= expiry || !self.authority_covers(held, &scopes) {
            return Ok(None);
        }
        let text = |key: &str| binding[key].as_str().ok_or_else(Error::storage);
        let result: Result<Option<Vec<Actor>>> = (|| {
            let route = crate::identity::actor_from_connection(c, text("route_credential_id")?)?;
            let mut actors = vec![route];
            if matches!(
                binding["source_kind"].as_str(),
                Some("initial-sip" | "private-initial-sip")
            ) {
                actors.push(crate::identity::actor_from_connection(
                    c,
                    text("endpoint_credential_id")?,
                )?);
            }
            if !actors
                .last()
                .ok_or_else(Error::storage)?
                .permits("stream.subscribe")
            {
                return Ok(None);
            }
            Ok(Some(actors))
        })();
        match result {
            Err(error) if matches!(error.code.as_str(), "forbidden" | "unauthorized") => Ok(None),
            result => result,
        }
    }
}
