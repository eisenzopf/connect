//! Participant-based endpoint selection. Discovery never grants admission.
use crate::{provisioning::EndpointTransport, Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RouteMedium {
    Audio,
    Video,
    Screen,
    Data,
    Messages,
    Files,
}
impl RouteMedium {
    pub fn capability(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Video => "video",
            Self::Screen => "screen",
            Self::Data => "data",
            Self::Messages => "messaging",
            Self::Files => "files",
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoutePurpose {
    Session,
    Message,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RouteDirection {
    Outgoing,
    Incoming,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailoverPolicy {
    None,
    OtherAvailableDevice,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReachabilityPolicy {
    #[default]
    RequireFresh,
    /// Permit an attempt without claiming endpoint reachability. Known offline
    /// or busy observations and all other prerequisites remain enforced.
    AllowUnknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ParticipantRouteRequest {
    pub conversation_id: String,
    pub participant_id: String,
    pub purpose: RoutePurpose,
    pub direction: RouteDirection,
    pub media: Vec<RouteMedium>,
    /// Only these transports may be considered; no implicit channel fallback.
    pub transports: Vec<EndpointTransport>,
    pub preferred_device_id: Option<String>,
    pub failover: FailoverPolicy,
    #[serde(default)]
    pub reachability: ReachabilityPolicy,
}
impl ParticipantRouteRequest {
    pub fn normalize(&mut self) -> Result<()> {
        for id in [&self.conversation_id, &self.participant_id]
            .into_iter()
            .chain(self.preferred_device_id.iter())
        {
            if id.is_empty()
                || id.len() > 128
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._~-".contains(&b))
            {
                return Err(invalid());
            }
        }
        if self.media.is_empty()
            || self.media.len() > 6
            || self.transports.is_empty()
            || self.transports.len() > 8
        {
            return Err(invalid());
        }
        if self.media.iter().any(|m| match self.purpose {
            RoutePurpose::Session => matches!(m, RouteMedium::Messages | RouteMedium::Files),
            RoutePurpose::Message => !matches!(m, RouteMedium::Messages | RouteMedium::Files),
        }) {
            return Err(invalid());
        }
        self.media.sort();
        self.transports.sort_by_key(|t| t.as_str());
        if self.media.windows(2).any(|p| p[0] == p[1])
            || self.transports.windows(2).any(|p| p[0] == p[1])
        {
            return Err(invalid());
        }
        Ok(())
    }
}
fn invalid() -> Error {
    Error::new("invalid-request", "Invalid participant routing selection")
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EndpointReachability {
    Reachable,
    Offline,
    Busy,
    Unknown,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RouteReason {
    Selected,
    AttemptAllowed,
    Ambiguous,
    NoEndpoint,
    Unverified,
    UnsupportedCapability,
    Unconfigured,
    PermissionRequired,
    ConsentRequired,
    Offline,
    Busy,
    Unknown,
    PrerequisitesMissing,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EndpointCandidate {
    pub endpoint_id: String,
    pub device_id: String,
    pub transport: EndpointTransport,
    pub endpoint_revision: u64,
    pub reason: RouteReason,
    pub reachability: EndpointReachability,
    pub observation_sequence: Option<u64>,
    pub valid_until: Option<i64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ParticipantRoute {
    pub conversation_id: String,
    pub conversation_revision: u64,
    pub participant_id: String,
    pub membership_epoch: u64,
    pub reason: RouteReason,
    pub selected: Option<EndpointCandidate>,
    pub candidates: Vec<EndpointCandidate>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    fn request() -> ParticipantRouteRequest {
        ParticipantRouteRequest {
            conversation_id: "conv_one".into(),
            participant_id: "part_one".into(),
            purpose: RoutePurpose::Session,
            direction: RouteDirection::Outgoing,
            media: vec![RouteMedium::Video, RouteMedium::Audio],
            transports: vec![EndpointTransport::Webrtc, EndpointTransport::Quic],
            preferred_device_id: None,
            failover: FailoverPolicy::None,
            reachability: ReachabilityPolicy::RequireFresh,
        }
    }
    #[test]
    fn routing_normalization_bounds_and_explicit_unknown_policy_are_not_authority_claims() {
        let mut base = request();
        base.normalize().unwrap();
        assert_eq!(base.media, vec![RouteMedium::Audio, RouteMedium::Video]);
        for case in 0..5 {
            let mut value = request();
            match case {
                0 => value.media.clear(),
                1 => value.media.push(RouteMedium::Audio),
                2 => value.transports.push(EndpointTransport::Quic),
                3 => value.media = vec![RouteMedium::Messages],
                _ => value.preferred_device_id = Some("private/other-device".into()),
            }
            assert!(value.normalize().is_err());
        }
        let mut value = serde_json::to_value(request()).unwrap();
        value.as_object_mut().unwrap().remove("reachability");
        assert_eq!(
            serde_json::from_value::<ParticipantRouteRequest>(value.clone())
                .unwrap()
                .reachability,
            ReachabilityPolicy::RequireFresh
        );
        value["reachability"] = serde_json::json!("reachable");
        assert!(serde_json::from_value::<ParticipantRouteRequest>(value).is_err());
        let mut value = serde_json::to_value(request()).unwrap();
        value["approved"] = true.into();
        assert!(serde_json::from_value::<ParticipantRouteRequest>(value).is_err());
    }
}
