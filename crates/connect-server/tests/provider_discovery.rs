//! Real public CLI uses only the explicitly selected provider credential.
#![allow(clippy::unwrap_used)]
use serde_json::json;
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, ResponseTemplate,
};
#[tokio::test]
async fn public_vapi_only_discovery_needs_no_telnyx_key_and_missing_selection_fails_closed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/assistant"))
        .and(header("authorization", "Bearer fixture-only"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id":"assistant-1","name":"fixture", "model":{}, "api_key":"never-export"}
        ])))
        .expect(2)
        .mount(&server)
        .await;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/gate5-provider-discovery-cli");
    std::fs::create_dir_all(&root).unwrap();
    let directory = tempfile::tempdir_in(root).unwrap();
    let references = directory.path().join("references.toml");
    connect_server::cli::init::private_write(&references,
        b"version = 1\n[references.VAPI_PRIVATE_KEY]\nstorage = 'environment'\nvariable = 'CONNECT_DISCOVERY_FIXTURE_KEY'\n").unwrap();
    let binary = std::env::var_os("CONNECT_TEST_BINARY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_connect").into());
    let clean = || {
        let mut command = std::process::Command::new(&binary);
        command.env_clear();
        for name in ["PATH", "HOME", "TMPDIR", "LANG"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command
    };
    for (index, command) in [["setup", "provider-discover"], ["providers", "discover"]]
        .into_iter()
        .enumerate()
    {
        let output = directory.path().join(format!("catalog-{index}.json"));
        let mut cli = clean();
        cli.args(command)
            .args(["--provider", "vapi", "--output"])
            .arg(&output)
            .env("CONNECT_SECRET_REFERENCES", &references)
            .env("CONNECT_DISCOVERY_FIXTURE_KEY", "fixture-only")
            .env("CONNECT_VAPI_API_BASE_URL", server.uri())
            .env_remove("VAPI_PRIVATE_KEY")
            .env("TELNYX_API_KEY", "ambient-must-not-be-used");
        let response = tokio::task::spawn_blocking(move || cli.output().unwrap())
            .await
            .unwrap();
        assert!(
            response.status.success(),
            "{}",
            String::from_utf8_lossy(&response.stderr)
        );
        let bytes = std::fs::read(&output).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("never-export"));
        if index == 0 {
            assert_eq!(value["complete"], false);
            assert_eq!(value["resources"].as_array().unwrap().len(), 1);
        } else {
            assert_eq!(value["complete"], false);
            assert!(value.get("phone_numbers").is_none());
        }
        let absent = directory.path().join(format!("absent-{index}.json"));
        // Rebuild command: changing selection cannot read missing Telnyx from ambient.
        let mut denied = clean();
        denied
            .args(command)
            .args(["--provider", "telnyx", "--output"])
            .arg(&absent)
            .env("CONNECT_SECRET_REFERENCES", &references)
            .env("TELNYX_API_KEY", "ambient-must-not-be-used");
        let response = tokio::task::spawn_blocking(move || denied.output().unwrap())
            .await
            .unwrap();
        assert!(!response.status.success() && !absent.exists());
        assert!(!String::from_utf8_lossy(&response.stderr).contains("ambient-must-not-be-used"));
    }
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}
