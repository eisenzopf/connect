use super::*;

fn relay() -> Addressing {
    Addressing {
        bind_ip: "0.0.0.0".parse().unwrap(),
        port_start: 41000,
        port_end: 41001,
        public_ip: Some("203.0.113.44".parse().unwrap()),
        ice_policy: IcePolicy::Relay,
        ice_services: vec![IceService {
            urls: vec!["turns:relay.fixture.invalid:443?transport=tcp".into()],
            username_ref: Some("CONNECT_FIXTURE_TURN_USER".into()),
            credential_ref: Some("CONNECT_FIXTURE_TURN_PASSWORD".into()),
        }],
    }
}

#[test]
fn explicit_remote_mapping_uses_the_bounded_driver_and_keeps_only_references_serialized() {
    let addressing = relay();
    let mut backend = WebRtcConfig::default();
    let mut reads = Vec::new();
    addressing
        .apply(NativeNetworkProfile::Remote, 2, &mut backend, |name| {
            reads.push(name.to_owned());
            Ok(if name.ends_with("_USER") {
                "1234567890:synthetic-user".into()
            } else {
                "  synthetic-turn-password-canary  ".into()
            })
        })
        .unwrap();
    assert_eq!(reads.len(), 2);
    assert_eq!(backend.udp_bind, "0.0.0.0:0");
    let range = backend.udp_port_range.as_ref().unwrap();
    assert_eq!((range.port_start, range.port_end), (41000, 41001));
    assert_eq!(range.bind_ip, addressing.bind_ip);
    assert_eq!(backend.nat_1to1_ips, ["203.0.113.44"]);
    assert_eq!(backend.ice_transport_policy, IceTransportPolicy::Relay);
    assert_eq!(backend.ice_servers.len(), 1);
    assert_eq!(backend.ice_servers[0].urls, addressing.ice_services[0].urls);
    assert_eq!(
        backend.ice_servers[0].username.as_deref(),
        Some("1234567890:synthetic-user")
    );
    assert_eq!(
        backend.ice_servers[0].credential.as_deref(),
        Some("  synthetic-turn-password-canary  ")
    );
    for rendered in [
        format!("{backend:?}"),
        format!("{addressing:?}"),
        toml::to_string(&addressing).unwrap(),
    ] {
        assert!(!rendered.contains("synthetic-turn-password-canary"));
        assert!(!rendered.contains("1234567890:synthetic-user"));
        assert!(!rendered.contains("stun.l.google.com"));
    }
}

#[test]
fn invalid_network_capacity_urls_and_reference_pairs_refuse_before_secret_io() {
    let mut cases = Vec::new();
    let base = relay();
    let mut value = base.clone();
    value.port_end = value.port_start;
    cases.push(value);
    let mut value = base.clone();
    value.port_start = 1000;
    cases.push(value);
    let mut value = base.clone();
    value.port_end = 50000;
    cases.push(value);
    let mut value = base.clone();
    value.public_ip = Some("::1".parse().unwrap());
    cases.push(value);
    let mut value = base.clone();
    value.public_ip = Some("0.0.0.0".parse().unwrap());
    cases.push(value);
    let mut value = base.clone();
    value.ice_services.clear();
    cases.push(value);
    let mut value = base.clone();
    value.ice_services[0].username_ref = None;
    cases.push(value);
    let mut value = base.clone();
    value.ice_services[0].credential_ref = Some("raw password with spaces".into());
    cases.push(value);
    let mut value = base.clone();
    let duplicate = value.ice_services[0].urls[0].clone();
    value.ice_services[0].urls.push(duplicate);
    cases.push(value);
    for url in [
        "https://relay.fixture.invalid",
        "turn://relay.fixture.invalid",
        "turn:user:secret@relay.fixture.invalid",
        "turn:relay.fixture.invalid/path",
        "turn:relay.fixture.invalid?transport=udp&other=secret",
        "turn:relay.fixture.invalid:0",
        "turn:relay.fixture.invalid:65536",
        "turn:224.0.0.1:3478",
        "turn:255.255.255.255:3478",
        "turn:[::]:3478",
        "turn:relay%2efixture.invalid:3478",
        "stun:localhost:3478?transport=tcp",
    ] {
        let mut value = base.clone();
        value.ice_services[0].urls = vec![url.into()];
        cases.push(value);
    }
    for addressing in cases {
        let mut backend = WebRtcConfig::loopback();
        assert!(addressing
            .apply(NativeNetworkProfile::Remote, 2, &mut backend, |_| panic!(
                "Invalid config must not read secrets"
            ))
            .is_err());
        assert!(backend.udp_port_range.is_none());
        assert!(backend.ice_servers.is_empty());
    }
    assert!(base
        .apply(
            NativeNetworkProfile::Local,
            2,
            &mut WebRtcConfig::loopback(),
            |_| panic!("Local config must refuse remote addressing")
        )
        .is_err());
}

#[test]
fn failed_secret_lookup_is_redacted_and_does_not_publish_partial_configuration() {
    let mut backend = WebRtcConfig::loopback();
    let mut reads = 0;
    let error = relay()
        .apply(NativeNetworkProfile::Remote, 2, &mut backend, |_| {
            reads += 1;
            if reads == 1 {
                Ok("private-username-canary".into())
            } else {
                anyhow::bail!("private-password-canary")
            }
        })
        .unwrap_err();
    assert_eq!(reads, 2);
    assert!(!format!("{error:#}").contains("canary"));
    assert!(backend.ice_servers.is_empty());
    assert!(backend.udp_port_range.is_none());
    assert!(backend.nat_1to1_ips.is_empty());
}

#[test]
fn local_addressing_stays_loopback_and_never_reads_a_stun_credential() {
    let mut addressing = relay();
    addressing.bind_ip = "127.0.0.1".parse().unwrap();
    addressing.public_ip = None;
    addressing.ice_policy = IcePolicy::All;
    addressing.ice_services = vec![IceService {
        urls: vec!["stun:127.0.0.1:3478".into()],
        username_ref: None,
        credential_ref: None,
    }];
    let mut backend = WebRtcConfig::loopback();
    addressing
        .apply(NativeNetworkProfile::Local, 2, &mut backend, |_| {
            panic!("STUN must not read secrets")
        })
        .unwrap();
    assert_eq!(backend.udp_bind, "127.0.0.1:0");
    assert_eq!(backend.ice_servers[0].urls, ["stun:127.0.0.1:3478"]);
    assert!(backend.ice_servers[0].credential.is_none());
    addressing.ice_services[0].urls = vec!["stun:public.fixture.invalid:3478".into()];
    assert!(addressing.validate(NativeNetworkProfile::Local, 2).is_err());
}
