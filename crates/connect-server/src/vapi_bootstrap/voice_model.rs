//! Voice MCP discovery needs a per-call model override, not a callable wrapper.
use anyhow::{ensure, Result};
use serde_json::{json, Value};

pub(super) fn overrides(mut model: Value, nonce: &str, url: &str, secret: &str) -> Result<Value> {
    let object = model
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("Invalid saved assistant model"))?;
    ensure!(
        object
            .get("provider")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty()),
        "Saved assistant model has no provider"
    );
    let tools = object.entry("tools").or_insert_with(|| json!([]));
    let tools = tools
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("Invalid saved assistant tools"))?;
    let mut shared = super::call::overrides(nonce, url, secret);
    tools.push(shared["tools:append"][0].clone());
    shared
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("Invalid voice tool configuration"))?
        .remove("tools:append");
    shared["model"] = model;
    Ok(shared)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn scoped_mcp_discovery_preserves_saved_model_and_other_tools() {
        let original = json!({"provider":"openai","model":"saved-model","temperature":0.3,
            "messages":[{"role":"system","content":"Saved prompt"}],"toolIds":["saved-tool"],
            "tools":[{"type":"function","function":{"name":"existing_tool"}}]});
        let result = overrides(
            original.clone(),
            "nonce",
            "https://example.com/mcp",
            "private-secret",
        )
        .unwrap();
        assert!(result.get("tools:append").is_none());
        let mut restored = result["model"].clone();
        let mcp = restored["tools"].as_array_mut().unwrap().pop().unwrap();
        assert_eq!(restored, original);
        assert_eq!(mcp["type"], "mcp");
        assert_eq!(
            mcp["server"]["headers"]["Authorization"],
            "Bearer private-secret"
        );
        assert_eq!(result["variableValues"]["connect_invocation_id"], "nonce");
    }

    #[test]
    fn absent_tools_are_supported_but_malformed_saved_models_are_rejected() {
        let result =
            overrides(json!({"provider":"custom-llm"}), "n", "https://x/mcp", "s").unwrap();
        assert_eq!(result["model"]["tools"].as_array().unwrap().len(), 1);
        for model in [
            Value::Null,
            json!([]),
            json!({}),
            json!({"provider":""}),
            json!({"provider":"openai","tools":null}),
            json!({"provider":"openai","tools":{}}),
        ] {
            assert!(overrides(model, "n", "https://x/mcp", "s").is_err());
        }
    }
}
