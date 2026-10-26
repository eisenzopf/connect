#![allow(clippy::unwrap_used)]
use super::*;
use std::os::unix::fs::{symlink, PermissionsExt};
#[test]
fn configured_der_and_pem_roots_retain_native_bounds() {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    validate_trust(cert.cert.der()).unwrap();
    validate_trust(cert.cert.pem().as_bytes()).unwrap();
    validate_trust(cert.cert.pem().repeat(8).as_bytes()).unwrap();
    for invalid in [
        Vec::new(),
        b"invalid certificate".to_vec(),
        vec![0; files::MAX_TRUST + 1],
        cert.cert.pem().repeat(9).into_bytes(),
        [cert.cert.der().as_ref(), b"trailing"].concat(),
    ] {
        assert!(validate_trust(&invalid).is_err());
    }
}
#[test]
fn private_atomic_files_refuse_replacement_symlinks_links_and_nonprivate_assets() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("installation");
    let dir = Directory::open(&path, true).unwrap();
    dir.publish("intent.json", b"original").unwrap();
    dir.publish("intent.json", b"original").unwrap();
    assert!(dir.publish("intent.json", b"replacement").is_err());
    assert_eq!(dir.read("intent.json").unwrap(), b"original");
    assert!(Directory::open(&path, false).is_err());
    let unrelated = temp.path().join("unrelated");
    std::fs::write(&unrelated, b"keep").unwrap();
    symlink(&unrelated, path.join("linked")).unwrap();
    assert!(dir.read("linked").is_err());
    assert!(dir.publish("linked", b"changed").is_err());
    assert_eq!(std::fs::read(&unrelated).unwrap(), b"keep");
    dir.publish("key", b"private-key").unwrap();
    std::fs::hard_link(path.join("key"), temp.path().join("other-link")).unwrap();
    assert!(dir.read("key").is_err());
    dir.publish("nonprivate", b"bytes").unwrap();
    std::fs::set_permissions(
        path.join("nonprivate"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(dir.read("nonprivate").is_err());
    drop(dir);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(Directory::open(&path, false).is_err());
}
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    app: PathBuf,
    template: PathBuf,
    request: PathBuf,
    output: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = listener.local_addr().unwrap();
        drop(listener);
        crate::cli::canonical_init::initialize_with_http(
            &root,
            true,
            "127.0.0.1:4444".parse().unwrap(),
            Some(endpoint),
        )
        .unwrap();
        let app = root.join("test.application.toml");
        let mut config =
            ApplicationClientConfig::load(&root.join("operator.application.toml")).unwrap();
        config.timeout_ms = 50;
        crate::cli::init::private_write(&app, toml::to_string_pretty(&config).unwrap().as_bytes())
            .unwrap();
        let template = root.join("operator.toml");
        let source = CanonicalClientConfig::load(&template).unwrap();
        // Template contact remains useful even with no readable parent signer.
        std::fs::remove_file(&source.signing_private_key).unwrap();
        let request = root.join("request.json");
        crate::cli::init::private_write(
            &request,
            &serde_json::to_vec(&CredentialIssueRequest {
                operation_id: "original".into(),
                identity_id: source.identity_id,
                operations: vec!["operation.get".into()],
                recipients: vec![],
                expires_at: chrono::Utc::now().timestamp() + 120,
            })
            .unwrap(),
        )
        .unwrap();
        let output = temp.path().join("output");
        Self {
            _temp: temp,
            root,
            app,
            template,
            request,
            output,
        }
    }
}
#[tokio::test]
async fn pending_intent_and_fresh_key_survive_failed_io_without_copying_parent_authority() {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::new();
        let parent = admin(&f.app).unwrap();
        assert!(issue(&f.app, &f.template, &f.request, &f.output, algorithm)
            .await
            .is_err());
        let original = std::fs::read(f.output.join("intent.json")).unwrap();
        let key = std::fs::read(f.output.join("device-signing.der")).unwrap();
        assert!(!original
            .windows(parent.credential.len())
            .any(|v| v == parent.credential.as_bytes()));
        let directory = Directory::open(&f.output, false).unwrap();
        let intent = validate(&directory, &parent).unwrap();
        assert_eq!(intent.native.signing_algorithm, algorithm);
        assert_eq!(intent.native.credential, PLACEHOLDER);
        assert_eq!(public_key(&key, algorithm).unwrap(), intent.public_key);
        drop(directory);
        assert!(resume(&f.app, &f.output).await.is_err());
        assert!(issue(&f.app, &f.template, &f.request, &f.output, algorithm)
            .await
            .is_err());
        assert_eq!(
            std::fs::read(f.output.join("intent.json")).unwrap(),
            original
        );
        assert_eq!(
            std::fs::read(f.output.join("device-signing.der")).unwrap(),
            key
        );
        assert!(!f.output.join("issued.json").exists());
        assert!(!f.output.join("install.json").exists());
        assert!(f.root.join("server.toml").exists());
    }
}
#[tokio::test]
async fn resume_rejects_substituted_issuer_endpoint_key_and_incomplete_preparation() {
    let f = Fixture::new();
    assert!(issue(&f.app, &f.template, &f.request, &f.output, "EdDSA")
        .await
        .is_err());
    let directory = Directory::open(&f.output, false).unwrap();
    let parent = admin(&f.app).unwrap();
    let mut changed = parent.clone();
    changed.credential = "different-current-parent".into();
    assert!(validate(&directory, &changed).is_err());
    changed = parent.clone();
    changed
        .endpoint
        .set_port(if parent.endpoint.port() == 1 { 2 } else { 1 });
    assert!(validate(&directory, &changed).is_err());
    let original = directory.read("intent.json").unwrap();
    let mut edited: serde_json::Value = serde_json::from_slice(&original).unwrap();
    edited["request"]["operation_id"] = serde_json::json!("replacement-logical-id");
    std::fs::write(
        f.output.join("intent.json"),
        serde_json::to_vec(&edited).unwrap(),
    )
    .unwrap();
    assert!(validate(&directory, &parent).is_err());
    std::fs::write(f.output.join("intent.json"), &original).unwrap();
    validate(&directory, &parent).unwrap();
    std::fs::write(f.output.join("device-signing.der"), b"changed").unwrap();
    assert!(validate(&directory, &parent).is_err());
    drop(directory);
    let incomplete = f._temp.path().join("incomplete");
    drop(Directory::open(&incomplete, true).unwrap());
    assert!(resume(&f.app, &incomplete).await.is_err());
    assert!(!incomplete.join("intent.json").exists());
}
