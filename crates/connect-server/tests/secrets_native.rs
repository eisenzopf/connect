//! Opt-in native proof in a child process and a disposable file-based keychain.
#![cfg(target_os = "macos")]
use connect_server::secrets::{
    MutableSecretStore, NativeSecretStore, SecretReference, SecretStore, SecretValue,
};
use security_framework::os::macos::keychain::{CreateOptions, SecKeychain};
use std::{path::PathBuf, process::Command};

#[test]
#[ignore = "native Keychain qualification; disposable keychain only"]
fn isolated_native_keychain() {
    let status = Command::new(std::env::current_exe().expect("test binary"))
        .args([
            "--ignored",
            "--exact",
            "native_keychain_child",
            "--nocapture",
        ])
        .env("CONNECT_NATIVE_KEYCHAIN_CHILD", "1")
        .status()
        .expect("child");
    assert!(status.success());
}
#[test]
#[ignore = "run only via isolated_native_keychain"]
fn native_keychain_child() {
    if std::env::var("CONNECT_NATIVE_KEYCHAIN_CHILD").as_deref() != Ok("1") {
        return;
    }
    let before = Command::new("/usr/bin/security")
        .arg("list-keychains")
        .output()
        .expect("search list")
        .stdout;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.local/gate5-keychain");
    std::fs::create_dir_all(&root).expect("scratch");
    let dir = tempfile::Builder::new()
        .prefix("test-")
        .tempdir_in(root)
        .expect("private scratch");
    let path = dir.path().join("disposable.keychain-db");
    let password = uuid::Uuid::new_v4().to_string();
    let mut chain = CreateOptions::new()
        .password(&password)
        .create(&path)
        .expect("create disposable");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("protect only disposable keychain file");
    }
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = Command::new("/usr/bin/security")
                .arg("delete-keychain")
                .arg(&self.0)
                .output();
        }
    }
    let cleanup = Cleanup(path.clone());
    let _interaction = SecKeychain::disable_user_interaction().expect("noninteractive child");
    let service = format!("Connect/test/{}", uuid::Uuid::new_v4());
    let reference = SecretReference::Keychain {
        service: service.clone(),
        account: "fixture".into(),
        keychain: Some(path.clone()),
    };
    assert!(NativeSecretStore.read(&reference).is_err());
    NativeSecretStore
        .replace(
            &reference,
            &SecretValue::new("disposable-test-secret-v1".into()).expect("value"),
        )
        .expect("write");
    assert_eq!(
        NativeSecretStore.read(&reference).expect("read").expose(),
        "disposable-test-secret-v1"
    );
    NativeSecretStore
        .replace(
            &reference,
            &SecretValue::new("disposable-test-secret-v2".into()).expect("value"),
        )
        .expect("rotate");
    drop(chain);
    assert_eq!(
        NativeSecretStore.read(&reference).expect("reopen").expose(),
        "disposable-test-secret-v2"
    );
    assert!(Command::new("/usr/bin/security")
        .arg("lock-keychain")
        .arg(&path)
        .status()
        .expect("lock")
        .success());
    assert!(
        NativeSecretStore.read(&reference).is_err(),
        "locked storage must fail without a prompt or fallback"
    );
    chain = SecKeychain::open(&path).expect("open");
    chain.unlock(Some(&password)).expect("unlock test only");
    assert_eq!(
        NativeSecretStore
            .read(&reference)
            .expect("unlocked")
            .expose(),
        "disposable-test-secret-v2"
    );
    NativeSecretStore.remove(&reference).expect("remove owned");
    assert!(NativeSecretStore.read(&reference).is_err());
    drop(chain);
    let refs_path = dir.path().join("references.toml");
    let mut refs = connect_server::secrets::SecretReferences {
        version: 1,
        references: Default::default(),
    };
    refs.references
        .insert("VAPI_PRIVATE_KEY".into(), reference.clone());
    connect_server::cli::init::private_write(
        &refs_path,
        toml::to_string_pretty(&refs)
            .expect("serialize refs")
            .as_bytes(),
    )
    .expect("write refs");
    // These environment changes occur only in this dedicated child process.
    std::env::set_var("VAPI_PRIVATE_KEY", "ambient-fixture-never-used");
    std::env::set_var("CONNECT_SECRET_REFERENCES", &refs_path);
    assert!(
        connect_server::secrets::optional("VAPI_PRIVATE_KEY").is_err(),
        "missing Keychain entry cannot fall back to ambient plaintext"
    );
    refs.references.clear();
    connect_server::cli::init::private_replace(
        &refs_path,
        toml::to_string_pretty(&refs)
            .expect("serialize refs")
            .as_bytes(),
    )
    .expect("replace refs");
    assert!(connect_server::secrets::optional("VAPI_PRIVATE_KEY")
        .expect("absent ref")
        .is_none());
    // Even a regressed env-only constructor cannot contact a provider here:
    // Telnyx ambient credentials are removed, and no client survives construction.
    for (subcommand, flag) in [("discover", "--output"), ("check", "--state")] {
        let selected_binary = std::env::var_os("CONNECT_TEST_BINARY")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_connect")));
        let output = Command::new(&selected_binary)
            .args(["providers", subcommand, flag])
            .arg(dir.path().join(format!("never-created-{subcommand}.json")))
            .env_remove("TELNYX_API_KEY")
            .output()
            .expect("source CLI secret preflight");
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr)
            .contains("Required server secret is not configured"));
        assert!(!dir
            .path()
            .join(format!("never-created-{subcommand}.json"))
            .exists());
    }
    let selected_binary = std::env::var_os("CONNECT_TEST_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_connect")));
    let native_state = dir.path().join("never-created-native-voice.json");
    let output = Command::new(&selected_binary)
        .args(["providers", "voice-native", "--voice-state"])
        .arg(dir.path().join("no-provider-fixture-needed.json"))
        .arg("--state")
        .arg(&native_state)
        .env_remove("TELNYX_API_KEY")
        .output()
        .expect("selected CLI native provider secret preflight");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("Required server secret is not configured"));
    assert!(!native_state.exists());
    checkpoint_vault_lifecycle(dir.path(), &path, &password);
    public_cli_vault_lifecycle(dir.path(), &path, &password);
    drop(cleanup);
    let after = Command::new("/usr/bin/security")
        .arg("list-keychains")
        .output()
        .expect("search list")
        .stdout;
    assert_eq!(
        before, after,
        "native proof must restore the original search list"
    );
    println!(
        "Native disposable Keychain: missing/write/rotation/reopen/locked/unlock/delete passed"
    );
}

fn checkpoint_vault_lifecycle(root: &std::path::Path, keychain: &std::path::Path, password: &str) {
    use connect_connectors::setup::SetupSecretStore;
    use connect_server::{cli::init, setup_vault::CheckpointVault};
    let directory = root.join("installation");
    init::initialize(&directory, false, "127.0.0.1:0".parse().expect("bind"))
        .expect("private installation");
    let operator = directory.join("operator.toml");
    let server = directory.join("server.toml");
    let state = directory.join("legacy-sip.json");
    let original_secret = "original-disposable-sip-password";
    let legacy = serde_json::json!({"plan":{"name":"connect","assistant_id":null,"phone_number_id":null,"callback_url":null,"create_sip":true,"outbound_voice_profile_id":null},"name":"connect-disposable-sip","assistant_id":"12345678-1234-1234-1234-123456789012","messaging_profile_id":null,"sip_connection_id":"7654321","sip_user":"disposable-fixture-user","sip_password":original_secret,"pending":"sip","number_messaging_applied":false,"number_sip_applied":false,"previous_number":null});
    init::private_write(
        &state,
        &serde_json::to_vec(&legacy).expect("legacy serialize"),
    )
    .expect("legacy checkpoint");
    let vault = CheckpointVault::open(&operator, &server, Some(keychain.to_path_buf()))
        .expect("current operator vault");
    let migrated =
        connect_connectors::setup::migrate_secrets(&state, &vault).expect("native SIP migration");
    let reference = migrated.secret_reference.expect("reference");
    let operation = migrated.operation_id.expect("operation");
    let protected = std::fs::read(&state).expect("protected checkpoint");
    let metadata: serde_json::Value = serde_json::from_slice(&protected).expect("metadata");
    assert!(metadata.get("sip_password").is_none());
    assert!(!String::from_utf8_lossy(&protected).contains(original_secret));
    assert_eq!(vault.read(&reference).expect("vault read"), original_secret);
    assert!(
        !connect_connectors::setup::migrate_secrets(&state, &vault)
            .expect("repeat migration")
            .changed
    );
    let reopened = CheckpointVault::open(&operator, &server, Some(keychain.to_path_buf()))
        .expect("reopened vault");
    assert_eq!(
        reopened.read(&reference).expect("restart read"),
        original_secret
    );
    assert!(Command::new("/usr/bin/security")
        .arg("lock-keychain")
        .arg(keychain)
        .status()
        .expect("lock disposable")
        .success());
    assert!(connect_connectors::setup::migrate_secrets(&state, &vault).is_err());
    assert_eq!(std::fs::read(&state).expect("retained"), protected);
    let mut chain = SecKeychain::open(keychain).expect("open disposable");
    chain.unlock(Some(password)).expect("unlock disposable");
    let client = connect_client::ClientConfig::load(&operator).expect("operator");
    let installation = connect_store::Store::authorize_secret_storage_file(
        &directory.join("state.sqlite3"),
        &client.credential,
    )
    .expect("current admission");
    let location = SecretReference::Keychain {
        service: format!("Connect/provisioning/{installation}"),
        account: format!("sip-{operation}"),
        keychain: Some(keychain.to_path_buf()),
    };
    NativeSecretStore
        .remove(&location)
        .expect("remove only disposable SIP entry");
    assert!(connect_connectors::setup::migrate_secrets(&state, &vault).is_err());
    assert!(NativeSecretStore.read(&location).is_err());
    assert!(
        connect_connectors::setup::restore_secret(&state, &vault, "wrong-disposable-password")
            .is_err()
    );
    assert!(NativeSecretStore.read(&location).is_err());
    connect_connectors::setup::restore_secret(&state, &vault, original_secret)
        .expect("import original SIP password");
    assert_eq!(std::fs::read(&state).expect("same checkpoint"), protected);
    let store = connect_store::Store::open(&directory.join("state.sqlite3"), Default::default())
        .expect("fixture store");
    let actor = store
        .authenticate(&client.credential)
        .expect("operator actor");
    let replacement = store
        .issue(
            &actor.identity_id,
            connect_store::Scope::operator(),
            chrono::Utc::now().timestamp() + 3600,
        )
        .expect("replacement authority");
    store
        .revoke(&actor, &client.credential_id)
        .expect("revoke old authority");
    assert!(vault.read(&reference).is_err());
    assert!(connect_connectors::setup::migrate_secrets(&state, &vault).is_err());
    assert_eq!(std::fs::read(&state).expect("revoked retained"), protected);
    let mut replacement_client = client;
    replacement_client.credential = replacement.token;
    replacement_client.credential_id = replacement.id;
    let replacement_path = directory.join("replacement.toml");
    init::private_write(
        &replacement_path,
        toml::to_string_pretty(&replacement_client)
            .expect("client serialize")
            .as_bytes(),
    )
    .expect("replacement config");
    let repaired = CheckpointVault::open(&replacement_path, &server, Some(keychain.to_path_buf()))
        .expect("current replacement admission");
    assert_eq!(
        repaired
            .read(&reference)
            .expect("existing secret under current authority"),
        original_secret
    );
    NativeSecretStore
        .remove(&location)
        .expect("remove owned SIP test entry");
    println!("Native checkpoint vault: legacy migration/restart/lock/missing/correct-only restore/current revocation passed");
}

fn public_cli_vault_lifecycle(root: &std::path::Path, keychain: &std::path::Path, password: &str) {
    use std::{io::Write, process::Stdio};
    let binary = std::env::var_os("CONNECT_TEST_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_connect")));
    let installation = root.join("public-cli-vault");
    let initialized = Command::new(&binary)
        .args(["setup", "init", "--directory"])
        .arg(&installation)
        .args(["--bind", "127.0.0.1:0"])
        .output()
        .expect("selected binary first-run initialization");
    assert!(initialized.status.success());
    let state = installation.join("legacy-sip.json");
    let original_secret = "candidate-original-disposable-sip-password";
    let legacy = serde_json::json!({"plan":{"name":"connect","assistant_id":null,"phone_number_id":null,"callback_url":null,"create_sip":true,"outbound_voice_profile_id":null},"name":"connect-disposable-public-cli","assistant_id":null,"messaging_profile_id":null,"sip_connection_id":"7654321","sip_user":"disposable-public-cli","sip_password":original_secret,"pending":"sip","number_messaging_applied":false,"number_sip_applied":false,"previous_number":null});
    connect_server::cli::init::private_write(
        &state,
        &serde_json::to_vec(&legacy).expect("fixture serialization"),
    )
    .expect("private legacy fixture");
    let execute = |subcommand: &str, input: Option<&str>| {
        let mut child = Command::new(&binary)
            .args(["providers", subcommand, "--state"])
            .arg(&state)
            .arg("--config")
            .arg(installation.join("operator.toml"))
            .arg("--server-config")
            .arg(installation.join("server.toml"))
            .arg("--keychain")
            .arg(keychain)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("selected public vault CLI");
        let mut stdin = child.stdin.take().expect("owned stdin");
        if let Some(input) = input {
            stdin
                .write_all(input.as_bytes())
                .expect("original-value stdin");
        }
        drop(stdin);
        child
            .wait_with_output()
            .expect("public vault CLI completion")
    };
    let migrated = execute("vault-migrate", None);
    assert!(migrated.status.success());
    assert!(!String::from_utf8_lossy(&migrated.stdout).contains(original_secret));
    let protected = std::fs::read(&state).expect("protected checkpoint");
    let metadata: serde_json::Value =
        serde_json::from_slice(&protected).expect("metadata checkpoint");
    assert!(metadata.get("sip_password").is_none());
    assert!(!String::from_utf8_lossy(&protected).contains(original_secret));
    let repeated = execute("vault-migrate", None);
    assert!(repeated.status.success());
    let receipt: serde_json::Value =
        serde_json::from_slice(&repeated.stdout).expect("metadata receipt");
    assert_eq!(receipt["changed"], false);
    assert_eq!(
        std::fs::read(&state).expect("unchanged checkpoint"),
        protected
    );
    assert!(Command::new("/usr/bin/security")
        .arg("lock-keychain")
        .arg(keychain)
        .status()
        .expect("lock owned keychain")
        .success());
    assert!(!execute("vault-migrate", None).status.success());
    assert_eq!(
        std::fs::read(&state).expect("locked checkpoint retained"),
        protected
    );
    assert!(Command::new("/usr/bin/security")
        .args(["unlock-keychain", "-p", password])
        .arg(keychain)
        .status()
        .expect("unlock owned keychain")
        .success());
    let reference = metadata["sip_password_ref"]
        .as_str()
        .expect("metadata reference");
    let components = reference.split('/').collect::<Vec<_>>();
    assert_eq!(components.len(), 4);
    let refs_path = installation.join("owned-removal.toml");
    let refs = connect_server::secrets::SecretReferences {
        version: 1,
        references: [(
            "SIP_PASSWORD".into(),
            SecretReference::Keychain {
                service: format!("Connect/provisioning/{}", components[2]),
                account: format!("sip-{}", components[3]),
                keychain: Some(keychain.to_path_buf()),
            },
        )]
        .into(),
    };
    connect_server::cli::init::private_write(
        &refs_path,
        toml::to_string_pretty(&refs)
            .expect("metadata references")
            .as_bytes(),
    )
    .expect("private removal reference");
    let remove = || {
        Command::new(&binary)
            .args(["setup", "secret-remove", "--references"])
            .arg(&refs_path)
            .args(["--name", "SIP_PASSWORD"])
            .output()
            .expect("remove owned test entry")
    };
    assert!(remove().status.success());
    assert!(!execute("vault-migrate", None).status.success());
    assert!(!execute("vault-restore", Some("wrong-disposable-original"))
        .status
        .success());
    let restored = execute("vault-restore", Some(original_secret));
    assert!(restored.status.success());
    assert!(!String::from_utf8_lossy(&restored.stdout).contains(original_secret));
    assert_eq!(
        std::fs::read(&state).expect("restored checkpoint retained"),
        protected
    );
    assert!(execute("vault-migrate", None).status.success());
    assert!(remove().status.success());
}
