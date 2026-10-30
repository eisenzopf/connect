//! Synthetic credentials and loopback addresses supplied only by the owned coturn runner.
use serde_json::{json, Value};

pub(super) struct Fixture {
    url: String,
    port_start: u16,
    port_end: u16,
}
impl Fixture {
    pub(super) fn load() -> Self {
        let path = std::env::var("CONNECT_GATE6_TURN_FIXTURE")
            .expect("explicit owned loopback coturn fixture");
        let value: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(
            value["all_policy"], false,
            "browser lane requires relay-only policy"
        );
        let port = u16::try_from(value["listener_port"].as_u64().unwrap()).unwrap();
        let port_start = u16::try_from(value["media_port_start"].as_u64().unwrap()).unwrap();
        let port_end = u16::try_from(value["media_port_end"].as_u64().unwrap()).unwrap();
        assert!(port >= 1024 && port_start >= 1024 && port_end >= port_start);
        assert!(port_end - port_start < 32);
        let transport = value["transport"]
            .as_str()
            .expect("explicit TURN transport");
        assert!(matches!(transport, "udp" | "tcp"));
        Self {
            url: format!("turn:127.0.0.1:{port}?transport={transport}"),
            port_start,
            port_end,
        }
    }
    pub(super) fn addressing(&self) -> connect_server::canonical_webrtc::CanonicalWebRtcAddressing {
        use connect_server::canonical_webrtc::*;
        CanonicalWebRtcAddressing {
            bind_ip: "127.0.0.1".parse().unwrap(),
            port_start: self.port_start,
            port_end: self.port_end,
            public_ip: None,
            ice_policy: CanonicalIcePolicy::Relay,
            ice_services: vec![CanonicalIceService {
                urls: vec![self.url.clone()],
                username_ref: Some("CONNECT_GATE6_TURN_USER".into()),
                credential_ref: Some("CONNECT_GATE6_TURN_PASSWORD".into()),
            }],
        }
    }
    pub(super) fn configure(&self, participant: &mut Value, expired: bool) {
        participant["connection"]["iceTransportPolicy"] = json!("relay");
        participant["connection"]["iceServers"] = json!([{
            "urls":self.url,
            "username":std::env::var(if expired { "CONNECT_GATE6_TURN_EXPIRED_USER" } else { "CONNECT_GATE6_TURN_USER" }).unwrap(),
            "credential":std::env::var(if expired { "CONNECT_GATE6_TURN_EXPIRED_PASSWORD" } else { "CONNECT_GATE6_TURN_PASSWORD" }).unwrap(),
        }]);
    }
}
