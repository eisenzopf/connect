//! Signed selection metadata; this does not authorize or enable media feedback.
use super::*;
use serde::{Deserialize, Serialize};

pub(super) const PROFILE: &str = "rtcp-pli-epoch/1";
const MAX_SAFE: u64 = 9_007_199_254_740_991;

/// The allocation that owns native video history, retained across unchanged
/// resource reoffers. Only signed, confirmed native parameters establish this.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeVideoFeedback {
    profile: String,
    pub original_operation: OperationId,
    pub original_offer: String,
    pub stream_generation: u64,
    pub connection_generation: u64,
}

pub(super) struct Selection<'a> {
    pub local: &'a Value,
    pub remote: &'a Value,
    pub operation: &'a str,
    pub offer: &'a str,
    pub generation: u64,
    pub connection_generation: u64,
    pub previous: Option<&'a NativeVideoFeedback>,
}

pub(super) fn selection(
    stream: &Value,
    context: Selection<'_>,
) -> Result<Option<NativeVideoFeedback>> {
    let bad = || Error::local(ErrorKind::Protocol);
    let enabled = context.local["video_feedback"] == PROFILE
        && context.remote["video_feedback"] == PROFILE
        && stream["kind"] == "video";
    let Some(value) = stream.get("video_feedback") else {
        return if enabled || context.previous.is_some() {
            Err(bad())
        } else {
            Ok(None)
        };
    };
    if !enabled {
        return Err(bad());
    }
    let selected: NativeVideoFeedback = serde_json::from_value(value.clone()).map_err(|_| bad())?;
    if selected.profile != PROFILE
        || crate::canonical::Identifier::try_from(selected.original_offer.clone()).is_err()
        || !(1..=MAX_SAFE).contains(&selected.stream_generation)
        || !(1..=MAX_SAFE).contains(&selected.connection_generation)
        || selected.stream_generation != context.generation
        || selected.connection_generation != context.connection_generation
    {
        return Err(bad());
    }
    if let Some(previous) = context.previous {
        if &selected != previous {
            return Err(bad());
        }
    } else if selected.original_operation.as_str() != context.operation
        || selected.original_offer != context.offer
    {
        return Err(bad());
    }
    Ok(Some(selected))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn selected_video_history_requires_bilateral_profile_and_exact_allocation_provenance() {
        let caps = json!({"video_feedback":PROFILE});
        let base = json!({"kind":"video","video_feedback":{
            "profile":PROFILE,"original_operation":"operation","original_offer":"offer",
            "stream_generation":1,"connection_generation":1}});
        let read = |value: &Value, previous| {
            selection(
                value,
                Selection {
                    local: &caps,
                    remote: &caps,
                    operation: "operation",
                    offer: "offer",
                    generation: 1,
                    connection_generation: 1,
                    previous,
                },
            )
        };
        let selected = read(&base, None).unwrap().unwrap();
        for (field, bad_value) in [
            ("profile", json!("other/1")),
            ("original_operation", json!("other")),
            ("original_offer", json!("other")),
            ("original_offer", json!("invalid offer")),
            ("stream_generation", json!(0)),
            ("stream_generation", json!(2)),
            ("stream_generation", json!(9_007_199_254_740_992_u64)),
            ("connection_generation", json!(2)),
            ("extra", json!(true)),
        ] {
            let mut value = base.clone();
            value["video_feedback"][field] = bad_value;
            assert!(read(&value, None).is_err(), "{field}");
        }
        for local in [json!({}), caps.clone()] {
            for remote in [json!({}), caps.clone()] {
                let result = selection(
                    &base,
                    Selection {
                        local: &local,
                        remote: &remote,
                        operation: "operation",
                        offer: "offer",
                        generation: 1,
                        connection_generation: 1,
                        previous: None,
                    },
                );
                assert_eq!(result.is_ok(), local == caps && remote == caps);
            }
        }
        for kind in ["audio", "screen", "data"] {
            let mut value = base.clone();
            value["kind"] = json!(kind);
            assert!(read(&value, None).is_err());
        }
        assert!(read(&json!({"kind":"video"}), None).is_err());
        assert!(read(&json!({"kind":"video","video_feedback":null}), None).is_err());
        // An unchanged resource still belongs to its first operation even
        // after a new control operation and parameter offer are confirmed.
        let same = selection(
            &base,
            Selection {
                local: &caps,
                remote: &caps,
                operation: "new-operation",
                offer: "new-offer",
                generation: 1,
                connection_generation: 1,
                previous: Some(&selected),
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(same, selected);
        let mut replaced = base.clone();
        replaced["video_feedback"]["original_operation"] = json!("new-operation");
        replaced["video_feedback"]["original_offer"] = json!("new-offer");
        replaced["video_feedback"]["stream_generation"] = json!(2);
        assert!(read(&replaced, Some(&selected)).is_err());
        assert!(selection(
            &replaced,
            Selection {
                local: &caps,
                remote: &caps,
                operation: "new-operation",
                offer: "new-offer",
                generation: 2,
                connection_generation: 1,
                previous: None,
            }
        )
        .is_ok());
    }
}
