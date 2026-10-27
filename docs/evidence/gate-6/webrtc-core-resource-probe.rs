//! G6-051 typed Core WebRTC resource preparation against ordinary production
//! libraries. Policy/membership fixtures are isolated; Core reservation,
//! canonical adapter, RTC allocation/driver closure and socket rebinding are real.
//! No Store media readiness, browser or remote hosting qualification is claimed.
#[allow(dead_code)]
#[path = "../../../.deps/rvoip/crates/foundation/rvoip-core/tests/support/participant_control_fixture.rs"]
mod core_fixture;
use chrono::Utc;
use rvoip_core::{
    adapter::{ConnectionAdapter, EndReason},
    connection::Transport,
    identity::{AuthenticatedPrincipal, CredentialKind, IdentityAssurance},
    ids::*,
    orchestrator::peer_native::*,
    peer_admission::{PeerAdmission, PeerAdmissionLimits, PeerAdmissionOwner},
    relationships::{EndpointBinding, IdentityBinding},
    session::SessionState,
};
use rvoip_webrtc::{
    WebRtcConfig,
    canonical::{CanonicalWebRtcAdapter, WebRtcPhysicalBinding},
    peer::PeerRole,
};
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
fn actor(name: &str) -> AuthenticatedPrincipal {
    let mut p = core_fixture::principal(name);
    p.expires_at = Some(Utc::now() + chrono::Duration::hours(1));
    p.assurance = IdentityAssurance::Identified {
        credential_kind: CredentialKind::Oidc,
    };
    p
}
fn owner(limits: PeerAdmissionLimits) -> PeerAdmissionOwner {
    PeerAdmissionOwner::new(
        &actor("alice"),
        IdentityId::from_string("id_alice"),
        DeviceId::from_string("device_alice"),
        CancellationToken::new(),
        limits,
    )
    .unwrap()
}
fn endpoint(name: &str, kind: Transport) -> EndpointBinding {
    let b = IdentityBinding::from_authenticated(
        &actor(name),
        IdentityId::from_string(format!("id_{name}")),
        Utc::now(),
    )
    .unwrap();
    let mut ep = EndpointBinding::provision(
        b.clone(),
        EndpointId::from_string(format!("endpoint_{name}_{kind:?}")),
        DeviceId::from_string(format!("device_{name}")),
        kind,
        "controlled-loopback".into(),
    )
    .unwrap();
    ep.verify(&b, "controlled-proof".into(), Utc::now())
        .unwrap();
    ep
}
struct Lease<'a> {
    _guard: std::sync::RwLockReadGuard<'a, core_fixture::PolicyState>,
    authority: PeerNativeAuthority,
}
impl PeerNativeAuthorityLease for Lease<'_> {
    fn authority(&self) -> &PeerNativeAuthority {
        &self.authority
    }
}
fn lease(f: &core_fixture::Fixture) -> Lease<'_> {
    let guard = f.policy.0.read().unwrap();
    let authority = PeerNativeAuthority {
        actor: guard.members[&core_fixture::participant("alice")].clone(),
        endpoint: endpoint("alice", Transport::WebRtc),
        may_prepare: !guard.denied,
    };
    Lease {
        _guard: guard,
        authority,
    }
}
fn intent(f: &core_fixture::Fixture) -> PeerWebRtcIntent {
    PeerWebRtcIntent {
        session: f.session.clone(),
        connection: ConnectionId::new(),
    }
}
fn prepare(
    f: &core_fixture::Fixture,
    p: &Arc<PeerAdmission>,
    i: PeerWebRtcIntent,
    b: &Arc<WebRtcPhysicalBinding>,
) -> Result<PreparedPeerWebRtcConnection, PeerNativeError> {
    f.orch
        .prepare_peer_webrtc_connection(p.clone(), &actor("alice"), i, &lease(f), b.clone())
}
fn binding(p: &Arc<PeerAdmission>) -> Arc<WebRtcPhysicalBinding> {
    WebRtcPhysicalBinding::new(p, CancellationToken::new())
}
async fn setup() -> (core_fixture::Fixture, Arc<CanonicalWebRtcAdapter>) {
    let f = core_fixture::Fixture::new_without_adapter().await;
    let a = CanonicalWebRtcAdapter::new();
    f.orch.register(a.clone()).unwrap();
    (f, a)
}
fn config() -> (WebRtcConfig, std::net::SocketAddr) {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = socket.local_addr().unwrap();
    drop(socket);
    let mut c = WebRtcConfig::loopback();
    c.udp_bind = addr.to_string();
    (c, addr)
}
async fn retired(a: &CanonicalWebRtcAdapter, p: &PeerAdmission) {
    core_fixture::until(|| a.route_count() == 0 && p.snapshot().connections == 0).await;
}
fn passed(s: &str) {
    println!("PASS {s}");
}
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let (f, a) = setup().await;
    let o = owner(Default::default());
    let p = o.peer();
    let b = binding(&p);
    let (c, addr) = config();
    assert!(b.start_peer(c.clone(), PeerRole::Answerer).is_err());
    assert!(std::net::UdpSocket::bind(addr).is_ok());
    assert_eq!(a.total_allocations(), 0);
    passed("RTC allocation requires an original Core reservation");
    assert!(!a.supports_peer_native_preparation());
    let native = PeerNativeIntent {
        session: f.session.clone(),
        connection: ConnectionId::new(),
        transport: Transport::WebRtc,
    };
    assert!(
        f.orch
            .prepare_peer_native_connection(
                p.clone(),
                &actor("alice"),
                native,
                &lease(&f),
                b.clone()
            )
            .is_err()
    );
    assert_eq!(a.total_allocations(), 0);
    assert_eq!(p.snapshot().connections, 0);
    passed("native entrypoint cannot allocate WebRTC");
    for case in [
        "subject",
        "issuer",
        "expiry",
        "endpoint",
        "grant",
        "transport",
        "context",
    ] {
        let mut principal = actor("alice");
        let mut l = lease(&f);
        match case {
            "subject" => principal.subject = "bob".into(),
            "issuer" => principal.issuer = Some("other".into()),
            "expiry" => principal.expires_at = Some(Utc::now() - chrono::Duration::seconds(1)),
            "endpoint" => l.authority.endpoint = endpoint("bob", Transport::WebRtc),
            "grant" => l.authority.may_prepare = false,
            "transport" => l.authority.endpoint = endpoint("alice", Transport::WebTransport),
            _ => {}
        }
        let context: Arc<dyn std::any::Any + Send + Sync> = if case == "context" {
            Arc::new(())
        } else {
            b.clone()
        };
        assert!(
            f.orch
                .prepare_peer_webrtc_connection(p.clone(), &principal, intent(&f), &l, context)
                .is_err()
        );
    }
    retired(&a, &p).await;
    assert_eq!(a.total_allocations(), 0);
    passed(
        "identity expiry Endpoint transport current grant and context denials precede RTC allocation",
    );
    let foreign = owner(Default::default());
    assert!(prepare(&f, &foreign.peer(), intent(&f), &b).is_err());
    retired(&a, &foreign.peer()).await;
    assert_eq!(a.total_allocations(), 0);
    passed("a context rejects a foreign authenticated physical peer before first claim");
    let i = intent(&f);
    let candidate = prepare(&f, &p, i.clone(), &b).unwrap();
    assert_eq!(candidate.connection_id(), &i.connection);
    assert_eq!(p.snapshot().connections, 1);
    assert_eq!(p.snapshot().sessions, 1);
    let id = candidate.retain(&actor("alice"), &lease(&f)).unwrap();
    assert_eq!(a.route_count(), 1);
    assert_eq!(f.orch.connection_transport(&id).unwrap(), Transport::WebRtc);
    assert_eq!(f.orch.participant_capacity(&f.session).unwrap(), (0, 0));
    assert_eq!(
        f.orch.session(&f.session).unwrap().read().unwrap().state,
        SessionState::Initiating
    );
    assert!(a.streams(id.clone()).await.unwrap().is_empty());
    assert!(a.capabilities().audio_codecs.is_empty());
    passed(
        "typed preparation reserves a dormant Core route without Participant or media readiness",
    );
    assert!(prepare(&f, &p, intent(&f), &b).is_err());
    core_fixture::until(|| p.snapshot().connections == 1).await;
    assert_eq!(a.total_allocations(), 1);
    passed("a physical binding cannot reserve a second Core Connection");
    b.start_peer(c.clone(), PeerRole::Answerer).unwrap();
    assert!(b.start_peer(c, PeerRole::Answerer).is_err());
    let rtc = b
        .peer(tokio::time::Instant::now() + Duration::from_secs(3))
        .await
        .unwrap();
    assert!(std::net::UdpSocket::bind(addr).is_err());
    let same = b
        .peer(tokio::time::Instant::now() + Duration::from_secs(1))
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&rtc, &same));
    drop(same);
    passed("exactly one original RTC allocation owns the configured UDP socket");
    let mut observer = Box::pin(b.shutdown());
    std::future::poll_fn(|cx| {
        assert!(observer.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(observer);
    b.shutdown().await.unwrap();
    assert!(
        b.peer(tokio::time::Instant::now() + Duration::from_secs(1))
            .await
            .is_err()
    );
    assert!(std::net::UdpSocket::bind(addr).is_ok());
    assert_eq!(a.route_count(), 1);
    assert_eq!(p.snapshot().connections, 1);
    passed(
        "canceled close observation retains original close and a live Arc does not retain the RTC socket",
    );
    a.end(id, EndReason::Normal).await.unwrap();
    retired(&a, &p).await;
    assert!(prepare(&f, &p, i, &binding(&p)).is_err());
    assert_eq!(a.total_allocations(), 1);
    passed("actual cleanup precedes Core quota refund and connection IDs never reuse");
    drop(rtc);
    drop(o);

    let (f, a) = setup().await;
    let o = owner(Default::default());
    let p = o.peer();
    for _ in 0..64 {
        let b = binding(&p);
        prepare(&f, &p, intent(&f), &b)
            .unwrap()
            .retain(&actor("alice"), &lease(&f))
            .unwrap();
    }
    assert_eq!(a.total_allocations(), 64);
    assert_eq!(p.snapshot().connections, 64);
    assert!(matches!(
        prepare(&f, &p, intent(&f), &binding(&p)),
        Err(PeerNativeError::Capacity)
    ));
    assert_eq!(a.total_allocations(), 64);
    drop(o);
    retired(&a, &p).await;
    passed("default 64-Connection quota precedes adapter callback and EOF retires all routes");

    let (f, a) = setup().await;
    let o = owner(Default::default());
    let p = o.peer();
    for _ in 0..32 {
        let mut i = intent(&f);
        i.session = f.second_session().await;
        prepare(&f, &p, i, &binding(&p))
            .unwrap()
            .retain(&actor("alice"), &lease(&f))
            .unwrap();
    }
    let mut i = intent(&f);
    i.session = f.second_session().await;
    assert!(matches!(
        prepare(&f, &p, i, &binding(&p)),
        Err(PeerNativeError::Capacity)
    ));
    assert_eq!(a.total_allocations(), 32);
    drop(o);
    retired(&a, &p).await;
    passed("default 32-Session union quota precedes adapter allocation");

    let (f, a) = setup().await;
    let o = owner(Default::default());
    let p = o.peer();
    let b = binding(&p);
    let i = intent(&f);
    let candidate = prepare(&f, &p, i.clone(), &b).unwrap();
    let (c, addr) = config();
    b.start_peer(c, PeerRole::Answerer).unwrap();
    let rtc = b
        .peer(tokio::time::Instant::now() + Duration::from_secs(3))
        .await
        .unwrap();
    assert!(std::net::UdpSocket::bind(addr).is_err());
    drop(candidate);
    retired(&a, &p).await;
    assert!(std::net::UdpSocket::bind(addr).is_ok());
    assert!(
        b.peer(tokio::time::Instant::now() + Duration::from_secs(1))
            .await
            .is_err()
    );
    assert!(prepare(&f, &p, i, &binding(&p)).is_err());
    drop(rtc);
    passed(
        "an unretained candidate closes its actual RTC driver before refund despite retained RTC Arc",
    );

    let b = binding(&p);
    let candidate = prepare(&f, &p, intent(&f), &b).unwrap();
    let mut denied = lease(&f);
    denied.authority.may_prepare = false;
    assert!(candidate.retain(&actor("alice"), &denied).is_err());
    drop(denied);
    retired(&a, &p).await;
    assert!(
        b.start_peer(WebRtcConfig::loopback(), PeerRole::Answerer)
            .is_err()
    );
    passed("current retention denial cancels the original Core reservation");

    let b = binding(&p);
    let candidate = prepare(&f, &p, intent(&f), &b).unwrap();
    let (mut c, addr) = config();
    c.udp_bind = "invalid address".into();
    b.start_peer(c, PeerRole::Answerer).unwrap();
    assert!(
        b.peer(tokio::time::Instant::now() + Duration::from_secs(3))
            .await
            .is_err()
    );
    drop(candidate);
    retired(&a, &p).await;
    assert!(std::net::UdpSocket::bind(addr).is_ok());
    passed("definite RTC construction failure safely retires without a socket leak");

    let b = binding(&p);
    let candidate = prepare(&f, &p, intent(&f), &b).unwrap();
    let (c, addr) = config();
    b.start_peer(c, PeerRole::Answerer).unwrap();
    let mut observer = Box::pin(b.peer(tokio::time::Instant::now() + Duration::from_secs(3)));
    std::future::poll_fn(|cx| {
        assert!(observer.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(observer);
    let rtc = b
        .peer(tokio::time::Instant::now() + Duration::from_secs(3))
        .await
        .unwrap();
    assert!(std::net::UdpSocket::bind(addr).is_err());
    assert!(
        b.start_peer(WebRtcConfig::loopback(), PeerRole::Answerer)
            .is_err()
    );
    drop(o);
    retired(&a, &p).await;
    assert!(std::net::UdpSocket::bind(addr).is_ok());
    assert!(candidate.retain(&actor("alice"), &lease(&f)).is_err());
    drop(rtc);
    passed("allocation observations cannot restart work and physical EOF joins actual cleanup");
    let (f, a) = setup().await;
    let o = owner(Default::default());
    let p = o.peer();
    let b = binding(&p);
    let id = prepare(&f, &p, intent(&f), &b)
        .unwrap()
        .retain(&actor("alice"), &lease(&f))
        .unwrap();
    let (mut c, addr) = config();
    // This invalid trusted fixture capacity panics before RTC construction.
    // The real original JoinError must remain unconfirmed; no injected adapter result.
    c.handler_channel_capacity = usize::MAX;
    b.start_peer(c, PeerRole::Answerer).unwrap();
    assert!(
        b.peer(tokio::time::Instant::now() + Duration::from_secs(3))
            .await
            .is_err()
    );
    assert!(b.shutdown().await.is_err());
    assert!(b.shutdown().await.is_err());
    assert!(std::net::UdpSocket::bind(addr).is_ok());

    assert!(a.end(id, EndReason::Normal).await.is_err());
    assert_eq!(a.route_count(), 1);
    assert_eq!(p.snapshot().connections, 1);
    assert!(
        b.start_peer(WebRtcConfig::loopback(), PeerRole::Answerer)
            .is_err()
    );
    passed("a missing original allocation join stays unconfirmed and quarantines Core quota");
}
