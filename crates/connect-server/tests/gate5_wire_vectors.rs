#![allow(clippy::unwrap_used)] // Published conformance vectors fail loudly.
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn published_wire_vectors_match_the_native_length_prefixed_parser() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/fixtures/wire/framing-v1.json");
    let vectors: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(vectors["profile"], connect_protocol::PROFILE);
    assert_eq!(vectors["alpn"], rvoip_uctp::UCTP_RAW_QUIC_ALPN);
    for vector in vectors["vectors"].as_array().unwrap() {
        let frame = hex::decode(vector["frame_hex"].as_str().unwrap()).unwrap();
        let (mut writer, reader) = tokio::io::duplex(16);
        let writing = tokio::spawn(async move {
            // Split prefix and JSON across reads to rule out packet-sized framing.
            for byte in frame {
                writer.write_all(&[byte]).await.unwrap();
            }
            writer.shutdown().await.unwrap();
        });
        let mut envelopes = Box::pin(rvoip_uctp::substrate::envelope_reader(reader));
        let decoded = tokio::time::timeout(std::time::Duration::from_secs(2), envelopes.next())
            .await
            .unwrap();
        if vector["expected"] == "valid-envelope" {
            let envelope = decoded.unwrap().unwrap();
            assert_eq!(serde_json::to_value(envelope).unwrap(), vector["json"]);
            assert!(envelopes.next().await.is_none());
        } else {
            assert!(decoded.unwrap().is_err(), "{}", vector["name"]);
        }
        writing.await.unwrap();
    }
}
