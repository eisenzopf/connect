//! Owned distinct-network audio fixture, using only the public canonical SDK.
//! All credentials, synthetic audio and observations stay in its private directory.
use connect_client::{
    canonical::{CanonicalClient, CanonicalClientConfig, CommandScope},
    ClientError, NativeNetworkProfile,
};
use serde_json::json;
use std::{net::ToSocketAddrs, path::PathBuf, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let dir = PathBuf::from(args.next().ok_or("expected private fixture directory")?);
    let host = args
        .next()
        .ok_or("expected designated host")?
        .into_string()
        .map_err(|_| "invalid host")?;
    let unreachable = match args.next() {
        None => false,
        Some(flag) if flag == "--expect-unreachable" => true,
        Some(_) => return Err("unexpected argument".into()),
    };
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    let mut config = CanonicalClientConfig::load(&dir.join("client.toml"))?;
    if config.network_profile != NativeNetworkProfile::Remote {
        return Err("explicit Remote profile required".into());
    }
    config.endpoint = (host.as_str(), config.endpoint.port())
        .to_socket_addrs()?
        .find(|a| {
            a.is_ipv4() && !a.ip().is_loopback() && config.network_profile.permits_endpoint(*a)
        })
        .ok_or("no designated non-loopback IPv4 endpoint")?;
    let ready: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("ready.json"))?)?;
    let cid = ready["conversation_id"]
        .as_str()
        .ok_or("missing conversation")?;
    let wave = std::fs::read(dir.join("guest-send.pcmu"))?;
    if wave.len() != 200 * 160 {
        return Err("expected four seconds of synthetic PCMU".into());
    }
    if unreachable {
        let started = std::time::Instant::now();
        match CanonicalClient::open(config.clone()).await {
            Err(ClientError::Timeout) => {}
            Err(error) => return Err(error.into()),
            Ok(client) => {
                client.shutdown().await?;
                return Err("unreachable endpoint unexpectedly authenticated".into());
            }
        }
        std::fs::write(dir.join("guest-refused"), b"timeout\n")?;
        println!(
            "{}",
            json!({"profile":"remote","os":std::env::consts::OS,"architecture":std::env::consts::ARCH,"algorithm":config.signing_algorithm,"open_refused":"timeout","no_offer_submitted":true,"elapsed_seconds":started.elapsed().as_secs_f64()})
        );
        return Ok(());
    }
    let mut peer = CanonicalClient::open(config.clone()).await?
        .prepare_public_native_audio(CommandScope {
            conversation_id: Some(cid.into()), session_id: Some("native-session".into()),
            connection_id: Some("distinct-media-guest".into()),
            operation_id: Some("distinct-media-guest-offer".into()), expected_revision: Some(0),
        }, json!({"substrate":"quic","capabilities":{"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},"streams_offered":[{"id":"distinct-media-guest-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]})).await?;
    if peer
        .subscribe_audio(
            "distinct-media-guest-select",
            1,
            vec![serde_json::from_value(
                json!({"strm_id":"distinct-media-host-voice"}),
            )?],
        )
        .await?
        != 2
    {
        return Err("unexpected stream revision".into());
    }
    std::fs::write(dir.join("guest-ready"), b"ready\n")?;
    tokio::time::timeout(Duration::from_secs(30), async {
        while !dir.join("media-go").exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    let mut received = Vec::new();
    let mut cadence = tokio::time::interval(Duration::from_millis(20));
    cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    for frame in wave.chunks_exact(160) {
        cadence.tick().await;
        peer.send_pcmu(frame)?;
        // Datagram reads are cancellation safe; no reliable frame is abandoned.
        if let Ok(result) =
            tokio::time::timeout(Duration::from_millis(10), peer.receive_pcmu()).await
        {
            received.extend(result?);
        }
    }
    std::fs::write(dir.join("guest-received.pcmu"), &received)?;
    peer.shutdown().await?;
    if received.len() < 8000 {
        return Err("less than one second received".into());
    }
    println!(
        "{}",
        json!({"profile":"remote","os":std::env::consts::OS,"architecture":std::env::consts::ARCH,"algorithm":config.signing_algorithm,"sent_frames":200,"received_bytes":received.len(),"owned_peer_shutdown":true})
    );
    Ok(())
}
