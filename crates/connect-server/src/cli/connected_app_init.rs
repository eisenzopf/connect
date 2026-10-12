//! Optional local preset: an app controller, accepted peer and policy-denied peer.
use super::init::private_write;
use anyhow::Result;
use connect_client::ClientConfig;
use connect_store::{Scope, Store};
use std::{net::SocketAddr, path::Path};

pub(super) fn write(store: &Store, directory: &Path, expiry: i64, bind: SocketAddr) -> Result<()> {
    let app = store.create_identity("reviews")?;
    let peer = store.create_identity("agent")?;
    let outside = store.create_identity("outside")?;
    let scope = super::credentials::with_task_management(Scope::messaging(vec![
        peer.id.clone(),
        outside.id.clone(),
    ]));
    for (name, identity, scope) in [
        ("reviews", &app, scope),
        ("agent", &peer, Scope::messaging(vec![app.id.clone()])),
        ("outside", &outside, Scope::messaging(vec![app.id.clone()])),
    ] {
        let key = store.issue(&identity.id, scope, expiry)?;
        let config = ClientConfig {
            endpoint: bind,
            server_name: "localhost".into(),
            trust_certificate: "certificate.der".into(),
            credential: key.token,
            identity_id: identity.id.clone(),
            credential_id: key.id,
            timeout_ms: 10000,
        };
        private_write(
            &directory.join(format!("{name}.toml")),
            toml::to_string_pretty(&config)?.as_bytes(),
        )?;
    }
    // The outside peer can reach the app at transport level, but cannot create
    // a business effect under this app's separately configured contact policy.
    let policy = serde_json::json!({"reviewer_address":"connect:agent@local","reviewer_id":peer.id,"allowed_contacts":[peer.id],"draft":{"id":"draft-002","title":"Connect release announcement","body":"Connect gives applications and agents a durable communication surface. Review this announcement before publication."}});
    private_write(
        &directory.join("connected-app.json"),
        serde_json::to_vec_pretty(&policy)?.as_slice(),
    )?;
    Ok(())
}
