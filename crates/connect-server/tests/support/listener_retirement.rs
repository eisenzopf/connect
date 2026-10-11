//! Observe actual listener retirement without exhausting local TCP addresses.
use std::{io, net::SocketAddr, time::Duration};

fn is_retired(result: io::Result<tokio::net::TcpStream>) -> io::Result<bool> {
    match result {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => Ok(true),
        // Shutdown may reset a connection that raced with listener teardown.
        // This is still pending, never evidence of a closed listener.
        Err(error) if error.kind() == io::ErrorKind::ConnectionReset => Ok(false),
        Err(error) => Err(error),
    }
}

pub async fn await_closed(endpoints: &[SocketAddr], expires_at: i64) {
    tokio::time::timeout(Duration::from_secs(8), async {
        // These listeners are expected to remain open until expiry. Repeatedly
        // opening them beforehand measures probe churn, not retirement.
        while chrono::Utc::now().timestamp() < expires_at {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        loop {
            let mut all_retired = true;
            for endpoint in endpoints {
                all_retired &= is_retired(tokio::net::TcpStream::connect(endpoint).await)
                    .unwrap_or_else(|error| {
                        panic!("Cannot observe listener {endpoint}: {error:?}")
                    });
            }
            if all_retired {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("Listeners did not retire within the original eight-second observation budget");
}

#[test]
fn only_connection_refused_proves_a_listener_closed() {
    assert!(is_retired(Err(io::ErrorKind::ConnectionRefused.into())).unwrap());
    assert!(!is_retired(Err(io::ErrorKind::ConnectionReset.into())).unwrap());
    for kind in [
        io::ErrorKind::AddrNotAvailable,
        io::ErrorKind::TimedOut,
        io::ErrorKind::PermissionDenied,
        io::ErrorKind::Interrupted,
        io::ErrorKind::Other,
    ] {
        assert_eq!(is_retired(Err(kind.into())).unwrap_err().kind(), kind);
    }
}

#[tokio::test]
async fn an_accepting_listener_is_observed_as_open() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    assert!(
        !is_retired(tokio::net::TcpStream::connect(listener.local_addr().unwrap()).await).unwrap()
    );
}
