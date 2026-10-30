//! Recorded control versions are observations, never physical media grants.
pub(crate) fn stream_revision(
    state: String,
    readiness: Option<String>,
    runtime: Option<String>,
) -> rusqlite::Result<Option<u64>> {
    if !matches!(state.as_str(), "ready" | "active") || runtime.is_none() {
        return Ok(None);
    }
    let Some(raw) = readiness else {
        return Ok(None);
    };
    let value: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(value["stream_revision"]
        .as_u64()
        .filter(|v| (1..=9_007_199_254_740_991).contains(v)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn control_revision_requires_recorded_ready_runtime_and_bounded_integer() {
        for state in ["ready", "active"] {
            for revision in [1, 5, 9_007_199_254_740_991_u64] {
                assert_eq!(
                    stream_revision(
                        state.into(),
                        Some(serde_json::json!({"stream_revision": revision}).to_string()),
                        Some("runtime".into()),
                    )
                    .unwrap(),
                    Some(revision)
                );
            }
        }
        for state in ["preparing", "retiring", "retired", "failed"] {
            assert_eq!(
                stream_revision(
                    state.into(),
                    Some(r#"{"stream_revision":5}"#.into()),
                    Some("runtime".into())
                )
                .unwrap(),
                None
            );
        }
        assert_eq!(
            stream_revision(
                "ready".into(),
                Some(r#"{"stream_revision":5}"#.into()),
                None
            )
            .unwrap(),
            None
        );
        for raw in [
            "null",
            "{}",
            r#"{"stream_revision":0}"#,
            r#"{"stream_revision":-1}"#,
            r#"{"stream_revision":1.5}"#,
            r#"{"stream_revision":"5"}"#,
            r#"{"stream_revision":9007199254740992}"#,
        ] {
            assert_eq!(
                stream_revision("ready".into(), Some(raw.into()), Some("runtime".into())).unwrap(),
                None
            );
        }
    }
}
