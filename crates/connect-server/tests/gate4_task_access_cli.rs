//! Protected exports, redaction and stable issuance recovery through the actual CLI.
#![allow(clippy::unwrap_used)]
#[path = "support/task_access.rs"]
mod access;
#[path = "support/task_access_cli.rs"]
mod cli;
#[path = "support/mcp_fixture.rs"]
mod fixture;
use connect_protocol::tasks::InvocationKind;
use connect_server::cli::init;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lost_response_resume_reconciles_original_id_and_protects_export() {
    let f = fixture::Fixture::start_independent().await;
    let config = cli::config(&f);
    let input = f.directory().join("issue.json");
    let output = f.directory().join("grant.json");
    let request = access::request(
        &f.task.id,
        "lost-response-install",
        InvocationKind::HostSession,
    );
    init::private_write(&input, &serde_json::to_vec(&request).unwrap()).unwrap();
    let (proxy, task, receive) = cli::proxy(&f).await;
    let lost = cli::run(
        &config,
        proxy,
        "issue",
        &[("--input", &input), ("--output", &output)],
    )
    .await;
    assert!(!lost.status.success());
    let (id, secret) = receive.await.unwrap();
    cli::no_secret(&lost, &[&f.owner.credential, &secret]);
    assert!(cli::file(&output).grant.is_none());
    task.abort();
    let _ = task.await;
    let resumed = cli::run(
        &config,
        f.server.http_address.unwrap(),
        "resume",
        &[("--grant-file", &output)],
    )
    .await;
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    cli::no_secret(&resumed, &[&f.owner.credential, &secret]);
    let file = cli::file(&output);
    assert_eq!(
        serde_json::to_value(file.request).unwrap(),
        serde_json::to_value(request).unwrap()
    );
    let grant = file.grant.unwrap();
    assert_eq!(grant.grant.id, id);
    assert_eq!(grant.expose_secret(), secret);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&output).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let before = std::fs::read(&output).unwrap();
    let saved = cli::run(&config, proxy, "resume", &[("--grant-file", &output)]).await;
    assert!(saved.status.success());
    assert!(String::from_utf8_lossy(&saved.stdout).contains("saved-not-refreshed"));
    assert_eq!(std::fs::read(&output).unwrap(), before);
    cli::no_secret(&saved, &[&f.owner.credential, &secret]);
    let duplicate = cli::run(
        &config,
        f.server.http_address.unwrap(),
        "issue",
        &[("--input", &input), ("--output", &output)],
    )
    .await;
    assert!(!duplicate.status.success());
    assert_eq!(std::fs::read(&output).unwrap(), before);
    cli::no_secret(&duplicate, &[&f.owner.credential, &secret]);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invalid_files_and_public_destination_fail_without_reflecting_input() {
    let f = fixture::Fixture::start_independent().await;
    let config = cli::config(&f);
    let input = f.directory().join("bad.json");
    let output = f.directory().join("grant.json");
    init::private_write(&input, br#"{"opaque":"test-private-marker","malformed":}"#).unwrap();
    let invalid = cli::run(
        &config,
        f.server.http_address.unwrap(),
        "issue",
        &[("--input", &input), ("--output", &output)],
    )
    .await;
    assert!(!invalid.status.success());
    cli::no_secret(&invalid, &["test-private-marker", &f.owner.credential]);
    assert!(!output.exists());
    init::private_replace(
        &input,
        &serde_json::to_vec(&access::request(
            &f.task.id,
            "public-denial",
            InvocationKind::Native,
        ))
        .unwrap(),
    )
    .unwrap();
    let public = cli::run(
        &config,
        "192.0.2.1:80".parse().unwrap(),
        "issue",
        &[("--input", &input), ("--output", &output)],
    )
    .await;
    assert!(!public.status.success());
    assert!(!output.exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::{symlink, PermissionsExt};
        symlink(&input, &output).unwrap();
        let linked = cli::run(
            &config,
            f.server.http_address.unwrap(),
            "resume",
            &[("--grant-file", &output)],
        )
        .await;
        assert!(!linked.status.success());
        std::fs::remove_file(&output).unwrap();
        init::private_write(&output, br#"{"opaque":"test-private-marker"}"#).unwrap();
        std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o644)).unwrap();
        let exposed = cli::run(
            &config,
            f.server.http_address.unwrap(),
            "resume",
            &[("--grant-file", &output)],
        )
        .await;
        assert!(!exposed.status.success());
        cli::no_secret(&exposed, &["test-private-marker", &f.owner.credential]);
    }
    f.close().await;
}
