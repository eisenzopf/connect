//! Optional exact wire samples from real listeners, separate from Store proof.
pub(super) async fn record(
    label: &str,
    request: serde_json::Value,
    response: reqwest::Response,
) -> (u16, serde_json::Value) {
    let status = response.status().as_u16();
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    let response: serde_json::Value = response.json().await.unwrap();
    if let Some(directory) = std::env::var_os("CONNECT_GATE6_INCOMING_SAMPLES") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join(format!("{label}.json")),
            serde_json::to_vec_pretty(
                &serde_json::json!({"request":request,"response":response,"status":status}),
            )
            .unwrap(),
        )
        .unwrap();
    }
    (status, response)
}
