//! Untrusted loopback responses cannot reflect bearer prose or redirect setup.
#![allow(clippy::unwrap_used)]
#[path = "support/task_access.rs"]
mod access;
#[path = "support/task_access_cli.rs"]
mod cli;
#[path = "support/mcp_fixture.rs"]
mod fixture;
use connect_protocol::{tasks::*, Error};
use connect_server::cli::init;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn untrusted_errors_redirects_and_large_responses_keep_pending_intent_private() {
    let f = fixture::Fixture::start_independent().await;
    let config = cli::config(&f);
    let input = f.directory().join("issue.json");
    let request = access::request(&f.task.id, "untrusted-install", InvocationKind::Native);
    let TaskAccessCommand::Issue { definition, .. } = &request.command else {
        unreachable!()
    };
    let reflected = InvocationGrant {
        id: f.owner.credential.clone(),
        task_id: definition.task_id.clone(),
        binding: definition.binding.clone(),
        operations: definition.operations.clone(),
        expires_at: definition.expires_at,
        revoked: false,
    };
    init::private_write(&input, &serde_json::to_vec(&request).unwrap()).unwrap();
    let destination = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&destination)
        .await;
    let templates = [
        ResponseTemplate::new(200).set_body_json(TaskAccessResponse {
            profile: TASK_ACCESS_PROFILE.into(),
            result: Err(Error::new("forbidden", &f.owner.credential)),
        }),
        ResponseTemplate::new(307)
            .insert_header("location", format!("{}/v1/task-access", destination.uri())),
        ResponseTemplate::new(200).set_body_string(format!(
            "{}{}",
            f.owner.credential,
            "x".repeat(65_536)
        )),
        ResponseTemplate::new(200).set_body_json(TaskAccessResponse {
            profile: TASK_ACCESS_PROFILE.into(),
            result: Ok(TaskAccessOutcome::Issued(ProtectedTaskGrant::new(
                f.issued.grant.clone(),
                f.issued.expose_secret().into(),
            ))),
        }),
        ResponseTemplate::new(200).set_body_json(TaskAccessResponse {
            profile: TASK_ACCESS_PROFILE.into(),
            result: Ok(TaskAccessOutcome::Issued(ProtectedTaskGrant::new(
                reflected,
                "untrusted-secret".into(),
            ))),
        }),
    ];
    for (index, template) in templates.into_iter().enumerate() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/task-access"))
            .respond_with(template)
            .expect(1)
            .mount(&mock)
            .await;
        let output = f.directory().join(format!("pending-{index}.json"));
        let result = cli::run(
            &config,
            *mock.address(),
            "issue",
            &[("--input", &input), ("--output", &output)],
        )
        .await;
        assert!(!result.status.success());
        cli::no_secret(&result, &[&f.owner.credential, f.issued.expose_secret()]);
        assert!(cli::file(&output).grant.is_none());
        mock.verify().await;
    }
    destination.verify().await;
    f.close().await;
}
