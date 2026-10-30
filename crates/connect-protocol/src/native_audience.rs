//! Captured gate metadata from a route-only commit; never a media IO grant.
use crate::{transitions::id, Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudienceStreamState {
    pub stream_id: String,
    pub stream_generation: u64,
    pub state: String,
}
/// Every actual receiver is present. An empty array means the atomic commit
/// changed no gates on its existing ports; its descriptors can carry forward.
pub type AudienceMetadata = BTreeMap<String, Vec<AudienceStreamState>>;

pub fn validate(metadata: &AudienceMetadata, receivers: &Map<String, Value>) -> Result<()> {
    let invalid = || Error::new("invalid-observation", "Invalid captured audience metadata");
    if metadata.len() > 100
        || metadata.len() != receivers.len()
        || metadata.keys().any(|id| !receivers.contains_key(id))
    {
        return Err(invalid());
    }
    for (connection, streams) in metadata {
        id(connection)?;
        if streams.len() > 32 {
            return Err(invalid());
        }
        let mut unique = BTreeSet::new();
        for stream in streams {
            id(&stream.stream_id)?;
            if !unique.insert(&stream.stream_id)
                || !(1..=9_007_199_254_740_991).contains(&stream.stream_generation)
                || !matches!(stream.state.as_str(), "active" | "paused" | "ended")
            {
                return Err(invalid());
            }
        }
    }
    // Separate from the bounded public transition receipt: these facts are
    // recorded atomically into per-Connection metadata rows, never sent as IO.
    if serde_json::to_vec(metadata).map_err(|_| invalid())?.len() > 1024 * 1024 {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn facts_bind_exact_receivers_unique_streams_and_bounded_generations() {
        let receivers = json!({"receiver":2,"unchanged":2});
        let metadata: AudienceMetadata = serde_json::from_value(json!({
            "receiver":[{"stream_id":"voice","stream_generation":1,"state":"active"}],
            "unchanged":[]
        }))
        .unwrap();
        validate(&metadata, receivers.as_object().unwrap()).unwrap();
        for value in [
            json!({"receiver":[],"foreign":[]}),
            json!({"receiver":[]}),
            json!({"receiver":[{"stream_id":"voice","stream_generation":0,"state":"active"}],"unchanged":[]}),
            json!({"receiver":[{"stream_id":"voice","stream_generation":9007199254740992_u64,"state":"active"}],"unchanged":[]}),
            json!({"receiver":[{"stream_id":"voice","stream_generation":1,"state":"unknown"}],"unchanged":[]}),
            json!({"receiver":[metadata["receiver"][0],metadata["receiver"][0]],"unchanged":[]}),
        ] {
            let value = serde_json::from_value(value).unwrap();
            assert!(validate(&value, receivers.as_object().unwrap()).is_err());
        }
    }
}
