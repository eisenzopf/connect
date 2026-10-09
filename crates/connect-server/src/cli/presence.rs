//! Local operator setup for a two-way assistant presence, without external calls.
use super::init;
use connect_client::ClientConfig;
use connect_protocol::{Operation, Outcome};
use connect_store::{Route, Scope, Store};
use std::{net::SocketAddr, path::Path};
pub fn initialize(
    directory: &Path,
    bind: SocketAddr,
    assistant_id: &str,
    human_number: Option<&str>,
    connect_number: Option<&str>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !directory.exists(),
        "Presence initialization requires a new private directory"
    );
    anyhow::ensure!(
        uuid::Uuid::parse_str(assistant_id).is_ok(),
        "Invalid assistant ID"
    );
    anyhow::ensure!(
        human_number.is_some() == connect_number.is_some(),
        "Set both human and Connect SMS numbers, or neither"
    );
    init::initialize(directory, false, bind)?;
    let server = crate::config::ServerConfig::load(&directory.join("server.toml"))?;
    let store = Store::open(&server.database, server.limits)?;
    let operator = ClientConfig::load(&directory.join("operator.toml"))?;
    let authority = store.authenticate(&operator.credential)?;
    let human = store.create_identity("human")?;
    let assistant = store.create_identity("assistant")?;
    let expiry = chrono::Utc::now().timestamp() + 30 * 86400;
    let human_key = store.issue_as(
        &authority,
        &human.id,
        Scope::communications(vec![assistant.id.clone()]),
        expiry,
    )?;
    let assistant_key = store.issue_as(
        &authority,
        &assistant.id,
        Scope::communications(vec![human.id.clone()]),
        expiry,
    )?;
    let human_config = ClientConfig {
        endpoint: bind,
        server_name: "localhost".into(),
        trust_certificate: server.certificate.canonicalize()?,
        credential: human_key.token,
        identity_id: human.id,
        credential_id: human_key.id,
        timeout_ms: 10_000,
    };
    let assistant_config = ClientConfig {
        endpoint: bind,
        server_name: "localhost".into(),
        trust_certificate: server.certificate.canonicalize()?,
        credential: assistant_key.token,
        identity_id: assistant.id,
        credential_id: assistant_key.id,
        timeout_ms: 10_000,
    };
    let conversation = match store.execute(
        &human_config.credential_id,
        &Operation::Create {
            request_id: "presence-conversation".into(),
            participants: vec![assistant_config.identity_id.clone()],
        },
    )? {
        Outcome::Conversation(c) => c,
        _ => anyhow::bail!("Unexpected conversation result"),
    };
    store.configure_route(
        &authority.credential_id,
        &Route {
            id: "assistant-chat".into(),
            identity_id: assistant_config.identity_id.clone(),
            credential_id: assistant_config.credential_id.clone(),
            conversation_id: conversation.id.clone(),
            connector: "vapi".into(),
            target: assistant_id.into(),
            source: None,
            enabled: true,
        },
    )?;
    if let (Some(human), Some(source)) = (human_number, connect_number) {
        store.configure_route(
            &authority.credential_id,
            &Route {
                id: "human-sms".into(),
                identity_id: human_config.identity_id.clone(),
                credential_id: human_config.credential_id.clone(),
                conversation_id: conversation.id.clone(),
                connector: "telnyx".into(),
                target: human.into(),
                source: Some(source.into()),
                enabled: true,
            },
        )?;
    }
    init::private_write(
        &directory.join("human.toml"),
        toml::to_string_pretty(&human_config)?.as_bytes(),
    )?;
    init::private_write(
        &directory.join("assistant.toml"),
        toml::to_string_pretty(&assistant_config)?.as_bytes(),
    )?;
    init::private_write(&directory.join("presence.json"),serde_json::to_vec_pretty(&serde_json::json!({"conversation_id":conversation.id,"human_identity":human_config.identity_id,"assistant_identity":assistant_config.identity_id,"assistant_id":assistant_id,"sms_configured":human_number.is_some()}))?.as_slice())?;
    println!(
        "Presence initialized; config and manifest in {}",
        directory.display()
    );
    Ok(())
}
