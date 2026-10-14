//! Keep signal registration alive across asynchronous media branch work.
#[cfg(unix)]
pub fn listen() -> std::io::Result<tokio::signal::unix::Signal> {
    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
}

#[cfg(windows)]
pub fn listen() -> std::io::Result<tokio::signal::windows::CtrlC> {
    tokio::signal::windows::ctrl_c()
}

#[cfg(all(test, unix))]
mod tests {
    use super::listen;
    use std::{future::ready, time::Duration};
    use tokio::{process::Command, signal::unix::Signal, time::timeout};

    async fn deliver_interrupt(observer: &mut Signal) {
        let status = Command::new("/bin/kill")
            .args(["-INT", &std::process::id().to_string()])
            .status()
            .await
            .expect("deliver actual process SIGINT");
        assert!(status.success());
        timeout(Duration::from_secs(1), observer.recv())
            .await
            .expect("independent listener confirms OS signal delivery")
            .expect("signal listener remains open");
    }

    #[tokio::test]
    async fn interrupt_remains_pending_during_selected_media_work_and_before_first_poll() {
        let mut observer = listen().expect("register independent signal observer");

        // Negative control: the former select polled ctrl_c, then dropped its
        // receiver before awaiting the selected media branch. The OS delivery
        // is acknowledged independently; a later receiver cannot recover it.
        tokio::select! {
            biased;
            _ = tokio::signal::ctrl_c() => panic!("no interrupt sent yet"),
            _ = ready(()) => deliver_interrupt(&mut observer).await,
        }
        assert!(timeout(Duration::from_millis(50), tokio::signal::ctrl_c())
            .await
            .is_err());

        let mut interrupt = listen().expect("register production listener");
        tokio::select! {
            biased;
            _ = interrupt.recv() => panic!("no new interrupt sent yet"),
            _ = ready(()) => deliver_interrupt(&mut observer).await,
        }
        assert!(timeout(Duration::from_secs(1), interrupt.recv())
            .await
            .expect("interrupt survives selected media await")
            .is_some());

        let mut early = listen().expect("register before capture/readiness");
        deliver_interrupt(&mut observer).await;
        assert!(timeout(Duration::from_secs(1), early.recv())
            .await
            .expect("registration retains signal before first recv poll")
            .is_some());
    }
}
