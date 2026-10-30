//! Fixed local port-preserving NAT and owned impairment proxy fixture.
use serde_json::{json, Value};
use std::path::PathBuf;

pub(super) struct Fixture {
    start: u16,
    end: u16,
    bursts: Value,
    window_ready: PathBuf,
}
impl Fixture {
    pub(super) fn load() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let path = PathBuf::from(
            std::env::var_os("CONNECT_GATE6_BROWSER_NETWORK_FIXTURE")
                .expect("owned network fixture manifest"),
        )
        .canonicalize()
        .unwrap();
        assert!(path.starts_with(root.join(".local")));
        let value: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let start = u16::try_from(value["port_start"].as_u64().unwrap()).unwrap();
        let end = u16::try_from(value["port_end"].as_u64().unwrap()).unwrap();
        assert!(start >= 1024 && end >= start && end - start == 7);
        let window_ready = PathBuf::from(value["window_ready"].as_str().unwrap());
        let output =
            PathBuf::from(std::env::var_os("CONNECT_GATE6_CANONICAL_BROWSER_ARTIFACTS").unwrap())
                .canonicalize()
                .unwrap();
        assert_eq!(window_ready, output.join("window-ready.json"));
        assert!(!window_ready.exists());
        let bursts = value["bursts"].clone();
        let burst = bursts.as_array().filter(|v| v.len() == 1).unwrap()[0]
            .as_object()
            .unwrap();
        assert_eq!(burst.len(), 3);
        let begin = burst["start_unix_seconds"].as_f64().unwrap();
        let finish = burst["end_unix_seconds"].as_f64().unwrap();
        assert!(begin.is_finite() && finish.is_finite() && finish - begin == 3.0);
        assert_eq!(burst["duration_seconds"], 3.0);
        Self {
            start,
            end,
            bursts,
            window_ready,
        }
    }
    pub(super) fn addressing(&self) -> connect_server::canonical_webrtc::CanonicalWebRtcAddressing {
        use connect_server::canonical_webrtc::*;
        CanonicalWebRtcAddressing {
            bind_ip: "127.0.0.1".parse().unwrap(),
            port_start: self.start,
            port_end: self.end,
            public_ip: Some("127.0.0.2".parse().unwrap()),
            ice_policy: CanonicalIcePolicy::All,
            ice_services: vec![],
        }
    }
    pub(super) fn configure(&self, spec: &mut Value) {
        spec["profileMode"] = json!("impairment");
        spec["windowReadyPath"] = json!(self.window_ready);
        spec["networkBursts"] = self.bursts.clone();
        spec["ownedMediaProxy"] =
            json!({"address":"127.0.0.2","portStart":self.start,"portEnd":self.end});
    }
}
