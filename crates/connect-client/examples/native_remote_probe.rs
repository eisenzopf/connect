//! Isolated native profile probe. Credentials are read from a private config file.
use connect_client::{
    canonical::{CanonicalClient, CanonicalClientConfig},
    conversations::CreateRequest,
    NativeNetworkProfile,
};
use std::{net::ToSocketAddrs, path::Path};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("expected private config path")?;
    let host = args
        .next()
        .ok_or("expected designated host")?
        .into_string()
        .map_err(|_| "invalid host")?;
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    let mut config = CanonicalClientConfig::load(Path::new(&path))?;
    if config.network_profile != NativeNetworkProfile::Remote {
        return Err("explicit Remote profile required".into());
    }
    config.endpoint = (host.as_str(), config.endpoint.port())
        .to_socket_addrs()?
        .find(|address| {
            address.is_ipv4()
                && !address.ip().is_loopback()
                && config.network_profile.permits_endpoint(*address)
        })
        .ok_or("no designated non-loopback IPv4 endpoint")?;
    let mut input = CreateRequest::persistent(vec![]);
    input.tenant_id = Some("tnt_default".into());
    let mut first = CanonicalClient::open(config.clone()).await?;
    let original = first
        .create_conversation("distinct-host-create", input.clone())
        .await?;
    let id = first.keepalive().await?;
    let ack = first.receive().await?;
    if ack.kind() != "ack" || ack.original()["in_reply_to"] != id {
        return Err("uncorrelated native heartbeat".into());
    }
    first.shutdown().await?;
    let mut successor = CanonicalClient::open(config.clone()).await?;
    let replay = successor
        .create_conversation("distinct-host-create", input)
        .await?;
    if replay.id != original.id {
        return Err("reconnect replaced the original operation".into());
    }
    successor.shutdown().await?;
    println!(
        "{}",
        serde_json::json!({"profile":"remote","os":std::env::consts::OS,"architecture":std::env::consts::ARCH,"algorithm":config.signing_algorithm,"logical_operation":"distinct-host-create","reconnected_receipt_preserved":true,"verified_native_heartbeat":true})
    );
    Ok(())
}
