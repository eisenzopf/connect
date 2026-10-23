//! Operator-selected local retention. No capture or remote deletion permission.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Lifetime {
    UntilDeleted,
    After { seconds: u32 },
}
impl Lifetime {
    pub fn seconds(&self) -> Option<u32> {
        match self {
            Self::UntilDeleted => None,
            Self::After { seconds } => Some(*seconds),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub name: String,
    pub metadata: Lifetime,
    pub transcript: Lifetime,
    pub recorded_media: Lifetime,
    pub provider_reference: Lifetime,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            name: "connect-local-until-deleted-v1".into(),
            metadata: Lifetime::UntilDeleted,
            transcript: Lifetime::UntilDeleted,
            recorded_media: Lifetime::UntilDeleted,
            provider_reference: Lifetime::UntilDeleted,
        }
    }
}
impl Policy {
    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty()
            || self.name.len() > 128
            || !self
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._~-".contains(&b))
            || [
                &self.metadata,
                &self.transcript,
                &self.recorded_media,
                &self.provider_reference,
            ]
            .into_iter()
            .any(|v| v.seconds().is_some_and(|s| s == 0 || s > 315_576_000))
        {
            return Err(Error::new(
                "invalid-request",
                "Invalid named local retention policy",
            ));
        }
        Ok(())
    }
}
