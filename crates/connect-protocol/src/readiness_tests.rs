#![allow(clippy::unwrap_used)]
use super::*;
#[test]
fn legacy_observation_retains_wire_shape_and_unconfigured_is_explicit() {
    let old = serde_json::json!({"component":"mcp","state":"ready","reason":"listener-bound","observed_at":1});
    let mut value: ComponentReadiness = serde_json::from_value(old.clone()).unwrap();
    assert!(value.configured);
    assert_eq!(serde_json::to_value(&value).unwrap(), old);
    value.configured = false;
    let serialized = serde_json::to_value(value).unwrap();
    assert_eq!(serialized["configured"], false);
}
