//! Freshness and directions remain separate for every setup prerequisite.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Inbound,
    Outbound,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObservationState {
    Ready,
    Missing,
    Rejected,
    Unknown,
    Stale,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Prerequisite {
    Authentication,
    Resources,
    Callback,
    SipRegistration,
    Rtp,
    Media,
    SmsEligibility,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub prerequisite: Prerequisite,
    pub direction: Direction,
    pub state: ObservationState,
    pub observed_at: i64,
    pub valid_until: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadinessAction {
    pub prerequisite: Prerequisite,
    pub direction: Direction,
    pub state: ObservationState,
    pub next_action: String,
}
/// No readiness result claims human receipt or selects an alternative route.
pub fn readiness(observations: &[Observation], now: i64) -> anyhow::Result<Vec<ReadinessAction>> {
    anyhow::ensure!(observations.len() <= 64, "Too many readiness observations");
    let mut keys = std::collections::HashSet::new();
    for item in observations {
        anyhow::ensure!(
            keys.insert(format!("{:?}:{:?}", item.prerequisite, item.direction))
                && item.observed_at <= now
                && item.observed_at >= 0
                && item.valid_until >= item.observed_at,
            "Invalid or ambiguous readiness observation"
        );
    }
    let mut result = Vec::new();
    // Absent observations are unknown rather than implied by another component.
    for prerequisite in [
        Prerequisite::Authentication,
        Prerequisite::Resources,
        Prerequisite::Callback,
        Prerequisite::SipRegistration,
        Prerequisite::Rtp,
        Prerequisite::Media,
        Prerequisite::SmsEligibility,
    ] {
        for direction in [Direction::Inbound, Direction::Outbound] {
            let observation = observations
                .iter()
                .find(|o| o.prerequisite == prerequisite && o.direction == direction);
            let state = observation
                .map(|o| {
                    if o.valid_until <= now {
                        ObservationState::Stale
                    } else {
                        o.state.clone()
                    }
                })
                .unwrap_or(ObservationState::Unknown);
            let action = if state == ObservationState::Ready {
                "No action; readiness is an observation, not delivery proof"
            } else {
                match prerequisite {Prerequisite::Authentication=>"Check selected server secret and provider authentication",Prerequisite::Resources=>"Refresh resource catalog and reconcile selected IDs",Prerequisite::Callback=>"Verify the configured signed callback independently",Prerequisite::SipRegistration=>"Check the selected trunk registration and lease",Prerequisite::Rtp=>"Verify this direction's explicit RTP reachability",Prerequisite::Media=>"Negotiate supported media on this route",Prerequisite::SmsEligibility=>"Refresh the configured enrollment authority; native pairing is not SMS consent"}
            };
            result.push(ReadinessAction {
                prerequisite: prerequisite.clone(),
                direction,
                state,
                next_action: action.into(),
            });
        }
    }
    Ok(result)
}
