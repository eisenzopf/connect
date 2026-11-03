//! Explicit transfer of existing video selections during one device replacement.
use crate::{transitions::id, Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplacementVideoStream {
    pub previous: String,
    pub replacement: String,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplacementVideo {
    pub publications: Vec<ReplacementVideoStream>,
    pub receivers: Vec<ReplacementVideoStream>,
}
impl ReplacementVideo {
    pub fn validate(&self) -> Result<()> {
        let invalid = || Error::new("invalid-request", "Invalid replacement video mapping");
        if !(1..=32).contains(&(self.publications.len() + self.receivers.len())) {
            return Err(invalid());
        }
        let mut targets = BTreeSet::new();
        for group in [&self.publications, &self.receivers] {
            let mut previous = BTreeSet::new();
            for mapping in group {
                id(&mapping.previous)?;
                id(&mapping.replacement)?;
                if !previous.insert(&mapping.previous) || !targets.insert(&mapping.replacement) {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_video_mapping_is_bounded_unambiguous_and_closed() {
        let value = serde_json::json!({"publications":[{"previous":"camera","replacement":"new-camera"}],"receivers":[{"previous":"camera","replacement":"incoming-camera"}]});
        let good: ReplacementVideo = serde_json::from_value(value.clone()).unwrap();
        good.validate().unwrap();
        assert_eq!(serde_json::to_value(&good).unwrap(), value);
        let mut bad = good.clone();
        bad.receivers[0].replacement = "new-camera".into();
        assert!(bad.validate().is_err());
        let mut bad = good.clone();
        bad.publications.push(bad.publications[0].clone());
        assert!(bad.validate().is_err());
        let mut bad = good.clone();
        bad.receivers[0].previous.clear();
        assert!(bad.validate().is_err());
        assert!(ReplacementVideo {
            publications: vec![],
            receivers: vec![]
        }
        .validate()
        .is_err());
        let mut bad = value.clone();
        bad["permission"] = true.into();
        assert!(serde_json::from_value::<ReplacementVideo>(bad).is_err());
        let mut bad = value.clone();
        bad["publications"][0]["generation"] = 1.into();
        assert!(serde_json::from_value::<ReplacementVideo>(bad).is_err());
        let entries = (0..33)
            .map(|n| ReplacementVideoStream {
                previous: format!("old-{n}"),
                replacement: format!("new-{n}"),
            })
            .collect();
        assert!(ReplacementVideo {
            publications: entries,
            receivers: vec![]
        }
        .validate()
        .is_err());
    }
}
