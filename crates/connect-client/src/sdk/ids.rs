//! Validated public IDs and distinct optimistic revision families.
use super::{Error, ErrorKind, Result};
use serde::{Deserialize, Serialize};
macro_rules! ids {
    ($($name:ident),+ $(,)?) => {$ (
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);
        impl $name { pub fn as_str(&self) -> &str { &self.0 } }
        impl TryFrom<String> for $name {
            type Error = Error;
            fn try_from(value: String) -> Result<Self> {
                rvoip_uctp::v2::Identifier::try_from(value.clone())
                    .map_err(|_| Error::local(ErrorKind::Validation))?;
                Ok(Self(value))
            }
        }
        impl std::str::FromStr for $name {
            type Err = Error;
            fn from_str(value: &str) -> Result<Self> { Self::try_from(value.to_owned()) }
        }
        impl From<$name> for String { fn from(value: $name) -> Self { value.0 } }
    )+};
}
ids!(
    TenantId,
    ConversationId,
    ParticipantId,
    SessionId,
    ConnectionId,
    StreamId,
    OperationId,
    MessageId,
    IdentityId,
    EndpointId
);
macro_rules! revisions {
    ($($name:ident),+ $(,)?) => {$ (
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(try_from = "u64", into = "u64")]
        pub struct $name(u64);
        impl $name { pub fn get(self) -> u64 { self.0 } }
        impl TryFrom<u64> for $name {
            type Error = Error;
            fn try_from(value: u64) -> Result<Self> {
                if value > 9_007_199_254_740_991 { return Err(Error::local(ErrorKind::Validation)); }
                Ok(Self(value))
            }
        }
        impl From<$name> for u64 { fn from(value: $name) -> Self { value.0 } }
    )+};
}
revisions!(
    ConversationRevision,
    SessionRevision,
    TopologyRevision,
    StreamRevision
);
#[derive(Clone, Debug)]
pub struct NonEmpty<T> {
    pub first: T,
    pub rest: Vec<T>,
}
impl<T> NonEmpty<T> {
    pub fn into_vec(self) -> Vec<T> {
        std::iter::once(self.first).chain(self.rest).collect()
    }
}
