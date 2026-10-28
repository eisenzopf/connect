//! Installed typed Node package -> public Rust SDK + current canonical HTTP authority.
use super::*;
use std::{os::unix::fs::PermissionsExt, path::PathBuf, process::Stdio};

#[tokio::test]
#[ignore = "Requires installed Node consumer via CONNECT_GATE6_NODE_CONSUMER under .local"]
async fn installed_typed_node_sdk_recovers_typed_operations_and_current_authority() {
    let script = PathBuf::from(std::env::var_os("CONNECT_GATE6_NODE_CONSUMER").unwrap())
        .canonicalize()
        .unwrap();
    let root = std::env::current_dir().unwrap();
    assert!(script.starts_with(root.join(".local").canonicalize().unwrap()));
    let project = script.parent().unwrap();
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_extra_scopes(
            algorithm,
            8,
            true,
            false,
            false,
            &["operation.cancel", "operation.reconcile"],
        )
        .await;
        let sdk_config = connect_client::sdk::ClientConfig {
            control: f.client.clone(),
            tenant_id: "tnt_default".parse().unwrap(),
            application: None,
        };
        let native = f.temp.path().join("node-native.toml");
        let pid_file = f.temp.path().join("node-helper-pids");
        init::private_write(&native, toml::to_string(&sdk_config).unwrap().as_bytes()).unwrap();
        let wrapper = f.temp.path().join("owned-connect-wrapper");
        init::private_write(&wrapper, br#"#!/usr/bin/env python3
import os,sys
with open(os.environ['CONNECT_G6_NODE_PID_FILE'],'a') as stream: stream.write(str(os.getpid())+'\n')
os.execv(os.environ['CONNECT_G6_NODE_CONNECT'],[os.environ['CONNECT_G6_NODE_CONNECT'],*sys.argv[1:]])
"#).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let input = f.temp.path().join("node-spec.json");
        init::private_write(
            &input,
            &serde_json::to_vec(&json!({
                "project":project,"config":native,"executable":wrapper,"pid_file":pid_file,
                "peer_identity":f.peer.identity_id,"http":format!("http://{}",f.server.http_address.unwrap()),
                "credential":f.client.credential,"algorithm":algorithm
            }))
            .unwrap(),
        )
        .unwrap();
        let stderr_path = project.join(format!("native-server-{algorithm}.stderr"));
        let mut child = tokio::process::Command::new("node")
            .arg(&script)
            .arg(&input)
            .env("CONNECT_G6_NODE_PID_FILE", &pid_file)
            .env("CONNECT_G6_NODE_CONNECT", env!("CARGO_BIN_EXE_connect"))
            .current_dir(project)
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(std::fs::File::create(&stderr_path).unwrap()))
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut stdin = child.stdin.take().unwrap();
        async fn phase(lines: &mut Lines<BufReader<ChildStdout>>, expected: &str) -> Value {
            let line = tokio::time::timeout(Duration::from_secs(15), lines.next_line())
                .await
                .unwrap()
                .unwrap()
                .expect("Node phase result");
            assert!(line.len() < 4096);
            let value: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(value["phase"], expected);
            value
        }
        let initial = phase(&mut lines, "initial").await;
        assert_eq!(
            std::fs::metadata(&native).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::write(&native, toml::to_string(&sdk_config).unwrap()).unwrap();
        stdin.write_all(b"restart\n").await.unwrap();
        let recovered = phase(&mut lines, "recovered").await;
        assert_eq!(initial["record_id"], recovered["record_id"]);
        let principal = f.server.store.authenticate(&f.client.credential).unwrap();
        let admin = f.server.store.authenticate(&f.admin).unwrap();
        let store = f.server.store.clone();
        tokio::task::spawn_blocking(move || store.revoke(&admin, &principal.credential_id))
            .await
            .unwrap()
            .unwrap();
        stdin.write_all(b"revoked\n").await.unwrap();
        let completed = phase(&mut lines, "completed").await;
        drop(stdin);
        assert!(
            tokio::time::timeout(Duration::from_secs(5), lines.next_line())
                .await
                .unwrap()
                .unwrap()
                .is_none()
        );
        let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(
            status.success(),
            "Node worker failed; inspect private stderr"
        );
        assert!(!std::fs::read_to_string(stderr_path)
            .unwrap()
            .contains(&f.client.credential));
        f.server.shutdown().await.unwrap();
        eprintln!(
            "installed-typed-node-sdk-witness {}",
            json!({"initial":initial,"recovered":recovered,"completed":completed,"server_shutdown_joined":true})
        );
    }
}
