//! Synthetic rvoip MediaStream datagrams over two real loopback QUIC endpoints.
use anyhow::{Context, Result};
use rvoip_core::{
    capability::CodecInfo,
    connection::Direction,
    ids::StreamId,
    stream::{MediaFrame, MediaStream, StreamKind},
};
use rvoip_quic::QuicDatagramMediaStream;
use rvoip_uctp::substrate;
use std::{sync::Arc, time::Duration};

#[tokio::main]
async fn main() -> Result<()> {
    tokio::time::timeout(Duration::from_secs(15), probe()).await??;
    Ok(())
}
async fn probe() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (cert, key) = substrate::self_signed_for_dev(&["localhost".into()])?;
    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.clone()], key)?;
    tls.alpn_protocols = vec![rvoip_uctp::UCTP_RAW_QUIC_ALPN_BYTES.to_vec()];
    let server = substrate::make_server_endpoint(
        "127.0.0.1:0".parse()?,
        Arc::new(tls),
        quinn::TransportConfig::default(),
    )?;
    let mut tls = substrate::dev_client_config_trusting(&cert)?;
    tls.alpn_protocols = vec![rvoip_uctp::UCTP_RAW_QUIC_ALPN_BYTES.to_vec()];
    let client = substrate::make_client_endpoint("127.0.0.1:0".parse()?, Arc::new(tls.clone()))?;
    let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls)?;
    let dialing = client.connect_with(
        quinn::ClientConfig::new(Arc::new(crypto)),
        server.local_addr()?,
        "localhost",
    )?;
    let accepting = async {
        server
            .accept()
            .await
            .context("server closed")?
            .await
            .map_err(anyhow::Error::from)
    };
    let (incoming, outgoing) = tokio::join!(accepting, dialing);
    let incoming = incoming?;
    let outgoing = outgoing?;
    let codec = CodecInfo {
        name: "PCMU".into(),
        clock_rate_hz: 8000,
        channels: 1,
        fmtp: None,
        payload_type: Some(0),
    };
    let up = QuicDatagramMediaStream::start(
        StreamId::new(),
        StreamKind::Audio,
        codec.clone(),
        Direction::Outbound,
        1,
        outgoing.clone(),
    );
    let down = QuicDatagramMediaStream::start(
        StreamId::new(),
        StreamKind::Audio,
        codec,
        Direction::Outbound,
        2,
        incoming.clone(),
    );
    let frame = |stream: &QuicDatagramMediaStream| MediaFrame {
        stream_id: stream.id(),
        kind: StreamKind::Audio,
        payload: vec![0xff; 160].into(),
        timestamp_rtp: 160,
        captured_at: chrono::Utc::now(),
        payload_type: Some(0),
    };
    up.frames_out().send(frame(&up)).await?;
    down.frames_out().send(frame(&down)).await?;
    let (uplink, downlink) = tokio::join!(incoming.read_datagram(), outgoing.read_datagram());
    let uplink = substrate::unpack_rtp_datagram(&uplink?)?;
    let downlink = substrate::unpack_rtp_datagram(&downlink?)?;
    assert_eq!(uplink.stream_local_id, 1);
    assert_eq!(downlink.stream_local_id, 2);
    for packet in [uplink, downlink] {
        assert_eq!(packet.rtp.payload_type, 0);
        assert_eq!(packet.rtp.timestamp, 160);
        assert_eq!(packet.rtp.payload.as_ref(), &[0xff; 160]);
    }
    // Reliable control remains independently usable alongside the datagram pumps.
    let (mut send, _) = outgoing.open_bi().await?;
    send.write_all(b"control-after-media").await?;
    send.finish()?;
    let (_, mut receive) = incoming.accept_bi().await?;
    assert_eq!(receive.read_to_end(64).await?, b"control-after-media");
    outgoing.close(0u32.into(), b"disconnect");
    incoming.closed().await;
    up.close().await?;
    down.close().await?;
    client.close(0u32.into(), b"done");
    server.close(0u32.into(), b"done");
    tokio::join!(client.wait_idle(), server.wait_idle());
    println!("PASS two-way synthetic PCMU rvoip MediaStream; UCTP/RTP datagrams; independent reliable stream; disconnect and pump shutdown");
    Ok(())
}
