//! Real UDP registrar with accelerated accepted leases; no provider accounts.
#![allow(clippy::unwrap_used)]
use super::*;
use rvoip_sip::Config;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::net::UdpSocket;

fn header<'a>(s: &'a str, name: &str) -> &'a str {
    s.lines()
        .find_map(|line| {
            let (k, v) = line.split_once(':')?;
            k.eq_ignore_ascii_case(name).then_some(v.trim())
        })
        .unwrap()
}
async fn carrier(socket: UdpSocket, mode: Arc<AtomicUsize>, count: Arc<AtomicUsize>) {
    let mut buf = [0; 8192];
    loop {
        let (len, peer) = socket.recv_from(&mut buf).await.unwrap();
        let s = std::str::from_utf8(&buf[..len]).unwrap();
        let unregister = header(s, "Expires") == "0";
        count.fetch_add(1, Ordering::SeqCst);
        let mode = mode.load(Ordering::SeqCst);
        if mode == 2 && !unregister {
            continue;
        }
        let status = if mode == 1 && !unregister {
            "403 Forbidden"
        } else {
            "200 OK"
        };
        let expiry = if unregister { 0 } else { 2 };
        let contact = header(s, "Contact").split(";expires=").next().unwrap();
        let reply=format!("SIP/2.0 {status}\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=fixture\r\nCall-ID: {}\r\nCSeq: {}\r\nContact: {contact};expires={expiry}\r\nExpires: {expiry}\r\nContent-Length: 0\r\n\r\n",header(s,"Via"),header(s,"From"),header(s,"To"),header(s,"Call-ID"),header(s,"CSeq"));
        socket.send_to(reply.as_bytes(), peer).await.unwrap();
    }
}
async fn start(
    registrar: &str,
    readiness: Arc<OperationalReadiness>,
) -> (Arc<UnifiedCoordinator>, Monitor, RegistrationHandle) {
    let port = std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut config = Config::local("fixture", port);
    config.contact_uri = Some(format!("sip:fixture@127.0.0.1:{port}"));
    let coordinator = UnifiedCoordinator::new(config).await.unwrap();
    let handle = coordinator
        .register(registrar, "fixture", "fixture-password")
        .with_from_uri(format!(
            "sip:fixture@{}",
            registrar.strip_prefix("sip:").unwrap()
        ))
        .with_expires(2)
        .send()
        .await
        .unwrap();
    let monitor = Monitor::start(coordinator.clone(), handle.clone(), readiness);
    (coordinator, monitor, handle)
}
async fn wait_state(r: &OperationalReadiness, predicate: impl Fn(ReadinessState) -> bool) {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if r.snapshot()
                .components
                .iter()
                .any(|c| c.component == "sip-registration" && predicate(c.state))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn accepted_lease_refreshes_after_idle_and_rejected_refresh_can_recover_on_restart() {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let registrar = format!("sip:{}", socket.local_addr().unwrap());
    let mode = Arc::new(AtomicUsize::new(0));
    let count = Arc::new(AtomicUsize::new(0));
    let task = tokio::spawn(carrier(socket, mode.clone(), count.clone()));
    let r = Arc::new(OperationalReadiness::default());
    let (coordinator, monitor, handle) = start(&registrar, r.clone()).await;
    wait_state(&r, |s| s == ReadinessState::Ready).await;
    // No calls or application traffic; accepted registrar expiry drives refresh.
    tokio::time::timeout(Duration::from_secs(6), async {
        while count.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
    wait_state(&r, |s| s == ReadinessState::Ready).await;
    let mut info = coordinator.registration_info(&handle).await.unwrap();
    assert_eq!(info.accepted_expires_secs, Some(2));
    info.status = RegistrationStatus::Registered;
    info.registered_at = Some(std::time::Instant::now() - Duration::from_secs(3));
    assert_eq!(classify(&info).0, ReadinessState::Unavailable);
    mode.store(1, Ordering::SeqCst);
    wait_state(&r, |s| s == ReadinessState::Unavailable).await;
    monitor.shutdown().await.unwrap();
    tokio::time::timeout(
        Duration::from_secs(6),
        coordinator.shutdown_gracefully(Some(Duration::from_secs(5))),
    )
    .await
    .unwrap()
    .unwrap();
    mode.store(0, Ordering::SeqCst);
    let (coordinator, monitor, _) = start(&registrar, r.clone()).await;
    wait_state(&r, |s| s == ReadinessState::Ready).await;
    monitor.shutdown().await.unwrap();
    tokio::time::timeout(
        Duration::from_secs(6),
        coordinator.shutdown_gracefully(Some(Duration::from_secs(5))),
    )
    .await
    .unwrap()
    .unwrap();
    task.abort();
}
#[tokio::test]
async fn dropped_refresh_does_not_leave_an_expired_binding_ready() {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let registrar = format!("sip:{}", socket.local_addr().unwrap());
    let mode = Arc::new(AtomicUsize::new(0));
    let task = tokio::spawn(carrier(socket, mode.clone(), Arc::new(AtomicUsize::new(0))));
    let r = Arc::new(OperationalReadiness::default());
    let (coordinator, monitor, _) = start(&registrar, r.clone()).await;
    wait_state(&r, |s| s == ReadinessState::Ready).await;
    mode.store(2, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(3)).await;
    wait_state(&r, |s| {
        matches!(s, ReadinessState::Uncertain | ReadinessState::Unavailable)
    })
    .await;
    // Restore the registrar. The existing REGISTER retry can recover; this is
    // transport registration, never a replay of a phone call or provider job.
    mode.store(0, Ordering::SeqCst);
    wait_state(&r, |s| s == ReadinessState::Ready).await;
    monitor.shutdown().await.unwrap();
    tokio::time::timeout(
        Duration::from_secs(6),
        coordinator.shutdown_gracefully(Some(Duration::from_secs(5))),
    )
    .await
    .unwrap()
    .unwrap();
    task.abort();
}
