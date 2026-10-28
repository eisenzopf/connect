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
            true,
            true,
            &["operation.cancel", "operation.reconcile"],
        )
        .await;
        crate::canonical_native_connections::ready_endpoint_with_capabilities(
            &f,
            vec!["audio".into(), "video".into(), "data".into()],
        )
        .await;
        let mut peer_scope = connect_store::Scope::collaboration(vec!["*".into()]);
        peer_scope.operations.extend([
            "stream.subscribe".into(),
            "stream.unsubscribe".into(),
            "connection.end".into(),
        ]);
        let sdk_config = connect_client::sdk::ClientConfig {
            control: f.client.clone(),
            tenant_id: "tnt_default".parse().unwrap(),
            application: None,
        };
        let peer_control = crate::canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            72,
            vec!["audio".into()],
            peer_scope,
        );
        let peer_sdk_config = connect_client::sdk::ClientConfig {
            control: peer_control.clone(),
            tenant_id: "tnt_default".parse().unwrap(),
            application: None,
        };
        let peer_native = f.temp.path().join("peer-sdk-client.toml");
        init::private_write(
            &peer_native,
            toml::to_string(&peer_sdk_config).unwrap().as_bytes(),
        )
        .unwrap();
        let native = f.temp.path().join("node-native.toml");
        let pid_file = f.temp.path().join("node-helper-pids");
        init::private_write(&native, toml::to_string(&sdk_config).unwrap().as_bytes()).unwrap();
        let wrapper = f.temp.path().join("owned-connect-wrapper");
        init::private_write(&wrapper, br#"#!/usr/bin/env python3
import os,sys,time
path=os.path.join(os.environ['CONNECT_G6_NODE_HELPER_DIR'],'helper-'+os.environ['CONNECT_G6_NODE_ALGORITHM']+'-'+str(os.getpid())+'.stderr')
fd=os.open(path,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600)
os.dup2(fd,2)
os.close(fd)
os.write(2,('owned-exec monotonic_ns='+str(time.monotonic_ns())+' pid='+str(os.getpid())+'\n').encode())
with open(os.environ['CONNECT_G6_NODE_PID_FILE'],'a') as stream: stream.write(str(os.getpid())+'\n')
os.execv(os.environ['CONNECT_G6_NODE_CONNECT'],[os.environ['CONNECT_G6_NODE_CONNECT'],*sys.argv[1:]])
"#).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let input = f.temp.path().join("node-spec.json");
        init::private_write(
            &input,
            &serde_json::to_vec(&json!({
                "project":project,"config":native,"executable":wrapper,"pid_file":pid_file,
                "peer_identity":f.peer.identity_id,"peer_config":peer_native,"http":format!("http://{}",f.server.http_address.unwrap()),
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
            .env("CONNECT_G6_NODE_HELPER_DIR", project)
            .env("CONNECT_G6_NODE_ALGORITHM", algorithm)
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
        async fn phase(
            lines: &mut Lines<BufReader<ChildStdout>>,
            expected: &str,
            algorithm: &str,
        ) -> Value {
            let started = tokio::time::Instant::now();
            let until = tokio::time::Instant::now() + Duration::from_secs(15);
            let mut checkpoint = "create".to_owned();
            loop {
                let line = match tokio::time::timeout_at(until, lines.next_line()).await {
                    Ok(line) => line.unwrap().expect("Node phase result"),
                    Err(_) => {
                        eprintln!(
                            "installed-typed-node-sdk-failure {}",
                            json!({"phase":"failed","checkpoint":checkpoint,"kind":"deadline","certainty":null,"command":null,"observedState":null,"line":null,"column":null})
                        );
                        panic!("Original Node phase deadline elapsed");
                    }
                };
                assert!(line.len() < 4096);
                let value: Value = serde_json::from_str(&line).unwrap();
                if value["phase"] == "progress" {
                    checkpoint = value["checkpoint"].as_str().unwrap().to_owned();
                    continue;
                }
                if value["phase"] == "failed" {
                    eprintln!("installed-typed-node-sdk-failure {}", value);
                }
                assert_eq!(value["phase"], expected);
                eprintln!(
                    "installed-typed-node-sdk-phase {}",
                    json!({"algorithm":algorithm,"phase":expected,"passed":true,"seconds":started.elapsed().as_secs_f64(),"deadline_seconds":15})
                );
                return value;
            }
        }
        // These are separate bounded functional scenarios. The installed
        // journey grew beyond the original single Session-only phase; preserve
        // every 4s SDK request and 15s absolute scenario deadline. The source
        // runner independently retains its 90s whole-function limit.
        let controls = phase(&mut lines, "controls-ready", algorithm).await;
        assert_eq!(controls["session_participants_accepted"], true);
        stdin.write_all(b"native\n").await.unwrap();
        let native_phase = phase(&mut lines, "native-complete", algorithm).await;
        assert_eq!(native_phase["native_controls_completed"], true);
        stdin.write_all(b"finish\n").await.unwrap();
        let initial = phase(&mut lines, "initial", algorithm).await;
        assert_eq!(
            std::fs::metadata(&native).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::write(&native, toml::to_string(&sdk_config).unwrap()).unwrap();
        stdin.write_all(b"restart\n").await.unwrap();
        let recovered = phase(&mut lines, "recovered", algorithm).await;
        assert_eq!(initial["record_id"], recovered["record_id"]);
        let principal = f.server.store.authenticate(&f.client.credential).unwrap();
        let admin = f.server.store.authenticate(&f.admin).unwrap();
        let store = f.server.store.clone();
        tokio::task::spawn_blocking(move || store.revoke(&admin, &principal.credential_id))
            .await
            .unwrap()
            .unwrap();
        stdin.write_all(b"revoked\n").await.unwrap();
        let completed = phase(&mut lines, "completed", algorithm).await;
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
        assert!(!std::fs::read_to_string(&stderr_path)
            .unwrap()
            .contains(&f.client.credential));
        assert!(!std::fs::read_to_string(&stderr_path)
            .unwrap()
            .contains(&peer_control.credential));
        for entry in std::fs::read_dir(project).unwrap() {
            let entry = entry.unwrap();
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(&format!("helper-{algorithm}-"))
            {
                assert_eq!(
                    entry.metadata().unwrap().permissions().mode() & 0o777,
                    0o600
                );
                assert!(!std::fs::read_to_string(entry.path())
                    .unwrap()
                    .contains(&f.client.credential));
                assert!(!std::fs::read_to_string(entry.path())
                    .unwrap()
                    .contains(&peer_control.credential));
            }
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            while {
                let observed = f.server.service.canonical_runtime_snapshot().unwrap();
                observed.physical_peers != 0 || observed.media_readers != 0
            } {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        f.server.shutdown().await.unwrap();
        eprintln!(
            "installed-typed-node-sdk-witness {}",
            json!({"functional_phase_profile":"node-controls/2","controls":controls,"native":native_phase,"initial":initial,"recovered":recovered,"completed":completed,"server_shutdown_joined":true,"physical_peers_drained":true,"media_readers_drained":true})
        );
    }
}
