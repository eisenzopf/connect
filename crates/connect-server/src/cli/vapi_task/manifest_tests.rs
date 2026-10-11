//! Private manifest validation occurs before provider workers or intent writes.
#![allow(clippy::unwrap_used)]
use super::Manifest;
use crate::cli::init::private_write;
use serde_json::{json, Value};

fn value() -> Value {
    json!({"profile":"connect-vapi-task-host/1", "state_directory":"private-setup",
        "definition":{"credential_id":"owner", "request_id":"host-issue", "task_id":"task",
            "assistant_id":"assistant-one", "origin":"chat_session", "operations":["history"],
            "expires_at":4102444800_i64}, "callback_bind":"127.0.0.1:9234"})
}
fn load(value: &Value) -> anyhow::Result<Manifest> {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("host.json");
    private_write(&path, &serde_json::to_vec(value).unwrap()).unwrap();
    Manifest::load(&path)
}
#[test]
fn supported_modes_resolve_state_beside_private_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("host.json");
    private_write(&path, &serde_json::to_vec(&value()).unwrap()).unwrap();
    assert_eq!(
        Manifest::load(&path).unwrap().state_directory,
        temp.path().join("private-setup")
    );
    let mut voice = value();
    voice["definition"]["origin"] = json!("call");
    voice.as_object_mut().unwrap().remove("callback_bind");
    voice["session_id"] = json!("accepted-session");
    assert!(load(&voice).is_ok());
}
#[test]
fn rejects_ambiguous_modes_public_binding_and_callback_credentials() {
    for (field, content) in [
        ("profile", json!("other")),
        ("session_id", json!("voice-in-chat")),
        ("callback_bind", json!("127.0.0.1:0")),
        ("callback_bind", json!("0.0.0.0:9234")),
        ("callback_bind", json!(null)),
        (
            "callback_url",
            json!("https://user:canary@example.invalid/mcp"),
        ),
        ("callback_url", json!("http://public.example.invalid/mcp")),
        ("state_directory", json!("")),
        ("unknown", json!("ignored")),
    ] {
        let mut request = value();
        request[field] = content;
        let error = load(&request).err().expect("invalid manifest").to_string();
        assert!(!error.contains("canary"));
    }
    for origin in ["call", "chat"] {
        let mut request = value();
        request["definition"]["origin"] = json!(origin);
        assert!(load(&request).is_err());
    }
}
#[test]
fn rejects_oversized_or_nonregular_private_inputs() {
    let temp = tempfile::tempdir().unwrap();
    assert!(Manifest::load(temp.path()).is_err());
    let path = temp.path().join("large.json");
    private_write(&path, &vec![b' '; 65_537]).unwrap();
    assert!(Manifest::load(&path).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let path = temp.path().join("host.json");
        private_write(&path, &serde_json::to_vec(&value()).unwrap()).unwrap();
        let link = temp.path().join("link.json");
        symlink(&path, &link).unwrap();
        assert!(Manifest::load(&link).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Manifest::load(&path).is_err());
    }
}
